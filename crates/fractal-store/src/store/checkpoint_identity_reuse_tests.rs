//! A seal pass reuses the sealed-identity digest while nothing it covers has changed, and every
//! change that can alter the digest makes it walk again (`SealedIdentityMemo` in smallclaims).
use super::checkpoint::*;
use super::*;
use crate::store::checkpoint_agreement_tests_shared_attention_tests::{context, kinds, observe, step, sync};
use crate::store::checkpoint_tests_shared_seats::{AGENT, input};
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};

/// The most a reused digest costs: sealing nothing new, then one snapshot of the seal row and
/// guard with its quiet check (4 statements and about 50 VM steps here; a walk of 640 envelopes
/// took 57 and 32,369).
const REUSE_STATEMENTS: u64 = 8;
const REUSE_VM_STEPS: u64 = 400;

/// A seal pass waiting on a peer that has not sealed reads the identities every few minutes. On
/// an unchanged store it reuses the digest instead of walking every envelope again.
#[test]
fn a_waiting_seal_pass_reuses_the_unchanged_identity_digest() {
    let scratch = tempfile::tempdir().unwrap();
    let mut context = context(scratch.path(), 0);
    // A configured peer that never seals keeps alder sealed and waiting.
    context.configured_peers = vec!["birch".into()];
    let store = Store::open_memory("alder").unwrap();
    observe(&store, 640);
    let cut = newest_due_cut(context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["sealed"]);
    let walked = store.checkpoint_sealed_identities_paged(cut, None, 64).unwrap();
    assert_eq!(walked.count, 640);
    // Before reuse, with 640 envelopes, every waiting pass walked all of them in 11 pages:
    // step 64 statements / 35,134 VM steps, status 65 / 35,163, identities 57 / 32,369. With
    // it, step 16 / 2,903 (the first, which registers the new seal row, 20 / 3,040), status
    // 12 / 2,844 and identities 4 / 50; what remains of a step is mostly the writer listing.
    for _ in 0..3 {
        let scope = SqliteWorkScope::start();
        assert!(step(&store, &context).is_empty());
        let work = scope.finish();
        assert!(work.statements <= 22 && work.vm_steps <= 3_300, "step: {work:?}");
    }
    for _ in 0..2 {
        let scope = SqliteWorkScope::start();
        let status = store.checkpoint_status(context.now_unix_ms, &context.configured_peers).unwrap();
        let work = scope.finish();
        assert_eq!(status.pending.unwrap().terms.sealed_digest, walked.digest);
        assert!(work.statements <= 14 && work.vm_steps <= 3_100, "status: {work:?}");
    }
    let (reused, work) = identities(&store, cut);
    assert_eq!(reused, walked);
    assert_reused(work);
}

/// After every kind of change the reused digest could miss, the digest a seal reads is the one
/// a full walk reads; the changes that alter the set alter it, and each makes the read walk.
#[test]
fn a_reused_identity_digest_is_the_walked_one_after_every_change_it_covers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "alder").unwrap();
    let cut: u128 = 1_000_000;
    let mut claims = Vec::new();
    for n in 0..150_u128 {
        store.set_write_clock_at(1_000 + n).unwrap();
        // Harness transitions, which a trim keeps, between warnings, which it drops.
        let claim = if n % 2 == 0 {
            let state = if n % 4 == 0 { "idle" } else { "working" };
            input(
                AGENT, "harness.observed", Some(AGENT),
                json!({"state": state, "incarnation_id": "inc-1", "observed_at_ms": n}),
                &format!("harness-{n}"),
            )
        } else {
            input(
                "daemon/alder", "daemon.diagnostic", None,
                json!({"severity": "warning", "code": "slow-request", "reason": format!("slow {n}")}),
                &format!("diagnostic-{n}"),
            )
        };
        claims.push(store.append_claim(&claim).unwrap().id);
    }
    store.seal_local_batches().unwrap();
    // Envelope and tombstone writes below come from another connection, as from another
    // process: the triggers fence them all the same. Claims and records carry projection
    // triggers that need the store's own connection, so they change through its writer.
    let other = rusqlite::Connection::open(&path).unwrap();
    other.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
    let epoch = || smallclaims::store::checkpoint_capture_epoch(&other).unwrap();
    let envelope_of = |claim: &str| -> i64 {
        other.query_row(
            "SELECT envelopes.rowid FROM replica_envelopes envelopes JOIN replica_records records
               ON records.writer=envelopes.writer AND records.sequence=envelopes.sequence
                 AND records.envelope_hash=envelopes.envelope_hash
             WHERE records.claim_id=?1",
            [claim], |row| row.get(0),
        ).unwrap()
    };

    // Before any capture registered this cut, the guard does not fence it: no reuse.
    let (first, work) = identities(&store, cut);
    assert_walked(work);
    assert_walked(identities(&store, cut).1);
    register(&store, cut);
    assert_walked(identities(&store, cut).1);
    let (again, work) = identities(&store, cut);
    assert_eq!(again, first);
    assert_reused(work);

    // History after the cut keeps the digest, and the reuse.
    store.set_write_clock_at(cut + 1_000).unwrap();
    let late = store.append_claim(&input(
        AGENT, "harness.observed", Some(AGENT),
        json!({"state": "idle", "incarnation_id": "inc-1", "observed_at_ms": 999}), "after-cut",
    )).unwrap().id;
    store.seal_local_batches().unwrap();
    let (after_cut, work) = identities(&store, cut);
    assert_eq!(after_cut.digest, first.digest);
    assert!(after_cut.seal_rowid > first.seal_rowid);
    assert_reused(work);

    // An envelope above the guard's frontier moved before the cut, with its claims, moves no
    // epoch; the reuse reads those rows' dates itself.
    let unfenced = envelope_of(&late);
    let before = epoch();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h164d197fb0c6 owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE claims SET accepted_at_unix_ms='5000' WHERE id IN (
             SELECT records.claim_id FROM replica_records records JOIN replica_envelopes envelopes
               ON records.writer=envelopes.writer AND records.sequence=envelopes.sequence
                 AND records.envelope_hash=envelopes.envelope_hash
             WHERE envelopes.rowid=?1)",
        [unfenced],
    ).unwrap();
    other.execute(/* sqlite-large-value: allow-for-now lv-hba67ffaf8769 owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET accepted_at_unix_ms='5000' WHERE rowid=?1", [unfenced]).unwrap();
    assert_eq!(epoch(), before, "rows above the frontier are not fenced");
    let (moved, work) = identities(&store, cut);
    assert_ne!(moved.digest, first.digest);
    assert_eq!(moved.count, first.count + 1);
    assert_walked(work);
    register(&store, cut);
    assert_walked(identities(&store, cut).1);
    assert_reused(identities(&store, cut).1);

    // A peer's envelope from before the cut arrives late.
    let birch = Store::open_memory("birch").unwrap();
    birch.set_write_clock_at(2_000).unwrap();
    birch.append_claim(&input(
        "agent/birch.worker", "harness.observed", Some("agent/birch.worker"),
        json!({"state": "working", "incarnation_id": "inc-2"}), "birch-harness",
    )).unwrap();
    let before = epoch();
    sync(&[&store, &birch]);
    assert!(epoch() > before);
    let (added, work) = identities(&store, cut);
    assert!(added.count > moved.count, "{added:?} {moved:?}");
    assert_walked(work);
    // It lies above the frontier, so reads walk until a seal pass registers it.
    assert_walked(identities(&store, cut).1);
    register(&store, cut);
    assert_walked(identities(&store, cut).1);
    assert_reused(identities(&store, cut).1);

    // A claim dated after the cut excludes its envelope; a repair of its record lets it back in.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hb59720c737d2 owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?1 WHERE id=?2",
        params![(cut + 1).to_string(), claims[3]],
    ).unwrap();
    let (excluded, work) = identities(&store, cut);
    assert_eq!(excluded.count, added.count - 1);
    assert_walked(work);
    assert_reused(identities(&store, cut).1);
    record_state(&store, &claims[3], "repaired");
    let (repaired, work) = identities(&store, cut);
    assert_eq!(repaired.digest, added.digest);
    assert_walked(work);
    record_state(&store, &claims[3], "valid");
    let (unrepaired, work) = identities(&store, cut);
    assert_eq!(unrepaired.digest, excluded.digest);
    assert_walked(work);
    assert_reused(identities(&store, cut).1);

    // A trim moves identities from envelopes to tombstones: the same set, read again.
    let plan = plan_drops(&store.checkpoint_sealed_set(cut).unwrap());
    assert!(!plan.envelopes.is_empty());
    assert_reused(identities(&store, cut).1);
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        record_checkpoint_tombstones_tx(&transaction, &checkpoint_name(cut), &plan.envelopes, &plan.claims).unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        transaction.commit().unwrap();
    }
    let (trimmed, work) = identities(&store, cut);
    assert_eq!(trimmed, SealedIdentities { seal_rowid: trimmed.seal_rowid, ..unrepaired.clone() });
    assert_walked(work);
    assert_reused(identities(&store, cut).1);

    // A tombstone removed, and one added, from before the cut.
    other.execute(
        /* sqlite-large-value: allow-for-now lv-hb6cfaca286ba owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_envelopes WHERE rowid=(SELECT MIN(rowid) FROM checkpoint_envelopes)", [],
    ).unwrap();
    let (untombstoned, work) = identities(&store, cut);
    assert_eq!(untombstoned.count, trimmed.count - 1);
    assert_walked(work);
    assert_reused(identities(&store, cut).1);
    tombstone(&other, "cedar");
    let (tombstoned, work) = identities(&store, cut);
    assert_eq!(tombstoned.count, untombstoned.count + 1);
    assert_walked(work);
    assert_reused(identities(&store, cut).1);

    // A restored copy is another store, which starts with nothing to reuse.
    let copy = dir.path().join("restored.sqlite3");
    store.copy_store_to(&copy).unwrap();
    let restored = Store::open(&copy, "alder").unwrap();
    let (read, work) = identities(&restored, cut);
    assert_eq!(read, tombstoned);
    assert_walked(work);

    // A deleted envelope.
    let removed: i64 = other.query_row(
        "SELECT MAX(rowid) FROM replica_envelopes WHERE CAST(accepted_at_unix_ms AS INTEGER) < ?1",
        [i64::try_from(cut).unwrap()], |row| row.get(0),
    ).unwrap();
    other.execute(/* sqlite-large-value: allow-for-now lv-h18680c8a2948 owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes WHERE rowid=?1", [removed]).unwrap();
    let (deleted, work) = identities(&store, cut);
    assert_eq!(deleted.count, tombstoned.count - 1);
    assert_walked(work);
    assert_reused(identities(&store, cut).1);
}

/// A walk that a fenced change interrupts is returned as it was read, but not kept: the next read
/// walks again and sees the change.
#[test]
fn a_walk_a_fenced_change_interrupts_is_not_reused() {
    use std::cell::Cell;
    struct Mutate {
        connection: rusqlite::Connection,
        done: Cell<bool>,
    }
    unsafe extern "C" fn tombstone_mid_walk(
        _: std::ffi::c_uint,
        context: *mut std::ffi::c_void,
        statement: *mut std::ffi::c_void,
        _: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        let mutation = unsafe { &*context.cast::<Mutate>() };
        let sql = unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_sql(statement.cast())) };
        if !mutation.done.get() && sql.to_bytes() == SEALED_IDENTITY_TOMBSTONES_SQL.as_bytes() {
            mutation.done.set(true);
            // Committed while the first page's snapshot is open, at a key before every key the
            // walk reads later, so this walk never sees it.
            tombstone(&mutation.connection, "aaa");
        }
        0
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "alder").unwrap();
    let cut: u128 = 1_000_000;
    for n in 0..100_u128 {
        store.set_write_clock_at(1_000 + n).unwrap();
        store.append_claim(&input(
            "daemon/alder", "daemon.diagnostic", None,
            json!({"severity": "warning", "code": "slow-request", "reason": format!("slow {n}")}),
            &format!("diagnostic-{n}"),
        )).unwrap();
    }
    store.seal_local_batches().unwrap();
    register(&store, cut);
    let (before, _) = identities(&store, cut);
    assert_reused(identities(&store, cut).1);
    let other = rusqlite::Connection::open(&path).unwrap();
    other.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
    // A fenced change first, so the next read walks.
    tombstone(&other, "zzz");
    let mutation = Mutate { connection: other, done: Cell::new(false) };
    // SAFETY: the handles stay valid while the store holds its connections, past every use here.
    let idle = store.readers.idle.lock().unwrap().iter()
        .map(|connection| unsafe { connection.handle() })
        .collect::<Vec<_>>();
    assert!(!idle.is_empty());
    for handle in &idle {
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                *handle, rusqlite::ffi::SQLITE_TRACE_STMT as u32,
                Some(tombstone_mid_walk), (&raw const mutation).cast_mut().cast(),
            );
        }
    }
    let torn = store.checkpoint_sealed_identities(cut, None);
    for handle in &idle {
        unsafe { rusqlite::ffi::sqlite3_trace_v2(*handle, 0, None, std::ptr::null_mut()) };
    }
    assert!(mutation.done.get(), "the walk read a tombstone page");
    let torn = torn.unwrap();
    assert_eq!(torn.count, before.count + 1, "the walk read the earlier tombstone, not the later one");
    let read = store.checkpoint_sealed_identities(cut, None).unwrap();
    let walked = store.checkpoint_sealed_identities_paged(cut, None, 64).unwrap();
    assert_eq!(read, walked);
    assert_eq!(read.count, before.count + 2);
}

/// A tombstone of `writer`'s first envelope, dated before every cut these tests use.
fn tombstone(connection: &rusqlite::Connection, writer: &str) {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8a572700717c owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
         VALUES (?1, 1, ?2, 3000, 'checkpoint/1970-01-01')",
        params![writer, format!("hash-{writer}-1")],
    ).unwrap();
}

fn record_state(store: &Store, claim: &str, state: &str) {
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h53f8f1472a60 owner=fractal_store::store::checkpoint_identity_reuse_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state=?1 WHERE claim_id=?2", params![state, claim],
    ).unwrap();
}

/// Register `cut` and the newest envelope with the capture guard, as a seal pass does.
fn register(store: &Store, cut: u128) {
    store.checkpoint_sealed_set(cut).unwrap();
}

/// The digest a seal reads and its work, checked against a fresh walk at two page sizes.
fn identities(store: &Store, cut: u128) -> (SealedIdentities, SqliteWork) {
    let scope = SqliteWorkScope::start();
    let read = store.checkpoint_sealed_identities(cut, None).unwrap();
    let work = scope.finish();
    for page in [64, 7] {
        assert_eq!(read, store.checkpoint_sealed_identities_paged(cut, None, page).unwrap(), "page {page}");
    }
    (read, work)
}

fn assert_reused(work: SqliteWork) {
    assert!(work.statements <= REUSE_STATEMENTS && work.vm_steps <= REUSE_VM_STEPS, "not reused: {work:?}");
}

fn assert_walked(work: SqliteWork) {
    // A walk of this fixture's 150 envelopes is three pages and several thousand VM steps.
    assert!(work.statements > REUSE_STATEMENTS && work.vm_steps > REUSE_VM_STEPS * 5, "not walked: {work:?}");
}
