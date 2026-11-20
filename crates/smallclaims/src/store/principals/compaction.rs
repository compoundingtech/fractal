//! Reader side of storage version 24: a compacted claim keeps its facade as its only candidate.
//!
//! A settled claim with one candidate keeps its `claim_signatures` facade (inline text), its
//! `claim_signature_counts` row (candidates = 1), its verdict, nonce use and dependency links,
//! and drops its candidate and source rows. A counted claim without candidate rows is
//! therefore compacted: readers take the facade as its single candidate, and any write of a
//! candidate first expands the facade back into a candidate and a source, so an exact
//! duplicate still adds nothing and a second candidate never loses the first.
//!
//! The compaction writer ships in a later release, behind its own meta flag; this release only
//! reads compacted claims and expands them.
use super::*;

/// The facade of a compacted claim: counted, without candidate rows.
pub(super) fn facade(connection: &Connection, claim_id: &str) -> Result<Option<String>> {
    let value = connection
        .prepare_cached(
            "SELECT s.signature FROM claim_signatures s
              WHERE s.claim_id=?1
                AND EXISTS(SELECT 1 FROM claim_signature_counts c WHERE c.claim_id=s.claim_id)
                AND NOT EXISTS(SELECT 1 FROM claim_signature_candidates d WHERE d.claim_id=s.claim_id)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-ha5155cd52ec4 owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get::<_, SqlValue>(0))
        .optional()?;
    value
        .map(|value| references::resolve(connection, claim_id, value))
        .transpose()
}

/// Expand a compacted claim back into its candidate and source rows before a candidate write.
/// The source is the claim's batch origin, which is what wrote the single candidate.
pub(super) fn expand_tx(connection: &Connection, claim_id: &str) -> Result<bool> {
    let Some(text) = facade(connection, claim_id)? else {
        return Ok(false);
    };
    let signature: ClaimSignature = serde_json::from_str(&text)?;
    let writer: String = connection
        .prepare_cached(
            "SELECT b.origin FROM claims c JOIN batches b ON b.id=c.batch_id WHERE c.id=?1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h1c1e2c4418af owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
        .optional()?
        .unwrap_or_else(|| "unknown-legacy".to_owned());
    // The candidate trigger counts the restored row, so drop the compacted count first.
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h92b0e591751c owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_signature_counts WHERE claim_id=?1")?
        .execute(/* sqlite-large-value: allow-for-now lv-h428772d36dee owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ [claim_id])?;
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h35cb2b585420 owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_signature_candidates(claim_id,signature,key,nonce) VALUES (?1,?2,?3,?4)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h2a1fb85fc3c3 owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ params![claim_id, text, signature.key, signature.nonce])?;
    connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hc5e3af7efbd3 owner=smallclaims::store::principals::compaction TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_signature_sources(claim_id,signature,writer) VALUES (?1,?2,?3)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-hfddd7253665c owner=smallclaims::store::principals::compaction TODO=trace-statement-and-value-representation */ params![claim_id, text, writer])?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimInput;
    use crate::fleet::MemberKey;

    #[test]
    fn forgetting_a_compacted_claim_leaves_no_count_behind() {
        let store = Store::open_memory("studio", Arc::new(runtime::Plain)).unwrap();
        store.set_node_key(Arc::new(MemberKey::generate().unwrap().0)).unwrap();
        let claim = store
            .append_claim(&ClaimInput {
                subject: "note/plans".into(),
                kind: "example.note".into(),
                actor: None,
                fields: serde_json::from_value(serde_json::json!({"text": "forgotten"})).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        store.replication_snapshot().unwrap();
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
        }
        let signature = store.claim_signature(&claim.id).unwrap().unwrap();
        let count = |connection: &Connection| -> i64 {
            connection
                .query_row("SELECT COUNT(*) FROM claim_signature_counts WHERE claim_id=?1", [&claim.id], |row| row.get(0))
                .unwrap()
        };
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        // Compacted as the version 24 writer leaves it: the count at 1, no candidate or source
        // (the candidate's delete trigger takes the source with it).
        tx.execute("DELETE FROM claim_signature_candidates WHERE claim_id=?1", [&claim.id]).unwrap();
        assert_eq!(tx.query_row("SELECT COUNT(*) FROM claim_signature_sources WHERE claim_id=?1", [&claim.id], |row| row.get::<_, i64>(0)).unwrap(), 0);
        tx.execute("INSERT INTO claim_signature_counts(claim_id,candidates) VALUES (?1,1)", [&claim.id]).unwrap();
        assert!(facade(&tx, &claim.id).unwrap().is_some());

        forget_claim_tx(&tx, &claim.id).unwrap();
        assert_eq!(count(&tx), 0, "a forgotten compacted claim keeps no count");
        // Should it come back, its one candidate counts once.
        store_claim_signature_tx(&tx, &claim.id, &signature).unwrap();
        assert_eq!(count(&tx), 1);
        tx.commit().unwrap();
        drop(connection);
        while store.verdicts_pending().unwrap() {
            store.advance_signature_judgment().unwrap();
        }
        assert_eq!(store.claim_verdict(&claim.id).unwrap(), crate::principal::Verdict::Verified);
    }
}
