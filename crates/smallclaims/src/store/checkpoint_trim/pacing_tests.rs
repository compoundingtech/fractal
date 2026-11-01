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
    assert_eq!(
        store.trim_pause_duration(Duration::from_secs(7)),
        Duration::from_secs(5)
    );
    store.set_trim_pause(Duration::ZERO);
    assert_eq!(
        store.trim_pause_duration(Duration::from_secs(7)),
        Duration::ZERO
    );
}

#[test]
fn a_local_walk_visits_only_its_own_cuts_tombstones() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h1b0708ce8e80 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE checkpoint_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
             accepted_at_unix_ms INTEGER, checkpoint TEXT, PRIMARY KEY(writer,sequence,envelope_hash));
         CREATE TABLE replica_envelopes(writer TEXT, sequence INTEGER, envelope_hash TEXT,
             PRIMARY KEY(writer,sequence,envelope_hash));
         CREATE TABLE checkpoint_claims(id TEXT PRIMARY KEY, operation_id TEXT, writer TEXT,
             sequence INTEGER, envelope_hash TEXT, checkpoint TEXT);
         CREATE INDEX claim_envelope ON checkpoint_claims(writer,sequence,envelope_hash);
         CREATE TABLE claims(id TEXT PRIMARY KEY);",
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    // Every earlier checkpoint's tombstones come first, then the cut's own.
    for sequence in 1..=10_000 {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h3ed673b5c629 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes VALUES('w', ?1, 'h', 0, 'checkpoint/old')", [sequence]).unwrap();
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h55b982191075 owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims VALUES(printf('old-%05d', ?1), NULL, 'w', ?1, 'h', 'checkpoint/old')",
            [sequence],
        )
        .unwrap();
    }
    for sequence in 1..=2 {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h05e8b31a243c owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes VALUES('cut', ?1, 'h', 0, 'checkpoint/cut')", [sequence]).unwrap();
    }
    for n in 1..=3 {
        tx.execute(/* sqlite-large-value: allow-for-now lv-h02af172fc00e owner=smallclaims::store::checkpoint_trim::pacing_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims VALUES(printf('cut-%d', ?1), NULL, 'cut', 9, 'h', 'checkpoint/cut')", [n])
            .unwrap();
    }
    tx.commit().unwrap();
    crate::sqlite::observe(&mut db);
    let tx = db.transaction().unwrap();
    let mut cursor = TrimCursor {
        local: Some(LocalWalk { checkpoint: "checkpoint/cut".into(), envelope: 10_000, claim: 10_000 }),
        ..TrimCursor::default()
    };
    let work = crate::sqlite::work::SqliteWorkScope::start();
    let mut steps = 0;
    while next_trim_rows(&tx, &mut cursor).unwrap().is_some() {
        steps += 1;
    }
    let work = work.finish();
    assert_eq!(steps, 5, "two envelopes and three claims, and none of the 20,000 earlier tombstones");
    assert!(work.statements > 0, "SQLite work instrumentation did not run");
    assert!(work.vm_steps < 2_000, "the walk read past its own tombstones: {work:?}");
}

#[test]
fn a_local_cut_trims_only_its_tombstones_and_goes_on_from_where_it_stopped() {
    let store = Arc::new(Store::open_memory("alder", Arc::new(Plain)).unwrap());
    store.trim_locally(true).unwrap();
    let claims: Vec<_> = (0..5)
        .map(|n| {
            store
                .append_claim(&ClaimInput {
                    subject: format!("note/{n}"),
                    kind: "example.note".into(),
                    actor: None,
                    fields: BTreeMap::from([("text".into(), json!(n))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap()
        })
        .collect();
    let tombstone = |claim: &ClaimRecord| ClaimTombstone {
        id: claim.id.clone(),
        operation_id: claim.operation_id.clone(),
        writer: "alder".into(),
        sequence: 1,
        envelope_hash: "envelope".into(),
        subject: claim.subject.clone(),
        kind: claim.kind.clone(),
        actor: claim.actor.clone(),
        predecessors: claim.predecessors.clone(),
        request_digest: claim.request_digest.clone(),
        accepted_at_unix_ms: claim.accepted_at_unix_ms,
    };
    let (old, cut) = ("checkpoint/2026-01-01", "checkpoint/2026-01-02");
    {
        // An earlier checkpoint's tombstone whose claim is still here: only a walk of every
        // tombstone would reach it.
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        record_checkpoint_tombstones_tx(&tx, old, &[], &[tombstone(&claims[0])]).unwrap();
        tx.commit().unwrap();
    }
    store.set_checkpoint_state(cut, 2, "trimming").unwrap();
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let cut_tombstones: Vec<_> = claims[1..].iter().map(tombstone).collect();
        record_checkpoint_tombstones_tx(&tx, cut, &[], &cut_tombstones).unwrap();
        tx.commit().unwrap();
    }
    let exists = |id: &str| store.claim_by_id(id).unwrap().is_some();
    store.set_trim_chunk_envelopes(1);
    store.set_trim_fault(Some(TrimFault::AfterChunk(2)));
    let mut actions = Vec::new();
    assert!(store.finish_trim_paced(cut, &mut actions, |_| {}).is_err());
    // Two chunks deleted two claims; the walk kept its place.
    assert_eq!(claims[1..].iter().filter(|claim| !exists(&claim.id)).count(), 2);
    let walk = store.local_walk(cut).unwrap().unwrap();
    let second: i64 = store
        .readers
        .get()
        .query_row("SELECT rowid FROM checkpoint_claims WHERE id=?1", [&claims[2].id], |row| row.get(0))
        .unwrap();
    assert_eq!(walk.claim, second);
    store.finish_trim_paced(cut, &mut actions, |_| {}).unwrap();
    assert!(matches!(actions.last(), Some(CheckpointAction::Trimmed { claims: 2, .. })), "{actions:?}");
    assert!(claims[1..].iter().all(|claim| !exists(&claim.id)));
    assert!(exists(&claims[0].id), "an earlier checkpoint's tombstone is not this cut's to walk");
    assert_eq!(local_checkpoint(&store.readers.get(), cut).unwrap().unwrap().0, "trimmed");
    let left: Option<String> = store
        .readers
        .get()
        .query_row("SELECT value FROM meta WHERE key=?1", [local_walk_key(cut)], |row| row.get(0))
        .optional()
        .unwrap();
    assert_eq!(left, None, "a finished walk leaves nothing behind");
}
