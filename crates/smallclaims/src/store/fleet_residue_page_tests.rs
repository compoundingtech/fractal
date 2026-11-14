//! The admission residue's page reads the envelope keys from an index and counts what reading
//! every envelope row counted (#5171).
use super::*;
use crate::store::runtime::Plain;

/// The page as it read every envelope row, payload beside it, through `replica_envelopes_state`.
const ROW_PAGE_QUERY: &str = "SELECT envelopes.sequence,
        (SELECT group_concat(member_key, ' ') FROM replica_envelope_signatures AS signatures
         WHERE signatures.writer=envelopes.writer
           AND signatures.sequence=envelopes.sequence
           AND signatures.envelope_hash=envelopes.envelope_hash)
 FROM replica_envelopes AS envelopes
 WHERE envelopes.receipt_state='validated' AND envelopes.writer=?1
   AND envelopes.sequence>=?2 AND envelopes.sequence<=?3
 ORDER BY envelopes.sequence LIMIT ?4";

const WRITER: &str = "invented-writer";

/// Envelopes of every state, two at one sequence, with zero, one or two signers each.
fn envelopes(store: &Store) {
    let connection = store.connection.write();
    let envelope = |sequence: i64, hash: &str, state: &str| {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-had58f03602a7 owner=smallclaims::store::fleet_residue_page_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms,
                     payload, relay, receipt_state, received_at_unix_ms)
                 VALUES (?1, ?2, ?3, '0', zeroblob(8192), ?1, ?4, '0')",
                params![WRITER, sequence, hash, state],
            )
            .unwrap();
    };
    let signer = |sequence: i64, hash: &str, key: &str| {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-ha1440c494a1a owner=smallclaims::store::fleet_residue_page_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_signatures(writer, sequence, envelope_hash, member_key,
                     signature, stored_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, 'invented-signature', '0')",
                params![WRITER, sequence, hash, key],
            )
            .unwrap();
    };
    for sequence in 1..=40 {
        let hash = format!("{sequence:064x}");
        let state = match sequence % 9 {
            4 => "pending",
            7 => "degraded",
            _ => "validated",
        };
        envelope(sequence, &hash, state);
        if sequence % 3 != 0 {
            signer(sequence, &hash, "invented-key-a");
        }
        if sequence % 5 == 0 {
            signer(sequence, &hash, "invented-key-b");
        }
    }
    // A fork: a second envelope at a validated sequence and one at a pending one.
    for (sequence, state) in [(10, "validated"), (13, "validated"), (22, "pending")] {
        let hash = format!("f{sequence:063x}");
        envelope(sequence, &hash, state);
        signer(sequence, &hash, "invented-key-c");
    }
}

/// Every envelope a page query visits, as the residue pages through them `page` at a time.
fn visited(store: &Store, query: &str, page: usize) -> Vec<(u64, Option<String>)> {
    let mut read = |at_least: u64, through: Option<u64>| -> Result<Vec<(u64, Option<String>)>> {
        let connection = store.readers.get();
        let rows = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h3f6355c99f88 owner=smallclaims::store::fleet_residue_page_tests TODO=trace-statement-and-value-representation */ query)?
            .query_map(
                params![
                    WRITER,
                    at_least as i64,
                    through.map_or(i64::MAX, |through| through as i64),
                    if through.is_some() { -1 } else { page as i64 },
                ],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, Option<String>>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    };
    let mut seen = Vec::new();
    visit_sequence_pages(page, &mut read, &mut |sequence, signers| seen.push((sequence, signers)))
        .unwrap();
    seen.sort();
    seen
}

#[test]
fn the_residue_page_visits_what_reading_every_envelope_row_visited() {
    let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
    envelopes(&store);
    let everything = visited(&store, ROW_PAGE_QUERY, RESIDUE_PAGE);
    assert_eq!(everything.len(), 33, "the 31 validated envelopes and the two validated forks");
    for page in [1, 2, 3, 7, RESIDUE_PAGE] {
        assert_eq!(visited(&store, RESIDUE_PAGE_QUERY, page), everything, "page {page}");
        assert_eq!(visited(&store, ROW_PAGE_QUERY, page), everything, "page {page}");
    }
}

#[test]
fn the_residue_page_reads_envelope_keys_from_an_index_not_envelope_rows() {
    let store = Store::open_memory("invented-origin", Arc::new(Plain)).unwrap();
    let connection = store.readers.get();
    let mut plan = connection
        .prepare(/* sqlite-large-value: allow-for-now lv-hf0b1a3c8593a owner=smallclaims::store::fleet_residue_page_tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN QUERY PLAN {RESIDUE_PAGE_QUERY}"))
        .unwrap();
    let plan = plan
        .query_map(params![WRITER, 0, i64::MAX, RESIDUE_PAGE as i64], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter().any(|step| step.starts_with("SEARCH envelopes USING COVERING INDEX sqlite_autoindex_replica_envelopes_1")),
        "{plan:#?}"
    );
    assert!(
        plan.iter().any(|step| step.starts_with("SEARCH signatures USING COVERING INDEX")),
        "{plan:#?}"
    );
}
