use super::*;
use crate::store::runtime::Plain;
use std::sync::{Arc, mpsc};
use std::time::Duration;

#[test]
fn already_deleted_tombstones_advance_without_claiming_exhaustion() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hc7e46360ec92 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE checkpoint_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
             accepted_at_unix_ms INTEGER, PRIMARY KEY(writer,sequence,envelope_hash));
         CREATE TABLE replica_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
             PRIMARY KEY(writer,sequence,envelope_hash));
         CREATE TABLE checkpoint_claims(id TEXT PRIMARY KEY,operation_id TEXT,writer TEXT,
             sequence INTEGER,envelope_hash TEXT);
         CREATE INDEX claim_envelope ON checkpoint_claims(writer,sequence,envelope_hash);
         CREATE TABLE claims(id TEXT PRIMARY KEY);",
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    for sequence in 1..=10_001 {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h9c52ebff18d6 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes VALUES('writer',?1,'hash',0)",
            [sequence],
        )
        .unwrap();
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h0f82774c88b3 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes VALUES('writer',10001,'hash')",
        [],
    )
    .unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h8c96c824c32a owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims VALUES('a-gone',NULL,'other',1,'other')",
        [],
    )
    .unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h6aaa2338f314 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims VALUES('z-held',NULL,'other',2,'other')",
        [],
    )
    .unwrap();
    tx.execute(/* sqlite-large-value: allow-for-now lv-h8f0e71b32555 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES('z-held')", [])
        .unwrap();
    tx.commit().unwrap();
    crate::sqlite::observe(&mut db);
    let tx = db.transaction().unwrap();
    let mut cursor = TrimCursor::default();
    let work = crate::sqlite::work::SqliteWorkScope::start();
    let first = next_trim_rows(&tx, &mut cursor).unwrap();
    let work = work.finish();
    assert!(
        work.statements > 0,
        "SQLite work instrumentation did not run"
    );
    assert!(matches!(first, Some((None, ref rows)) if rows.is_empty()));
    assert_eq!(cursor.envelope.1, 1);
    // The old EXISTS-filtered LIMIT 1 traversed all 10,000 absent identities in one loan.
    assert!(
        work.vm_steps < 1_000,
        "one cursor step did unbounded work: {work:?}"
    );
    // Simulate yielding/reacquiring the loan: the cursor, not an empty delete set, is progress.
    for _ in 2..=10_000 {
        assert!(
            matches!(next_trim_rows(&tx, &mut cursor).unwrap(), Some((None, ref rows)) if rows.is_empty())
        );
    }
    assert!(
        matches!(next_trim_rows(&tx, &mut cursor).unwrap(), Some((Some(ref envelope), _)) if envelope.sequence==10_001)
    );
    assert!(
        matches!(next_trim_rows(&tx, &mut cursor).unwrap(), Some((None, ref rows)) if rows.is_empty())
    );
    assert!(
        matches!(next_trim_rows(&tx, &mut cursor).unwrap(), Some((None, ref rows)) if rows.len()==1 && rows[0].id=="z-held")
    );
    assert!(next_trim_rows(&tx, &mut cursor).unwrap().is_none());
}

#[test]
fn every_committed_chunk_yields_without_a_writer_and_keeps_its_tombstones() {
    let store = Arc::new(Store::open_memory("alder", Arc::new(Plain)).unwrap());
    let checkpoint = "checkpoint/2026-01-01";
    store
        .set_checkpoint_state(checkpoint, 1, "trimming")
        .unwrap();
    let mut tombstones = Vec::new();
    for n in 0..3 {
        let claim = store
            .append_claim(&ClaimInput {
                subject: format!("note/{n}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::from([("text".into(), json!(n))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        tombstones.push(ClaimTombstone {
            id: claim.id,
            operation_id: claim.operation_id,
            writer: "alder".into(),
            sequence: n,
            envelope_hash: format!("envelope-{n}"),
            subject: claim.subject,
            kind: claim.kind,
            actor: claim.actor,
            predecessors: claim.predecessors,
            request_digest: claim.request_digest,
            accepted_at_unix_ms: claim.accepted_at_unix_ms,
        });
    }
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        record_checkpoint_tombstones_tx(&tx, checkpoint, &[], &tombstones).unwrap();
        let mut absent = tombstones[0].clone();
        absent.id = "!already-deleted".into();
        record_checkpoint_tombstones_tx(&tx, checkpoint, &[], &[absent]).unwrap();
        tx.commit().unwrap();
    }
    store.set_trim_chunk_envelopes(1);
    let before = graph_digest(&store.readers.get()).unwrap();
    let mut pauses = 0;
    let mut actions = Vec::new();
    let generation = store.replica_generation.load(Ordering::Acquire);
    store.set_trim_fault(Some(TrimFault::AfterChunk(1)));
    assert!(
        store
            .finish_trim_paced(checkpoint, &mut actions, |_| panic!(
                "fault must interrupt before resting"
            ))
            .is_err()
    );
    assert_eq!(store.replica_generation.load(Ordering::Acquire), generation);
    assert_eq!(
        local_checkpoint(&store.readers.get(), checkpoint)
            .unwrap()
            .unwrap()
            .0,
        "trimming"
    );
    assert!(actions.is_empty());
    // The first resumed loan has an expired budget and finds only an absent candidate.
    // It must yield progress, rather than declare this unfinished checkpoint exhausted.
    store.set_trim_fault(Some(TrimFault::ExpireChunkBudget));
    store
        .finish_trim_paced(checkpoint, &mut actions, |pause| {
            assert!(pause >= Duration::from_millis(100));
            pauses += 1;
            let foreground_store = store.clone();
            let (sent, received) = mpsc::channel();
            let foreground = std::thread::spawn(move || {
                let result = foreground_store.connection.write().execute(
                    /* sqlite-large-value: allow-for-now lv-h6cab025f7761 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('foreground-during-trim','ok')",
                    [],
                );
                sent.send(result.is_ok()).unwrap();
            });
            // A synchronization guard, not a performance SLO: retaining the loan deadlocks this
            // writer until the trim returns, so it cannot signal from inside the pause callback.
            assert!(received.recv_timeout(Duration::from_secs(5)).unwrap());
            foreground.join().unwrap();
        })
        .unwrap();
    assert_eq!(pauses, 4);
    assert_eq!(
        local_checkpoint(&store.readers.get(), checkpoint)
            .unwrap()
            .unwrap()
            .0,
        "trimmed"
    );
    for tombstone in &tombstones {
        assert!(
            !store
                .readers
                .get()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)",
                    [&tombstone.id],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
    }
    assert_eq!(
        store
            .readers
            .get()
            .query_row("SELECT count(*) FROM checkpoint_claims", [], |row| row
                .get::<_, usize>(0))
            .unwrap(),
        4
    );
    assert_eq!(graph_digest(&store.readers.get()).unwrap(), before);
    assert!(matches!(
        actions.last(),
        Some(CheckpointAction::Trimmed { claims: 3, .. })
    ));
}
