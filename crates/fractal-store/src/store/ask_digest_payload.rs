//! Lossless digest storage with a constant-sized source and bounded immutable documents.
//! Documents contain UTF-8 JSONL fragments, linked backwards by exact native document refs.
use super::*;

const FRAGMENT_BYTES: usize = 16 * 1024;

/// Consumes rows incrementally, retaining at most one row and one bounded fragment.
/// The returned object is merged into the notice's `source` metadata.
#[cfg(test)]
pub(super) fn write(
    tx: &Transaction<'_>,
    origin: &str,
    rows: impl IntoIterator<Item = Result<Value>>,
) -> Result<Value> {
    append(tx, origin, None, rows)
}

/// Adds a bounded producer page to an unpublished draft without rewriting earlier data.
pub(super) fn append(
    tx: &Transaction<'_>,
    origin: &str,
    previous: Option<&Value>,
    rows: impl IntoIterator<Item = Result<Value>>,
) -> Result<Value> {
    let mut data = String::with_capacity(FRAGMENT_BYTES);
    let mut previous_ref = previous.and_then(|value| value["digest_document"].as_str()).map(str::to_owned);
    let mut count = if previous_ref.is_some() {
        previous.context("missing digest metadata")?["digest_count"].as_str().context("missing digest count")?.parse()?
    } else { 0_u64 };
    let inline = previous.and_then(|value| value["digest"].as_array()).into_iter().flatten().cloned().map(Ok);
    let mut fragment_rows = 0;
    for row in inline.chain(rows) {
        if fragment_rows == 128 {
            previous_ref = Some(bind_fragment(tx, origin, &data, previous_ref.as_deref())?);
            data.clear();
            fragment_rows = 0;
        }
        let mut encoded = serde_json::to_string(&row?)?;
        encoded.push('\n');
        let mut remaining = encoded.as_str();
        while !remaining.is_empty() {
            if data.len() == FRAGMENT_BYTES {
                previous_ref = Some(bind_fragment(tx, origin, &data, previous_ref.as_deref())?);
                data.clear();
                fragment_rows = 0;
            }
            let mut take = remaining.len().min(FRAGMENT_BYTES - data.len());
            while !remaining.is_char_boundary(take) { take -= 1; }
            if take == 0 {
                previous_ref = Some(bind_fragment(tx, origin, &data, previous_ref.as_deref())?);
                data.clear();
                fragment_rows = 0;
                continue;
            }
            data.push_str(&remaining[..take]);
            remaining = &remaining[take..];
        }
        count = count.checked_add(1).context("digest row count overflow")?;
        fragment_rows += 1;
    }
    if previous_ref.is_some() {
        let root = bind_fragment(tx, origin, &data, previous_ref.as_deref())?;
        Ok(json!({"digest_document":root,"digest_count":count.to_string()}))
    } else {
        let rows = data.lines().map(serde_json::from_str::<Value>).collect::<serde_json::Result<Vec<_>>>()?;
        Ok(json!({"digest":rows,"digest_count":count.to_string()}))
    }
}

fn bind_fragment(tx: &Transaction<'_>, origin: &str, data: &str, previous: Option<&str>) -> Result<String> {
    let mut document = json!({"data":data});
    if let Some(previous) = previous { document["previous"] = json!(previous); }
    let bytes = serde_json::to_vec(&document)?;
    // JSON escaping can double an already-serialized JSONL fragment; keep ample room
    // beneath the notice/claim 64KiB limit, even for one very long valid title.
    anyhow::ensure!(bytes.len() < 64 * 1024, "digest fragment exceeds its byte budget");
    let hash = hex::encode(Sha256::digest(&bytes));
    let name = format!("doc/ask-digest/{hash}");
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM documents WHERE name=?1 AND hash=?2)",
        params![name, hash], |row| row.get(0),
    )?;
    if !exists {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h0e4e625fc7f0 owner=fractal_store::store::ask_digest_payload TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blobs(hash,bytes,size) VALUES(?1,?2,?3)",
            params![hash, bytes, bytes.len() as u64])?;
        let binding = append_claim_tx(tx, origin, &name, "doc.bound", None,
            &json!({"name":name,"hash":hash,"size":bytes.len()}), &[], None)?;
        select_replicated_document(tx, &binding, binding.store_index)?;
    }
    Ok(format!("{name}@{hash}"))
}

pub(super) struct PayloadPage {
    pub rows: Vec<Value>,
    pub next: Option<String>,
}

/// Reads one bounded fragment page (at most 128 rows). A single oversized row may
/// span several documents and is assembled atomically; the continuation retains
/// at most one fragment's prefix, never an entire day's rows or references.
pub(super) fn read_page(connection: &Connection, source: &Value, cursor: Option<&str>) -> Result<PayloadPage> {
    if let Some(rows) = source.get("digest").and_then(Value::as_array) {
        anyhow::ensure!(source.get("digest_document").is_none() && cursor.is_none(), "invalid inline digest cursor");
        anyhow::ensure!(rows.len() <= 128, "oversized inline digest");
        return Ok(PayloadPage { rows: rows.clone(), next: None });
    }
    let state: Value = cursor.map(serde_json::from_str).transpose()?.unwrap_or_else(|| json!({
        "document":source["digest_document"],"pending":"","count":"0"
    }));
    let mut reference = state["document"].as_str().context("missing digest document")?.to_owned();
    let mut fragments = Vec::new();
    if let Some(pending) = state["pending"].as_str().filter(|s| !s.is_empty()) {
        fragments.push(pending.to_owned());
    }
    let mut count: u64 = state["count"].as_str().context("missing cursor count")?.parse()?;
    let mut rows = Vec::new();
    loop {
        let (name, hash) = reference.rsplit_once('@').context("invalid digest document reference")?;
        anyhow::ensure!(name == format!("doc/ask-digest/{hash}"), "invalid digest document name");
        let bytes: Vec<u8> = connection.query_row(
            "SELECT b.bytes FROM documents d JOIN blobs b ON b.hash=d.hash WHERE d.name=?1 AND d.hash=?2",
            params![name, hash], |row| row.get(0),
        )?;
        anyhow::ensure!(bytes.len() < 64 * 1024 && hex::encode(Sha256::digest(&bytes)) == hash,
            "invalid digest document bytes");
        let document: Value = serde_json::from_slice(&bytes)?;
        let data = document["data"].as_str().context("digest document has no data")?;
        anyhow::ensure!(data.len() <= FRAGMENT_BYTES && data.bytes().filter(|byte| *byte == b'\n').count() <= 128,
            "oversized digest fragment");
        for (index, part) in data.rsplit('\n').enumerate() {
            if index != 0 && !fragments.is_empty() {
                fragments.reverse();
                rows.push(serde_json::from_str(&fragments.concat())?);
                fragments.clear();
                count += 1;
            }
            if !part.is_empty() { fragments.push(part.to_owned()); }
        }
        let previous = document.get("previous").map(|value| value.as_str()
            .context("invalid previous digest document")).transpose()?;
        if let Some(previous) = previous {
            if !rows.is_empty() {
                fragments.reverse();
                let next = serde_json::to_string(&json!({
                    "document":previous,"pending":fragments.concat(),"count":count.to_string()
                }))?;
                return Ok(PayloadPage { rows, next: Some(next) });
            }
            reference = previous.to_owned();
        } else {
            if !fragments.is_empty() {
                fragments.reverse();
                rows.push(serde_json::from_str(&fragments.concat())?);
                count += 1;
            }
            let expected: u64 = source["digest_count"].as_str().context("digest has no count")?.parse()?;
            anyhow::ensure!(count == expected, "digest row count mismatch");
            return Ok(PayloadPage { rows, next: None });
        }
    }
}

/// Backed rows arrive in reverse emission order; identity consumers are order-independent.
#[cfg(test)]
pub(super) fn visit_rows(connection: &Connection, source: &Value, mut visit: impl FnMut(Value) -> Result<()>) -> Result<()> {
    let mut cursor = None;
    loop {
        let page = read_page(connection, source, cursor.as_deref())?;
        for row in page.rows { visit(row)?; }
        cursor = page.next;
        if cursor.is_none() { return Ok(()); }
    }
}

#[cfg(test)]
fn read_all(connection: &Connection, source: &Value) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    visit_rows(connection, source, |row| { rows.push(row); Ok(()) })?;
    if source.get("digest_document").is_some() { rows.reverse(); }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_days_and_long_unicode_titles_round_trip_with_bounded_documents() {
        let store = Store::open_memory("digest-payload").unwrap();
        let expected = (0..400).map(|index| json!({
            "title": if index == 200 { "é\\\"\n".repeat(20_000) } else { format!("Ask {index}") },
            "resolution_id":format!("{index:064x}"),
            "target_id":format!("work/{index}"), "episode":format!("episode-{index}"),
            "requester_id":"agent/example/requester", "resolved_at":"2026-11-15T08:00:00Z"
        })).collect::<Vec<_>>();
        store.connection.batched(|tx| -> Result<()> {
            let payload = write(tx, &store.origin, expected.iter().cloned().map(Ok))?;
            assert!(payload.get("digest").is_none());
            assert!(serde_json::to_vec(&payload)?.len() < 1024);
            assert_eq!(read_all(tx, &payload)?, expected);
            let mut appended = None;
            for page in expected.chunks(128) {
                appended = Some(append(tx, &store.origin, appended.as_ref(), page.iter().cloned().map(Ok))?);
            }
            let appended = appended.context("missing appended payload")?;
            assert_eq!(read_all(tx, &appended)?, expected);
            let mut cursor = None;
            let mut recovered = 0;
            loop {
                let page = read_page(tx, &appended, cursor.as_deref())?;
                assert!(page.rows.len() <= 128);
                recovered += page.rows.len();
                cursor = page.next;
                if let Some(cursor) = &cursor { assert!(cursor.len() < 64 * 1024); }
                if cursor.is_none() { break; }
            }
            assert_eq!(recovered, expected.len());
            // A large person's payload does not prevent the next person's emission.
            let small = write(tx, &store.origin, [Ok(expected[0].clone())])?;
            assert_eq!(read_all(tx, &small)?, vec![expected[0].clone()]);
            let largest: u64 = tx.query_row("SELECT max(length(bytes)) FROM blobs WHERE hash IN (SELECT hash FROM documents WHERE name LIKE 'doc/ask-digest/%')", [], |row| row.get(0))?;
            assert!(largest < 64 * 1024);
            Ok(())
        }).unwrap().unwrap();
    }
}
