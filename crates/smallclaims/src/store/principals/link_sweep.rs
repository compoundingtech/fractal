//! The paced sweep of verdict links whose claim is gone. Links are found by link alone (their
//! primary key); with no index by claim, deleting a claim leaves its links behind. A requeue that
//! meets one drops it; this sweep finds the rest, a page at a time in key order from a cursor in
//! meta, so a restart resumes where it stopped. A finished pass records when it ended, and the
//! next begins a day later.
use super::*;

/// Rows a loan looks at.
pub const SWEEP_PAGE: usize = 512;
/// A loan stops starting rows after this much work.
pub const SWEEP_BUDGET: std::time::Duration = std::time::Duration::from_millis(10);
/// A pass begins this long after the previous one ended.
pub const SWEEP_INTERVAL_MS: u64 = 24 * 60 * 60 * 1_000;
/// The last link the current pass finished with: its link and its claim.
const CURSOR_LINK: &str = "claim_verdict_links_sweep_link";
const CURSOR_CLAIM: &str = "claim_verdict_links_sweep_claim";
/// When the last pass ended, in Unix milliseconds.
const PASSED: &str = "claim_verdict_links_sweep_passed";

fn meta(connection: &Connection, key: &str) -> Result<Option<String>> {
    Ok(connection
        .prepare_cached("SELECT value FROM meta WHERE key=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h922df61b1780 owner=smallclaims::store::principals::link_sweep TODO=trace-statement-and-value-representation */ [key], |row| row.get(0))
        .optional()?)
}

/// Milliseconds until the next pass may start, or none when one is due or under way.
pub(super) fn wait_ms(connection: &Connection, now_ms: u64) -> Result<Option<u64>> {
    if meta(connection, CURSOR_LINK)?.is_some() {
        return Ok(None);
    }
    Ok(meta(connection, PASSED)?
        .and_then(|value| value.parse::<u64>().ok())
        .map(|passed| passed.saturating_add(SWEEP_INTERVAL_MS))
        .filter(|next| *next > now_ms)
        .map(|next| next - now_ms))
}

/// One loan of the sweep: up to `SWEEP_PAGE` links after the cursor, each dropped when no claim
/// row has its claim ID. Returns the links looked at, those dropped, and whether the pass ended.
pub(super) fn sweep_page_tx(connection: &Connection, now_ms: u64) -> Result<(usize, usize, bool)> {
    let started = std::time::Instant::now();
    let after = (
        meta(connection, CURSOR_LINK)?.unwrap_or_default(),
        meta(connection, CURSOR_CLAIM)?.unwrap_or_default(),
    );
    let rows = connection
        .prepare_cached(
            "SELECT link,claim_id FROM claim_verdict_links
             WHERE (link,claim_id)>(?1,?2) ORDER BY link,claim_id LIMIT ?3",
        )?
        .query_map(params![after.0, after.1, SWEEP_PAGE as i64], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut last = None;
    let (mut examined, mut deleted) = (0, 0);
    for (link, claim_id) in &rows {
        if examined != 0 && started.elapsed() >= SWEEP_BUDGET {
            break;
        }
        examined += 1;
        let held: bool = connection
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?
            .query_row(/* sqlite-large-value: allow-for-now lv-h4c0ec222e58f owner=smallclaims::store::principals::link_sweep TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))?;
        if !held {
            deleted += connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-hce3c113dc5dd owner=smallclaims::store::principals::link_sweep TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_verdict_links WHERE link=?1 AND claim_id=?2")?
                .execute(/* sqlite-large-value: allow-for-now lv-he16189f1be07 owner=smallclaims::store::principals::link_sweep TODO=trace-statement-and-value-representation */ params![link, claim_id])?;
        }
        last = Some((link, claim_id));
    }
    let finished = examined == rows.len() && rows.len() < SWEEP_PAGE;
    if finished {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h158fd1090b10 owner=smallclaims::store::principals::link_sweep TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN (?1,?2)", params![CURSOR_LINK, CURSOR_CLAIM])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hfe62f5c13d7c owner=smallclaims::store::principals::link_sweep TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![PASSED, now_ms.to_string()],
        )?;
    } else if let Some((link, claim_id)) = last {
        for (key, value) in [(CURSOR_LINK, link), (CURSOR_CLAIM, claim_id)] {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h88259e3363a1 owner=smallclaims::store::principals::link_sweep TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
        }
    }
    Ok((examined, deleted, finished))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimInput;
    use crate::fleet::MemberKey;
    use std::sync::Mutex;

    fn signed_store(notes: usize) -> (Store, Vec<String>) {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        store.set_node_key(Arc::new(MemberKey::generate().unwrap().0)).unwrap();
        let ids = (0..notes)
            .map(|index| {
                store
                    .append_claim(&ClaimInput {
                        subject: format!("note/{index}"),
                        kind: "example.note".into(),
                        actor: None,
                        fields: serde_json::from_value(serde_json::json!({"text": index})).unwrap(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: None,
                    })
                    .unwrap()
                    .id
            })
            .collect();
        store.replication_snapshot().unwrap();
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
        }
        (store, ids)
    }

    fn links_of(store: &Store, claim_id: &str) -> i64 {
        store.readers.get()
            .query_row("SELECT COUNT(*) FROM claim_verdict_links WHERE claim_id=?1", [claim_id], |row| row.get(0))
            .unwrap()
    }

    /// As a trim or mail retirement does: the claim row goes, and forget clears what it can.
    fn delete_claim(store: &Store, claim_id: &str) {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        forget_claim_tx(&tx, claim_id).unwrap();
        tx.execute("DELETE FROM claims WHERE id=?1", [claim_id]).unwrap();
        tx.commit().unwrap();
    }

    fn sweep(store: &Store, now: u64) -> (usize, usize, usize) {
        let (mut loans, mut examined, mut deleted) = (0, 0, 0);
        loop {
            let page = store.sweep_orphan_verdict_links(now).unwrap();
            loans += 1;
            examined += page.examined;
            deleted += page.deleted;
            if page.finished {
                return (loans, examined, deleted);
            }
        }
    }

    #[test]
    fn a_deleted_claims_links_go_with_the_sweep_and_the_next_pass_waits_a_day() {
        let (store, ids) = signed_store(3);
        let total: i64 = store.readers.get().query_row("SELECT COUNT(*) FROM claim_verdict_links", [], |row| row.get(0)).unwrap();
        let gone = links_of(&store, &ids[0]);
        assert!(gone > 0);
        delete_claim(&store, &ids[0]);
        assert_eq!(links_of(&store, &ids[0]), gone, "forget no longer looks links up by claim");
        let now = 1_000_000;
        assert_eq!(store.verdict_link_sweep_wait(now).unwrap(), None, "the first pass is due");
        let (_, examined, deleted) = sweep(&store, now);
        assert_eq!((examined as i64, deleted as i64), (total, gone));
        assert_eq!(links_of(&store, &ids[0]), 0);
        assert!(ids[1..].iter().all(|id| links_of(&store, id) > 0), "live claims keep their links");
        assert_eq!(store.verdict_link_sweep_wait(now + 1).unwrap(), Some(SWEEP_INTERVAL_MS - 1));
        assert_eq!(store.verdict_link_sweep_wait(now + SWEEP_INTERVAL_MS).unwrap(), None);
        assert_eq!(store.claim_verdict(&ids[1]).unwrap(), crate::principal::Verdict::Verified);
    }

    #[test]
    fn the_cursor_resumes_a_pass_rather_than_starting_it_again() {
        let (store, ids) = signed_store(1);
        {
            let connection = store.connection.write();
            for index in 0..(3 * SWEEP_PAGE) {
                // Half for a claim that is gone, half for one that is held.
                let claim = if index % 2 == 0 { format!("claim/gone-{index:05}") } else { ids[0].clone() };
                connection.execute("INSERT INTO claim_verdict_links(link,claim_id) VALUES (?1,?2)",
                    params![format!("key:example-{index:05}"), claim]).unwrap();
            }
        }
        let total: usize = store.readers.get().query_row("SELECT COUNT(*) FROM claim_verdict_links", [], |row| row.get(0)).unwrap();
        let first = store.sweep_orphan_verdict_links(7).unwrap();
        assert!(!first.finished && first.examined > 0);
        assert_eq!(store.verdict_link_sweep_wait(7).unwrap(), None, "a pass under way stays due");
        // The next loans take up after the first, as they would after a restart: the cursor is in
        // meta, and every link is looked at exactly once.
        let (_, rest, deleted) = sweep(&store, 7);
        assert_eq!(first.examined + rest, total);
        assert_eq!(first.deleted + deleted, 3 * SWEEP_PAGE / 2);
    }

    #[test]
    fn a_requeue_that_meets_a_deleted_claims_link_drops_it() {
        let (store, ids) = signed_store(2);
        let link: String = store.readers.get().query_row(
            "SELECT link FROM claim_verdict_links WHERE claim_id=?1 LIMIT 1", [&ids[0]], |row| row.get(0)).unwrap();
        delete_claim(&store, &ids[0]);
        let held = |claim: &str| -> bool {
            store.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM claim_verdict_links WHERE link=?1 AND claim_id=?2)",
                [&link, &claim.to_owned()], |row| row.get(0)).unwrap()
        };
        assert!(held(&ids[0]));
        {
            let connection = store.connection.write();
            dependencies::queue_page(&connection, &link, "").unwrap();
        }
        assert!(!held(&ids[0]), "the requeue dropped the deleted claim's link");
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
        }
        assert_eq!(store.claim_verdict(&ids[1]).unwrap(), crate::principal::Verdict::Verified);
    }

    static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn capture(sql: &str) {
        CAPTURED.lock().unwrap().push(sql.to_owned());
    }

    /// Every statement `work` runs on the writer.
    fn statements(store: &Store, work: impl FnOnce()) -> Vec<String> {
        CAPTURED.lock().unwrap().clear();
        store.connection.write().trace(Some(capture));
        work();
        store.connection.write().trace(None);
        std::mem::take(&mut *CAPTURED.lock().unwrap())
    }

    /// The plan lines that walk all of claim_verdict_links, for each statement that names it.
    fn link_scans(store: &Store, statements: &[String]) -> Vec<String> {
        let reader = store.readers.get();
        let mut scans = Vec::new();
        for sql in statements.iter().filter(|sql| sql.contains("claim_verdict_links")) {
            let Ok(mut plan) = reader.prepare(&format!("EXPLAIN QUERY PLAN {sql}")) else { continue };
            let lines = plan.query_map([], |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap();
            scans.extend(lines.into_iter().filter(|line| line.starts_with("SCAN claim_verdict_links"))
                .map(|line| format!("{line}  <=  {sql}")));
        }
        scans
    }

    #[test]
    fn deleting_requeuing_judging_and_sweeping_never_scan_the_links() {
        let (store, ids) = signed_store(3);
        let index: bool = store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='claim_candidate_verdict_links_claim_index')",
            [], |row| row.get(0)).unwrap();
        assert!(!index, "the index by claim is gone");
        let link: String = store.readers.get().query_row(
            "SELECT link FROM claim_verdict_links WHERE claim_id=?1 LIMIT 1", [&ids[1]], |row| row.get(0)).unwrap();
        let mut ran = statements(&store, || delete_claim(&store, &ids[0]));
        ran.extend(statements(&store, || {
            let connection = store.connection.write();
            dependencies::queue_page(&connection, &link, "").unwrap();
        }));
        ran.extend(statements(&store, || {
            while store.verdicts_pending().unwrap() {
                store.advance_signature_judgment().unwrap();
            }
        }));
        ran.extend(statements(&store, || { sweep(&store, 1); }));
        assert!(ran.iter().any(|sql| sql.contains("claim_verdict_links")), "the traced work touched the links");
        assert_eq!(link_scans(&store, &ran), Vec::<String>::new());
    }
}
