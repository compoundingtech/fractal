//! Reader-first dependency representation. Default writers still append text edges;
//! readers include integer-key edges written by a later storage-version release.
//! Keep canonical claim IDs in the edge ordering so invalidation never sorts history.
use super::*;

pub(super) const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h9aa022ae6f1f owner=smallclaims::store::principals::dependencies TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS claim_verdict_dependency_keys (
    id INTEGER PRIMARY KEY,
    link TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS claim_verdict_dependency_edges (
    dependency_id INTEGER NOT NULL REFERENCES claim_verdict_dependency_keys(id)
        DEFERRABLE INITIALLY DEFERRED,
    claim_id TEXT NOT NULL,
    PRIMARY KEY(dependency_id,claim_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_verdict_dependency_edges_claim
    ON claim_verdict_dependency_edges(claim_id,dependency_id);
CREATE VIEW IF NOT EXISTS claim_verdict_dependency_links AS
    SELECT link,claim_id FROM claim_verdict_links
    UNION ALL
    SELECT k.link,e.claim_id FROM claim_verdict_dependency_keys k
    JOIN claim_verdict_dependency_edges e ON e.dependency_id=k.id;
"#;

// Use a compound ordered query, not DISTINCT over the UNION ALL reader view:
// SQLite can merge the two indexed claim-ID streams and stop at the page limit.
// The same static page is consumed by all three statements in a writer loan.
macro_rules! page_sql {
    ($before:literal, $after:literal) => {
        concat!(
            $before,
            "SELECT claim_id FROM claim_verdict_links WHERE link=?1 AND claim_id>?2
             UNION
             SELECT e.claim_id FROM claim_verdict_dependency_keys k
             CROSS JOIN claim_verdict_dependency_edges e ON e.dependency_id=k.id
             WHERE k.link=?1 AND e.claim_id>?2
             ORDER BY 1 LIMIT ?3",
            $after
        )
    };
}

/// Queue a bounded, deduplicated page across both representations atomically.
pub(super) fn queue_page(
    connection: &Connection,
    link: &str,
    after: &str,
) -> Result<(Option<String>, usize)> {
    let page = params![link, after, REQUEUE_PAGE as i64];
    let last = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-hfa597e4dd8d1 owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ page_sql!(
            "SELECT MAX(claim_id),COUNT(*) FROM (",
            ")"
        ))?
        .query_row(/* sqlite-large-value: allow-for-now lv-hdcbb278deb43 owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ page, |row| Ok((row.get(0)?, row.get(1)?)))?;
    connection
        .prepare_cached(page_sql!(
            /* sqlite-large-value: allow-for-now lv-h60637f5c4836 owner=smallclaims::store::principals::dependencies TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET dirty=1 WHERE claim_id IN (",
            ")"
        ))?
        .execute(/* sqlite-large-value: allow-for-now lv-h379bff36a470 owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ page)?;
    connection
        .prepare_cached(page_sql!(
            /* sqlite-large-value: allow-for-now lv-h116aa2dabcbd owner=smallclaims::store::principals::dependencies TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_queue(claim_id) ",
            ""
        ))?
        .execute(/* sqlite-large-value: allow-for-now lv-h6b1b40cffd79 owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ page)?;
    // A deleted claim keeps its links (there is no index to find them by claim): a page that
    // meets one drops it, by its own key.
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h00dcdefe45a1 owner=smallclaims::store::principals::dependencies TODO=prepare-body-and-predecessor-values */ "DELETE FROM claim_verdict_links WHERE link=?1 AND claim_id IN (
                 SELECT claim_id FROM claim_verdict_links WHERE link=?1 AND claim_id>?2
                 ORDER BY claim_id LIMIT ?3)
             AND NOT EXISTS(SELECT 1 FROM claims WHERE id=claim_verdict_links.claim_id)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h17c1066168bd owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ page)?;
    Ok(last)
}

/// Remove dictionary keys only when this claim is their last edge. Restrict the
/// orphan check through the reverse index; never scan the global dictionary.
/// The caller removes the edges in this same transaction before committing.
pub(super) fn forget_keys(connection: &Connection, claim_id: &str) -> Result<()> {
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h81af2ed7f036 owner=smallclaims::store::principals::dependencies TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_verdict_dependency_keys WHERE id IN (
                 SELECT dependency_id FROM claim_verdict_dependency_edges WHERE claim_id=?1
             ) AND NOT EXISTS (
                 SELECT 1 FROM claim_verdict_dependency_edges e
                 WHERE e.dependency_id=claim_verdict_dependency_keys.id AND e.claim_id!=?1
             )",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h0784c4c48c37 owner=smallclaims::store::principals::dependencies TODO=trace-statement-and-value-representation */ [claim_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::StatementStatus;

    #[test]
    fn mixed_dependency_pages_deduplicate_without_sorting_history() {
        fn steps(total: usize) -> i32 {
            let mut connection = Connection::open_in_memory().unwrap();
            connection.execute_batch(PRINCIPAL_SCHEMA).unwrap();
            connection.execute_batch(SCHEMA).unwrap();
            // A requeue page drops links whose claim is gone, so the claims exist.
            connection.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY)").unwrap();
            let tx = connection.transaction().unwrap();
            tx.execute("INSERT INTO claim_verdict_dependency_keys VALUES(1,'root')", []).unwrap();
            for index in 0..total {
                let claim = format!("claim/{index:08}");
                tx.execute("INSERT INTO claims(id) VALUES (?1)", [&claim]).unwrap();
                tx.execute("INSERT INTO claim_verdict_links VALUES('root',?1)", [&claim]).unwrap();
                tx.execute("INSERT INTO claim_verdict_dependency_edges VALUES(1,?1)", [&claim]).unwrap();
            }
            tx.commit().unwrap();
            let (last, count) = queue_page(&connection, "root", "").unwrap();
            assert_eq!(count, REQUEUE_PAGE);
            assert_eq!(last.as_deref(), Some("claim/00001023"));
            let steps = connection.prepare_cached(page_sql!(
                "SELECT MAX(claim_id),COUNT(*) FROM (", ")"
            )).unwrap().get_status(StatementStatus::VmStep);
            let (last, count) = queue_page(&connection, "root", last.as_deref().unwrap()).unwrap();
            assert_eq!(count, total.saturating_sub(REQUEUE_PAGE).min(REQUEUE_PAGE));
            if count != 0 {
                assert_eq!(last.as_deref(), Some("claim/00002047"));
            }
            steps
        }
        let small = steps(REQUEUE_PAGE);
        let large = steps(20_000);
        assert!(large <= small * 2, "page VM steps grew with history: {small} -> {large}");
    }

    #[test]
    fn forgetting_compact_edges_invalidates_nonce_dependents_and_removes_only_orphan_keys() {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(
            "INSERT INTO claim_verdict_dependency_keys VALUES(1,'root'),(2,'nonce:key|nonce'),(3,'private');
             INSERT INTO claim_verdict_dependency_edges VALUES(1,'forgotten'),(1,'dependent'),(2,'dependent'),(3,'forgotten');
             INSERT INTO claim_signature_nonce_uses VALUES('forgotten','key','nonce');
             INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,complete,dirty)
                 VALUES('dependent',1,'','','',1,0);"
        ).unwrap();
        forget_claim_tx(&tx, "forgotten").unwrap();
        assert!(tx.query_row("SELECT dirty FROM claim_signature_judging WHERE claim_id='dependent'", [], |row| row.get::<_, bool>(0)).unwrap());
        assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM claim_verdict_queue WHERE claim_id='dependent')", [], |row| row.get::<_, bool>(0)).unwrap());
        assert_eq!(tx.query_row("SELECT COUNT(*) FROM claim_verdict_dependency_edges WHERE claim_id='forgotten'", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
        assert_eq!(tx.query_row("SELECT COUNT(*) FROM claim_verdict_dependency_keys", [], |row| row.get::<_, u64>(0)).unwrap(), 2);
        assert!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM claim_verdict_dependency_keys WHERE link='private')", [], |row| row.get::<_, bool>(0)).unwrap());
        tx.commit().unwrap();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
    }
}
