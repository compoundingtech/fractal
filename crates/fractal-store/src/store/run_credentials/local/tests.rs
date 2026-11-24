use super::*;
use crate::store::run_credentials::RunBinding;

fn binding(attempt: u32, epoch: u64) -> ExecutionBinding {
    ExecutionBinding {
        version: 1, home: "home".into(), host: "execution-host".into(), epoch,
        subject: "step-run/example/work".into(), runtime: "program-step/example".into(),
        incarnation: format!("host-generation:{attempt}"),
        authorization_revision: format!("epoch-{epoch}-attempt-{attempt}"),
        run: Some(RunBinding { run: "mission-run/example".into(), step_run: "step-run/example/work".into(), attempt }),
    }
}
fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    initialize_local_authority(&connection).unwrap();
    connection
}
fn admit(connection: &mut Connection, binding: &ExecutionBinding) -> String {
    let tx = connection.transaction().unwrap();
    admit_execution_tx(&tx, binding).unwrap();
    assert_eq!(issue_local_credential_tx(&tx, binding, 60_000, 1_000).unwrap_err().code, "local-authority-backpressure");
    tx.commit().unwrap();
    let through = pending_changes(connection, binding.epoch).unwrap().last().unwrap().sequence;
    ack(connection, binding.epoch, through);
    let tx = connection.transaction().unwrap();
    let secret = issue_local_credential_tx(&tx, binding, 60_000, 1_000).unwrap();
    tx.commit().unwrap();
    secret
}
fn offer(connection: &mut Connection, binding: &ExecutionBinding) -> bool {
    let tx = connection.transaction().unwrap();
    let active = offer_execution_tx(&tx, binding).unwrap();
    tx.commit().unwrap();
    active
}
fn ack(connection: &mut Connection, epoch: u64, through: u64) {
    let tx = connection.transaction().unwrap();
    acknowledge_changes_tx(&tx, epoch, through).unwrap();
    tx.commit().unwrap();
}

#[test]
fn credentials_resolve_offline_after_reopening_only_the_local_journal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("journal.sqlite");
    let mut local = Connection::open(&path).unwrap();
    initialize_local_authority(&local).unwrap();
    let binding = binding(1, 1);
    let secret = admit(&mut local, &binding);
    drop(local);
    let reopened = Connection::open(&path).unwrap();
    initialize_local_authority(&reopened).unwrap();
    assert_eq!(local_binding(&reopened, &secret, 1_001).unwrap(), binding);
    assert!(local_binding(&reopened, &secret, 61_000).is_err());
    assert!(pending_changes(&reopened, 1).unwrap().is_empty());
    assert_eq!(acknowledged_change_high_water(&reopened, 1).unwrap(), 1);
}

#[test]
fn predecessor_revocation_commits_before_successor_and_home_ack_does_not_auto_admit() {
    let mut local = connection();
    let first = binding(1, 1);
    let secret = admit(&mut local, &first);
    let second = binding(2, 1);
    assert!(!offer(&mut local, &second));
    assert!(local_binding(&local, &secret, 1_001).is_err());
    assert_eq!(require_execution(&local, &second).unwrap_err().code, "local-authority-backpressure");
    let changes = pending_changes(&local, 1).unwrap();
    assert_eq!(changes.iter().map(|change| (change.sequence, change.revoked)).collect::<Vec<_>>(), [(2, true)]);
    let writes = local.total_changes();
    assert!(!offer(&mut local, &second));
    assert_eq!(local.total_changes(), writes, "duplicate pending offer does not append or notify");
    ack(&mut local, 1, 2);
    assert!(active_bindings(&local).unwrap().is_empty());
    assert!(require_execution(&local, &second).is_err(), "ACK alone cannot validate current native custody");
    assert!(offer(&mut local, &second));
    let writes = local.total_changes();
    assert!(offer(&mut local, &second));
    assert_eq!(local.total_changes(), writes);
    assert!(local_binding(&local, &secret, 1_001).is_err());
    assert_eq!(pending_changes(&local, 1).unwrap()[0].sequence, 3);
    let tx = local.transaction().unwrap();
    assert!(admit_execution_tx(&tx, &first).is_err(), "a revoked revision never resurrects");
}

#[test]
fn a_newer_native_proposal_tombstones_a_stale_disconnected_adoption() {
    let mut local = connection();
    admit(&mut local, &binding(1, 1));
    assert!(!offer(&mut local, &binding(2, 1)));
    assert!(!offer(&mut local, &binding(3, 1)));
    ack(&mut local, 1, 2);
    let tx = local.transaction().unwrap();
    assert!(offer_execution_tx(&tx, &binding(2, 1)).is_err());
    tx.rollback().unwrap();
    assert!(offer(&mut local, &binding(3, 1)));
}

#[test]
fn epoch_revocation_includes_compacted_commands_and_change_sequences_restart_per_epoch() {
    let mut local = connection();
    let secret = admit(&mut local, &binding(1, 1));
    ack(&mut local, 1, 1);
    assert!(pending_changes(&local, 1).unwrap().is_empty());
    let tx = local.transaction().unwrap();
    revoke_epoch_tx(&tx, "execution-host", 1).unwrap();
    tx.commit().unwrap();
    assert!(local_binding(&local, &secret, 1_001).is_err());
    assert_eq!(pending_changes(&local, 1).unwrap()[0].sequence, 2);
    ack(&mut local, 1, 2);
    assert!(offer(&mut local, &binding(2, 2)));
    assert_eq!(pending_changes(&local, 2).unwrap()[0].sequence, 1);
    assert_eq!(acknowledged_change_high_water(&local, 1).unwrap(), 2);
    assert_eq!(acknowledged_change_high_water(&local, 2).unwrap(), 0);
    let tx = local.transaction().unwrap();
    assert!(acknowledge_changes_tx(&tx, 2, 2).is_err(), "a future ACK cannot erase an unissued change");
}

#[test]
fn nonprogram_admission_keeps_native_identity_without_inventing_a_run() {
    let mut local = connection();
    let mut native = binding(1, 1);
    native.run = None;
    native.subject = "agent/example".into();
    assert!(offer(&mut local, &native));
    assert_eq!(active_bindings(&local).unwrap(), [native.clone()]);
    require_execution(&local, &native).unwrap();
    let mut foreign = native;
    foreign.incarnation = "another-native-incarnation".into();
    assert!(require_execution(&local, &foreign).is_err());
}

#[test]
fn unexpired_credential_hashes_are_bounded_and_actual_expiry_reclaims_them() {
    let mut local = connection();
    let execution = binding(1, 1);
    admit(&mut local, &execution);
    local.execute(
        /* sqlite-large-value: allow-for-now lv-h1c35b12b5b69 owner=fractal_store::store::run_credentials::local::tests TODO=bound-or-reference-values-and-reclamation */ "WITH RECURSIVE numbers(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM numbers WHERE value<?1)
         INSERT INTO local_execution_credentials_v1(secret_hash, host, revision, expires_at_unix_ms)
         SELECT printf('%064x',value), ?2, ?3, 10000 FROM numbers",
        params![MAX_CREDENTIALS - 1, execution.host, execution.authorization_revision],
    ).unwrap();
    let tx = local.transaction().unwrap();
    assert_eq!(issue_local_credential_tx(&tx, &execution, 1000, 1001).unwrap_err().code, "local-authority-backpressure");
    tx.rollback().unwrap();
    let tx = local.transaction().unwrap();
    let secret = issue_local_credential_tx(&tx, &execution, 1000, 61000).unwrap();
    tx.commit().unwrap();
    assert_eq!(local_binding(&local, &secret, 61001).unwrap(), execution);
    let count: usize = local.query_row("SELECT COUNT(*) FROM local_execution_credentials_v1", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn the_real_program_lifetime_is_not_truncated_to_sqlite_integer_range() {
    let mut local = connection();
    let execution = binding(1, 1);
    admit(&mut local, &execution);
    let tx = local.transaction().unwrap();
    let secret = issue_local_credential_tx(&tx, &execution, u64::MAX, 1_000).unwrap();
    tx.commit().unwrap();
    assert_eq!(local_binding(&local, &secret, u64::MAX).unwrap(), execution);
    assert!(pending_changes(&local, 1).unwrap().is_empty(), "acknowledged authority can remint offline");
    let tx = local.transaction().unwrap();
    revoke_execution_tx(&tx, &execution.host, &execution.authorization_revision).unwrap();
    tx.commit().unwrap();
    assert!(local_binding(&local, &secret, 1_001).is_err());
}
