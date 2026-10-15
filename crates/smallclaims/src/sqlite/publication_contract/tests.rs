//! Authored native controls for the private contract. These have NOT been compiled or run.
//! Fixture mutation tokens are maintained by fixture-only triggers, not a proposed Store schema.
use super::*;
use crate::sqlite::WriterConnection;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}, mpsc};
use std::time::Duration;

struct Fixture {
    directory: tempfile::TempDir,
    writer: Arc<WriterConnection>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join("source.sqlite3")).unwrap();
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT);
             CREATE TABLE authority(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO authority VALUES(1,'allowed');
             CREATE TABLE tombstones(id INTEGER PRIMARY KEY);
             CREATE TABLE epochs(id TEXT PRIMARY KEY, value INTEGER NOT NULL);
             INSERT INTO epochs VALUES('authority',0),('tombstones',0);
             CREATE TRIGGER authority_insert AFTER INSERT ON authority BEGIN
               UPDATE epochs SET value=value+1 WHERE id='authority'; END;
             CREATE TRIGGER authority_update AFTER UPDATE ON authority BEGIN
               UPDATE epochs SET value=value+1 WHERE id='authority'; END;
             CREATE TRIGGER authority_delete AFTER DELETE ON authority BEGIN
               UPDATE epochs SET value=value+1 WHERE id='authority'; END;
             CREATE TRIGGER tombstones_insert AFTER INSERT ON tombstones BEGIN
               UPDATE epochs SET value=value+1 WHERE id='tombstones'; END;
             CREATE TRIGGER tombstones_delete AFTER DELETE ON tombstones BEGIN
               UPDATE epochs SET value=value+1 WHERE id='tombstones'; END;
             CREATE TABLE output(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE parents(id INTEGER PRIMARY KEY);
             CREATE TABLE deferred_child(id INTEGER REFERENCES parents(id)
               DEFERRABLE INITIALLY DEFERRED);"
        ).unwrap();
        let writer = Arc::new(WriterConnection::new(connection, Arc::new(AtomicU64::new(0))));
        Self { directory, writer }
    }

    fn reader(&self) -> Connection {
        Connection::open(self.directory.path().join("source.sqlite3")).unwrap()
    }

    /// Capture on an independent consistent reader and finish that snapshot before publication.
    fn prepare(&self) -> PreparedPublication<&'static str> {
        let mut reader = self.reader();
        let snapshot = reader.transaction().unwrap();
        let reads = observations(&snapshot);
        snapshot.commit().unwrap();
        PreparedPublication { payload: "prepared result", reads }
    }

    fn output_count(&self) -> u64 {
        self.reader().query_row("SELECT count(*) FROM output", [], |r| r.get(0)).unwrap()
    }
}

fn key(name: &str) -> PredicateKey {
    PredicateKey { predicate: name.to_owned(), key: 1_i64.to_be_bytes().to_vec() }
}

fn identity() -> SourceIdentity {
    SourceIdentity { lifetime: [1; 32], schema: [2; 32], rules: [3; 32], producer: [4; 32] }
}

fn token(connection: &Connection, table: &str) -> MutationToken {
    let epoch: u64 = connection.query_row(
        "SELECT value FROM epochs WHERE id=?1", [table], |r| r.get(0)
    ).unwrap();
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&epoch.to_be_bytes());
    MutationToken(bytes)
}

fn observations(connection: &Connection) -> ReadSet {
    let authority: Option<String> = connection.query_row(
        "SELECT value FROM authority WHERE id=1", [], |r| r.get(0)
    ).optional().unwrap();
    let positive = match authority {
        Some(value) => Fact::Present {
            mutation: Some(token(connection, "authority")),
            value: Sha256::digest(value.as_bytes()).into(),
        },
        None => Fact::Absent { mutation: Some(token(connection, "authority")) },
    };
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM tombstones WHERE id=1)", [], |r| r.get(0)
    ).unwrap();
    let negative = if present {
        Fact::Present { mutation: Some(token(connection, "tombstones")), value: [1; 32] }
    } else {
        Fact::Absent { mutation: Some(token(connection, "tombstones")) }
    };
    ReadSet::capture(identity(), BTreeSet::from([key("authority"), key("tombstones")]),
        [(key("authority"), positive), (key("tombstones"), negative)]).unwrap()
}

fn publish_fixture(fixture: &Fixture, prepared: PreparedPublication<&str>) -> anyhow::Result<()> {
    // This fixed fixture command is not a generic production SQL callback/cost certificate.
    let mut guard = fixture.writer.write();
    let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
    prepared.reads.validate(&observations(&tx))
        .map_err(|error| anyhow::anyhow!("stale before apply: {error:?}"))?;
    tx.execute("INSERT INTO output VALUES(1,?1)", [prepared.payload])?;
    tx.commit_validated(|tx| {
        prepared.reads.validate(&observations(tx))
            .map_err(|error| anyhow::anyhow!("stale after finalizers: {error:?}"))
    })?;
    drop(guard); // Native authority callbacks still run here. No model return proof minted.
    Ok(())
}

#[test]
fn matching_fixture_publishes_but_does_not_mint_native_completion() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    assert_eq!(prepared.certification(),
        Err(UnprovidedContract::TransactionalAllMutatorEvidence));
    let outcome = UncertifiedCompletion::after_managed_result(publish_fixture(&fixture, prepared));
    assert_eq!(fixture.output_count(), 1);
    assert!(matches!(&outcome.commit, CommitKnowledge::Committed));
    assert_eq!(outcome.commit.acknowledgement(), Err(CompletionGap::AuthorityUnobserved));
    assert!(outcome.missing.contains(&NativeObligation::PhysicalQueueReturn));
}

#[test]
fn same_index_authority_revocation_refuses_without_publishing() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.writer.write().execute("UPDATE authority SET value='revoked' WHERE id=1", []).unwrap();
    assert_eq!(fixture.writer.committed_index.load(Ordering::Acquire), 0);
    let error = publish_fixture(&fixture, prepared).unwrap_err();
    assert!(error.to_string().contains("stale before apply"));
    assert_eq!(fixture.output_count(), 0);
}

#[test]
fn negative_insert_and_delete_reinsert_aba_refuse_at_unchanged_claim_index() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.writer.write().execute("INSERT INTO tombstones VALUES(1)", []).unwrap();
    assert!(publish_fixture(&fixture, prepared).is_err());
    fixture.writer.write().execute("DELETE FROM tombstones", []).unwrap();
    let prepared = fixture.prepare();
    // Restore exactly the same visible authority value and key, but a different mutation.
    fixture.writer.write().execute_batch(
        "DELETE FROM authority WHERE id=1; INSERT INTO authority VALUES(1,'allowed');"
    ).unwrap();
    assert!(publish_fixture(&fixture, prepared).is_err());
    assert_eq!(fixture.output_count(), 0);
    assert_eq!(fixture.writer.committed_index.load(Ordering::Acquire), 0);
}

#[test]
fn finalizer_invalidation_rolls_back_its_own_change_and_prepared_output() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.writer.install_transaction_finalizer(|tx| {
        tx.execute("UPDATE authority SET value='revoked' WHERE id=1", [])?;
        Ok(())
    }).unwrap();
    let resolved = Arc::new(Mutex::new(Vec::<(String, u64)>::new()));
    let seen = resolved.clone();
    let observer = fixture.writer.observe_commits(move |connection| {
        let current = connection.query_row(
            "SELECT authority.value, epochs.value FROM authority, epochs
             WHERE authority.id=1 AND epochs.id='authority'", [],
            |row| Ok((row.get(0)?, row.get(1)?))
        ).unwrap();
        seen.lock().unwrap().push(current);
    });
    let error = publish_fixture(&fixture, prepared).unwrap_err();
    drop(observer);
    assert!(error.to_string().contains("stale after finalizers"));
    assert_eq!(fixture.output_count(), 0);
    let value: String = fixture.reader().query_row(
        "SELECT value FROM authority WHERE id=1", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(value, "allowed");
    assert_eq!(*resolved.lock().unwrap(), vec![("allowed".to_owned(), 0)]);
    // Provisional source/evidence rolled back together, then the native observer read old
    // resolved authority. The committed-authority callback never ran on provisional rows.
}

#[test]
fn finalizer_failure_is_not_mistaken_for_a_commit_receipt() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.writer.install_transaction_finalizer(|tx| {
        tx.execute("INSERT INTO tombstones VALUES(1)", [])?;
        anyhow::bail!("fixture finalizer refused")
    }).unwrap();
    let outcome = UncertifiedCompletion::after_managed_result(publish_fixture(&fixture, prepared));
    assert!(matches!(&outcome.commit, CommitKnowledge::Unknown(_)));
    assert_eq!(outcome.commit.acknowledgement(), Err(CompletionGap::NotCommitted));
    assert_eq!(fixture.output_count(), 0);
    let tombstones: u64 = fixture.reader().query_row(
        "SELECT count(*) FROM tombstones", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(tombstones, 0);
}

#[test]
fn sqlite_commit_error_stays_unknown_instead_of_authorizing_retry() {
    let fixture = Fixture::new();
    let mut guard = fixture.writer.write();
    let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
    tx.execute("INSERT INTO output VALUES(1,'not durable')", []).unwrap();
    tx.execute("INSERT INTO deferred_child VALUES(99)", []).unwrap();
    let outcome = UncertifiedCompletion::after_managed_result(tx.commit());
    drop(guard);
    assert!(matches!(&outcome.commit, CommitKnowledge::Unknown(_)));
    assert_eq!(outcome.commit.acknowledgement(), Err(CompletionGap::NotCommitted));
    assert_eq!(fixture.output_count(), 0);
    // This fixture's failed deferred-FK COMMIT rolled back. Generic managed Result cannot
    // establish that for other SQLite/transport failures and supplies no retry/reapply API.
}

fn observer_gate_holds(batch: bool) {
    let fixture = Fixture::new();
    let (entered, observing) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let released = Mutex::new(released);
    let first = AtomicBool::new(true);
    let observer = fixture.writer.observe_commits(move |connection| {
        let count: u64 = connection.query_row("SELECT count(*) FROM output", [], |r| r.get(0)).unwrap();
        if count == 1 && first.swap(false, Ordering::AcqRel) {
            entered.send(()).unwrap();
            released.lock().unwrap().recv().unwrap();
        }
    });
    let writer = fixture.writer.clone();
    let (ack, acknowledged) = mpsc::sync_channel(1);
    let publishing = std::thread::spawn(move || {
        let result = if batch {
            writer.batched(|tx| tx.execute("INSERT INTO output VALUES(1,'durable')", []))
                .map_err(anyhow::Error::msg)
                .and_then(|result| result.map(|_| ()).map_err(Into::into))
        } else {
            let mut guard = writer.write();
            let tx = guard.transaction().unwrap();
            tx.execute("INSERT INTO output VALUES(1,'durable')", []).unwrap();
            let committed = tx.commit();
            drop(guard); // The test stalls in the real guard-return observer.
            committed
        };
        ack.send(result).unwrap();
    });
    let started = observing.recv_timeout(Duration::from_secs(5));
    let before_ack = acknowledged.try_recv();
    let writer = fixture.writer.clone();
    let (acquired, next_loan) = mpsc::sync_channel(1);
    let (trying, attempted) = mpsc::sync_channel(1);
    let next = std::thread::spawn(move || {
        trying.send(()).unwrap();
        let guard = writer.write();
        acquired.send(()).unwrap(); // Actual connection receipt, not a copied state flag.
        drop(guard);
    });
    let next_started = attempted.recv_timeout(Duration::from_secs(5));
    let before_return = next_loan.try_recv();
    // Release BEFORE unregistering: handle removal waits for an in-flight callback.
    release.send(()).unwrap();
    drop(observer);
    publishing.join().unwrap();
    next.join().unwrap();
    assert!(started.is_ok());
    assert!(next_started.is_ok());
    assert!(matches!(before_ack, Err(mpsc::TryRecvError::Empty)));
    assert!(matches!(before_return, Err(mpsc::TryRecvError::Empty)));
    acknowledged.recv().unwrap().unwrap();
    next_loan.recv().unwrap();
    let outcome = UncertifiedCompletion::after_managed_result(Ok(()));
    assert_eq!(outcome.commit.acknowledgement(), Err(CompletionGap::AuthorityUnobserved));
    assert!(outcome.missing.contains(&NativeObligation::PhysicalQueueReturn));
}

#[test]
fn batch_ack_waits_for_authority_and_next_dispatch_requires_actual_connection() {
    observer_gate_holds(true);
}

#[test]
fn lent_ack_waits_for_guard_observer_and_next_loan_requires_actual_return() {
    observer_gate_holds(false);
}

#[test]
fn observer_panic_after_commit_does_not_turn_durable_output_into_rollback_or_ack() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let observer = fixture.writer.observe_commits(|_| panic!("fixture authority failure"));
    let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        publish_fixture(&fixture, prepared)
    }));
    assert!(attempted.is_err());
    drop(observer);
    assert_eq!(fixture.output_count(), 1);
    // Caller saw no successful return. Recovery must inspect authoritative committed state;
    // no generic Result-to-rollback or idempotent reapply protocol is invented here.
}

#[test]
fn raw_capture_gaps_do_not_become_certified_by_matching_owned_facts() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    // Explicit bypass defeats fixture capture: same bytes/key/index and restored epoch.
    // A comparer cannot discover omitted writes. There is deliberately no certificate/ACK.
    fixture.writer.write().execute_batch(
        "DROP TRIGGER authority_delete; DROP TRIGGER authority_insert;
         DELETE FROM authority; INSERT INTO authority VALUES(1,'allowed');"
    ).unwrap();
    let reader = fixture.reader();
    assert!(prepared.reads.validate(&observations(&reader)).is_ok());
    assert_eq!(prepared.certification(),
        Err(UnprovidedContract::TransactionalAllMutatorEvidence));
    let outcome = UncertifiedCompletion::after_managed_result(Ok(()));
    assert!(outcome.commit.acknowledgement().is_err());
    assert_eq!(outcome.missing.len(), 3);
    // The fixture identity wasn't maintained across DDL: source-coverage certification would
    // have to reject this adapter. No generic bool claims that it was complete.
}

#[test]
fn missing_negative_mutation_or_predicate_is_refused_before_submission() {
    let required = BTreeSet::from([key("tombstones")]);
    assert!(matches!(ReadSet::capture(identity(), required.clone(), [
        (key("tombstones"), Fact::Absent { mutation: None })
    ]), Err(Refusal::MissingMutation(_))));
    assert!(matches!(ReadSet::capture(identity(), required, []),
        Err(Refusal::MissingPredicate(_))));
}
