//! Local, rebuildable digest indexes. Claims remain the replicated authority; local arrival
//! order discovers late replicas without confusing their acceptance time with ingestion time.
use super::*;
use fractal_schema::notice;

pub(super) const PAGE: usize = 128;
const CURSOR: &str = "ask_digest_claim_cursor_v1";
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-he2047aedbd02 owner=fractal_store::store::ask_digest_cache TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_ask_digest_pending(
 resolution_id TEXT PRIMARY KEY, episode TEXT NOT NULL, target TEXT NOT NULL,
 person TEXT, resolved_at INTEGER NOT NULL, staged_source TEXT, staged_owner TEXT);
CREATE INDEX IF NOT EXISTS local_ask_digest_due ON local_ask_digest_pending(person,resolved_at,resolution_id) WHERE staged_source IS NULL;
CREATE INDEX IF NOT EXISTS local_ask_digest_oldest ON local_ask_digest_pending(resolved_at,resolution_id) WHERE staged_source IS NULL;
CREATE INDEX IF NOT EXISTS local_ask_digest_episode ON local_ask_digest_pending(episode);
CREATE INDEX IF NOT EXISTS local_ask_digest_staged_source ON local_ask_digest_pending(staged_source,resolution_id) WHERE staged_source IS NOT NULL;
CREATE INDEX IF NOT EXISTS local_ask_digest_staged_owner ON local_ask_digest_pending(staged_owner,resolution_id) WHERE staged_owner IS NOT NULL;
CREATE TABLE IF NOT EXISTS local_ask_digest_emitted(resolution_id TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS local_ask_digest_sources(
 source_id TEXT PRIMARY KEY, person TEXT NOT NULL, date TEXT NOT NULL,
 covered_through INTEGER NOT NULL, source TEXT NOT NULL, cursor TEXT, complete INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS local_ask_digest_coverage ON local_ask_digest_sources(person,covered_through DESC);
CREATE INDEX IF NOT EXISTS local_ask_digest_source_work ON local_ask_digest_sources(complete,source_id);
CREATE TABLE IF NOT EXISTS local_ask_digest_draft(
 person TEXT PRIMARY KEY, source_id TEXT NOT NULL, date TEXT NOT NULL,
 covered_through INTEGER NOT NULL, owner TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS local_ask_digest_draft_owner ON local_ask_digest_draft(owner,person);
";

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h822dc1235889 owner=fractal_store::store::ask_digest_cache TODO=trace-statement-and-value-representation */ SCHEMA)?;
    Ok(())
}

/// At most three indexed pages are read, then only the first merged page is decoded. Return
/// true while more history remains, so the reconciler schedules an immediate work continuation.
pub(super) fn ingest(tx: &Transaction<'_>) -> Result<bool> {
    let after: u64 = tx.query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |r| r.get::<_,String>(0))?
        .parse()?;
    let mut ids = Vec::with_capacity((PAGE + 1) * 3);
    for kind in ["work.person-asked", "work.person-done", "custom.attention.notice-created"] {
        ids.extend(tx.prepare_cached("SELECT store_index,id FROM claims INDEXED BY claims_kind_index
            WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT ?3")?
            .query_map(params![kind, after, PAGE + 1], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    ids.sort_unstable();
    let more = ids.len() > PAGE;
    ids.truncate(PAGE);
    for (_, id) in &ids {
        let claim = claim_by_id_tx(tx, id)?.context("digest discovery claim disappeared")?;
        match claim.kind.as_str() {
            "work.person-done" if claim.actor.as_deref() == Some(notice::RUNTIME_ACTOR)
                && claim.body["fields"]["resolved_by"] == "deadline" => {
                let Some(episode) = claim.body["fields"]["episode"].as_str() else { continue };
                let ask = claim_by_id_tx(tx, episode)?;
                let person = ask.as_ref().filter(|ask| ask.subject == claim.subject && ask.origin == claim.origin
                    && ask.kind == "work.person-asked").and_then(|ask| ask.body["fields"]["person"].as_str());
                tx.execute(/* sqlite-large-value: allow-for-now lv-hb2f8a8e73148 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_pending(resolution_id,episode,target,person,resolved_at)
                    SELECT ?1,?2,?3,?4,?5 WHERE NOT EXISTS(SELECT 1 FROM local_ask_digest_emitted WHERE resolution_id=?1)",
                    params![claim.id,episode,claim.subject,person,i64::try_from(claim.accepted_at_unix_ms)?])?;
            }
            "work.person-asked" => {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h5b47dbcca06d owner=fractal_store::store::ask_digest_cache TODO=prepare-body-and-predecessor-values */ "UPDATE local_ask_digest_pending SET person=?1 WHERE episode=?2 AND target=?3
                    AND EXISTS(SELECT 1 FROM claims applied WHERE applied.id=resolution_id AND applied.origin=?4)",
                    params![claim.body["fields"]["person"].as_str(),claim.id,claim.subject,claim.origin])?;
            }
            "custom.attention.notice-created" if claim.subject.starts_with(notice::DIGEST_SOURCE_PREFIX)
                && notice_authority::authenticated_digest_source(tx, &claim)? => {
                let fields = custom::fields(&claim);
                let source = &fields["source"];
                let covered = source.get("covered_through").or_else(|| fields.get("created_at"))
                    .and_then(Value::as_str).context("digest history has no coverage boundary")?;
                let covered = chrono::DateTime::parse_from_rfc3339(covered)?.timestamp_millis();
                let person = fields["recipient"].as_str().context("digest history has no recipient")?;
                let date = claim.subject.rsplit('/').next().context("digest history has no date")?;
                tx.execute(/* sqlite-large-value: allow-for-now lv-h4129c5ab198d owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_sources(source_id,person,date,covered_through,source)
                    VALUES(?1,?2,?3,?4,?5)", params![claim.subject,person,date,covered,serde_json::to_string(source)?])?;
            }
            _ => {}
        }
    }
    if let Some((last, _)) = ids.last() {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h53a7c3351614 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![CURSOR,last.to_string()])?;
    }
    Ok(more)
}

/// Schema creation never scans history. The cursor's absence is the sole initialization marker.
pub(super) fn initialize(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h005b96d7003a owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,'0')", [CURSOR])?;
    Ok(())
}

/// Index reported IDs a bounded backing page at a time before issuing new digests. A late
/// replica is then either already reported in authenticated history, or remains pending.
pub(super) fn index_source_page(tx: &Transaction<'_>) -> Result<bool> {
    let source: Option<(String,String,Option<String>)> = tx.query_row(
        "SELECT source_id,source,cursor FROM local_ask_digest_sources WHERE complete=0 ORDER BY source_id LIMIT 1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id, source, cursor)) = source else { return Ok(false) };
    let source: Value = serde_json::from_str(&source)?;
    let page = ask_digest_payload::read_page(tx, &source, cursor.as_deref())?;
    for row in page.rows {
        let resolution = row["resolution_id"].as_str().context("digest row has no resolution identity")?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd329a4434419 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_ask_digest_emitted VALUES(?1)", [resolution])?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-ha9788b830cab owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_pending WHERE resolution_id=?1", [resolution])?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h9fb12654c1a9 owner=fractal_store::store::ask_digest_cache TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_sources SET cursor=?2,complete=?3 WHERE source_id=?1",
        params![id,page.next,i64::from(page.next.is_none())])?;
    Ok(true)
}
