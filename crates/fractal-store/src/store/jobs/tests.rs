use super::*;
use serde_json::json;

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE claims(id TEXT PRIMARY KEY, store_index INTEGER,
        batch_id TEXT, subject TEXT, kind TEXT, body TEXT, accepted_at_unix_ms TEXT);
        CREATE TABLE batches(id TEXT PRIMARY KEY, origin TEXT, replica_sequence INTEGER);
        CREATE TABLE replica_records(claim_id TEXT, state TEXT, position INTEGER);
        CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER);
        INSERT INTO batches VALUES('batch','fixture-host',1);").unwrap();
    create_schema(&connection).unwrap();
    connection
}

pub(super) fn submission(id: &str, key: &str) -> JobRecord {
    let request: ExecutionRequest = serde_json::from_value(json!({
        "source":{"kind":"content","sha256":vec![0; 32]},
        "environment":{"kind":"exact","value":{}},
        "command":{"argv":["fixture"],"cwd":"."},
        "resources":{"cpu_millicores":1,"memory_bytes":1,"io_weight":1,
            "tasks_max":1,"expected_duration_ms":1,"run_timeout_ms":null,
            "priority":0,"requested_class":"fixture","storage_request":null,"delegated_reservations":null}
    })).unwrap();
    JobRecord::Submitted {
        binding: JobHostBinding { job_id: id.into(), execution_host: "fixture-host".into(),
            authority_declaration: "claim/fixture-member".into() },
        identity: IdempotencyKey { requester: "person/fixture".into(), key: key.into(),
            request_digest: request.digest() }, request,
    }
}

fn insert(tx: &Transaction<'_>, subject: &str, claim: &str, time: u64, record: &JobRecord) {
    tx.execute("INSERT INTO claims VALUES(?1,?2,'batch',?3,?4,?5,?6)",
        params![claim, time, subject, KIND, json!({"fields":{"record":record}}).to_string(), time.to_string()]).unwrap();
    tx.execute("INSERT INTO claim_positions VALUES(?1,?2)", params![claim, time]).unwrap();
}

#[test]
fn empty_rebuild_and_graph_replay_are_noops_on_non_job_logs() {
    let mut connection = database();
    connection.execute_batch("INSERT INTO claims
        SELECT 'unrelated-'||value,value,'batch','person/fixture','attention.requested','{}','1'
        FROM json_each('[1,2,3,4,5]');").unwrap();
    let before = connection.total_changes();
    let tx = connection.transaction().unwrap();
    rebuild(&tx).unwrap();
    let stage = crate::store::projectors::GRAPH_REPLAY.iter()
        .find(|stage| stage.phase == "full-replay/jobs").unwrap();
    (stage.run)(&tx).unwrap();
    assert_eq!(tx.total_changes(), before);
    let plan: String = tx.query_row("EXPLAIN QUERY PLAN SELECT DISTINCT subject FROM claims
        INDEXED BY jobs_claim_subject WHERE kind='job.recorded'", [], |row| row.get(3)).unwrap();
    assert!(plan.contains("jobs_claim_subject"), "{plan}");
}

#[test]
fn unrelated_replication_does_not_even_require_projection_tables() {
    let mut connection = Connection::open_in_memory().unwrap();
    let tx = connection.transaction().unwrap();
    let claim = ClaimRecord { id: "unrelated".into(), store_index: 1, batch_id: "batch".into(),
        subject: "person/fixture".into(), kind: "attention.requested".into(), origin: "fixture".into(),
        actor: None, operation_id: None, request_digest: None, body: json!({}),
        predecessors: vec![], accepted_at_unix_ms: 1 };
    project(&tx, &claim).unwrap();
    assert_eq!(tx.total_changes(), 0);
}

#[test]
fn canonical_refold_refuses_illegal_edges_and_conflicting_binding() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    let id = job_id("person/fixture", "fixture");
    let submitted = submission(id.as_str(), "fixture");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    insert(&tx, id.as_str(), "later", 3, &JobRecord::Transition {
        binding: binding.clone(), event: Event::Started });
    insert(&tx, id.as_str(), "first", 1, &submitted);
    insert(&tx, id.as_str(), "middle", 2, &JobRecord::Transition {
        binding: binding.clone(), event: Event::LaunchIntent });
    flush(&tx).unwrap();
    let state: String = tx.query_row("SELECT state FROM jobs", [], |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JobState>(&state).unwrap(), JobState::Running);
    let mut conflict = binding.clone();
    conflict.execution_host = "different-host".into();
    insert(&tx, id.as_str(), "conflict", 4, &JobRecord::Transition { binding: conflict, event: Event::HostLost });
    insert(&tx, id.as_str(), "illegal", 5, &JobRecord::Transition { binding: binding.clone(), event: Event::Prepared });
    flush(&tx).unwrap();
    let refusals: u64 = tx.query_row("SELECT count(*) FROM job_history WHERE refusal IS NOT NULL", [], |row| row.get(0)).unwrap();
    assert_eq!(refusals, 2);
    assert_eq!(tx.query_row("SELECT state FROM jobs", [], |row| row.get::<_, String>(0)).unwrap(), state);
}

#[test]
fn oversized_subject_refuses_before_mutating_projection() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    for index in 0..=REFOLD_LIMIT {
        tx.execute("INSERT INTO claims VALUES(?1,?2,'batch','job/large',?3,'{}','1')",
            params![format!("claim-{index}"), index, KIND]).unwrap();
    }
    let before = tx.total_changes();
    assert!(refold(&tx, "job/large").unwrap_err().to_string().contains("job-refold-limit"));
    assert_eq!(tx.total_changes(), before);
}

#[test]
fn receipts_are_atomic_isolated_and_refuse_changed_digest() {
    let mut connection = database();
    let id = job_id("person/fixture", "receipt");
    let JobRecord::Submitted { identity, .. } = submission(&id, "receipt") else { unreachable!() };
    {
        let tx = connection.transaction().unwrap();
        receipts::reserve_receipt(&tx, &identity, &id).unwrap();
        receipts::reserve_receipt(&tx, &identity, &id).unwrap();
        let mut changed = identity.clone();
        changed.request_digest.0[0] ^= 1;
        assert!(receipts::reserve_receipt(&tx, &changed, &id).unwrap_err().to_string()
            .contains("idempotency-digest-mismatch"));
        let mut other = identity.clone();
        other.requester = "person/other".into();
        let other_id = job_id(&other.requester, &other.key);
        receipts::reserve_receipt(&tx, &other, &other_id).unwrap();
        assert_ne!(id, other_id);
        assert_eq!(receipt_for_key(&tx, &identity.requester, &identity.key).unwrap(),
            Some((identity.request_digest, id.clone())));
        // Rollback removes both reservations, just as a refused writer transaction does.
    }
    let tx = connection.transaction().unwrap();
    assert!(receipt_for_key(&tx, &identity.requester, &identity.key).unwrap().is_none());
}

#[test]
fn additive_migration_and_replay_match_reverse_arrival_history() {
    let mut connection = database();
    create_schema(&connection).unwrap();
    let tx = connection.transaction().unwrap();
    let first_id = job_id("person/fixture", "first");
    let second_id = job_id("person/fixture", "second");
    insert(&tx, &second_id, "second", 20, &submission(&second_id, "second"));
    insert(&tx, &first_id, "first", 10, &submission(&first_id, "first"));
    open(&tx).unwrap();
    let queue = queue_for_host(&tx, "fixture-host").unwrap();
    assert_eq!(queue.iter().map(|job| &job.id).collect::<Vec<_>>(), [&first_id, &second_id]);
    assert!(queue_for_host(&tx, "different-host").unwrap().is_empty());
    let before = serde_json::to_value(history_by_job(&tx, &first_id).unwrap()).unwrap();
    let receipt = receipt_for_key(&tx, "person/fixture", "first").unwrap();
    rebuild(&tx).unwrap();
    assert_eq!(serde_json::to_value(history_by_job(&tx, &first_id).unwrap()).unwrap(), before);
    assert_eq!(receipt_for_key(&tx, "person/fixture", "first").unwrap(), receipt);
    assert_eq!(job_by_id(&tx, &first_id).unwrap().unwrap().state, JobState::Queued);
    assert!(job_by_id(&tx, "job/absent").unwrap().is_none());
}

#[test]
fn canonical_receipt_wins_independently_of_arrival_order() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    let id = job_id("person/fixture", "repeat");
    let original = submission(&id, "repeat");
    let mut changed = original.clone();
    let JobRecord::Submitted { identity, request, .. } = &mut changed else { unreachable!() };
    request.command.argv.push("changed-input".into());
    identity.request_digest = request.digest();
    insert(&tx, &id, "later-changed", 20, &changed);
    flush(&tx).unwrap();
    insert(&tx, &id, "earlier-original", 10, &original);
    flush(&tx).unwrap();
    let JobRecord::Submitted { identity, .. } = &original else { unreachable!() };
    assert_eq!(receipt_for_key(&tx, &identity.requester, &identity.key).unwrap(),
        Some((identity.request_digest, id.clone())));
    let history = history_by_job(&tx, &id).unwrap();
    assert!(history[0].refusal.is_none());
    assert!(history[1].refusal.as_ref().unwrap().contains("idempotency-digest-mismatch"));
    let before = serde_json::to_value(&history).unwrap();
    rebuild(&tx).unwrap();
    assert_eq!(serde_json::to_value(history_by_job(&tx, &id).unwrap()).unwrap(), before);
}

#[test]
fn malformed_records_are_refused_raw_history_without_poisoning_open() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    fractal_schema::registry().validate_claim("job/malformed", KIND,
        &std::collections::BTreeMap::from([("record".into(), json!({}))])).unwrap();
    for (claim, body) in [("wrapped", json!({"fields":{"record":{}}})), ("direct", json!({"record":{}}))] {
        tx.execute("INSERT INTO claims VALUES(?1,1,'batch','job/malformed',?2,?3,'1')",
            params![claim, KIND, body.to_string()]).unwrap();
    }
    flush(&tx).unwrap();
    for _ in 0..2 {
        open(&tx).unwrap();
        rebuild(&tx).unwrap();
        let history = history_by_job(&tx, "job/malformed").unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|row| row.record == json!({})
            && row.refusal.as_deref().unwrap().starts_with("job-record-decode:")));
        assert!(job_by_id(&tx, "job/malformed").unwrap().is_none());
    }
}

#[test]
fn direct_and_enveloped_bodies_share_the_same_fold() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    let id = job_id("person/fixture", "direct");
    let submitted = submission(&id, "direct");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    insert(&tx, &id, "submit", 1, &submitted);
    tx.execute("UPDATE claims SET body=?1 WHERE id='submit'",
        [json!({"record":submitted}).to_string()]).unwrap();
    insert(&tx, &id, "launch", 2, &JobRecord::Transition {
        binding: binding.clone(), event: Event::LaunchIntent });
    let started = JobRecord::Transition { binding: binding.clone(), event: Event::Started };
    insert(&tx, &id, "start", 3, &started);
    tx.execute("UPDATE claims SET body=?1 WHERE id='start'",
        [json!({"record":started}).to_string()]).unwrap();
    flush(&tx).unwrap();
    assert_eq!(job_by_id(&tx, &id).unwrap().unwrap().state, JobState::Running);
    assert!(history_by_job(&tx, &id).unwrap().iter().all(|row| row.refusal.is_none()));
}

#[test]
fn repaired_claim_fences_read_only_queries_until_flush_matches_rebuild() {
    let mut connection = database();
    let tx = connection.transaction().unwrap();
    let id = job_id("person/fixture", "repair");
    let submitted = submission(&id, "repair");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    insert(&tx, &id, "submit", 1, &submitted);
    insert(&tx, &id, "launch", 2, &JobRecord::Transition {
        binding: binding.clone(), event: Event::LaunchIntent });
    insert(&tx, &id, "start", 3, &JobRecord::Transition {
        binding: binding.clone(), event: Event::Started });
    flush(&tx).unwrap();
    assert_eq!(job_by_id(&tx, &id).unwrap().unwrap().state, JobState::Running);
    tx.execute("INSERT INTO replica_records VALUES('start','valid',3)", []).unwrap();
    tx.execute("UPDATE replica_records SET state='repaired' WHERE claim_id='start'", []).unwrap();
    let changes = tx.total_changes();
    for error in [
        job_by_id(&tx, &id).unwrap_err(),
        history_by_job(&tx, &id).unwrap_err(),
        receipt_for_key(&tx, "person/fixture", "repair").unwrap_err(),
        queue_for_host(&tx, "fixture-host").unwrap_err(),
    ] {
        assert_eq!(error.downcast_ref::<JobsProjectionStale>().unwrap().subject, id);
    }
    assert!(queue_for_host(&tx, "unrelated-host").unwrap().is_empty());
    assert_eq!(tx.total_changes(), changes);
    flush(&tx).unwrap();
    assert_eq!(job_by_id(&tx, &id).unwrap().unwrap().state, JobState::Launching);
    let flushed = serde_json::to_value(history_by_job(&tx, &id).unwrap()).unwrap();
    let receipt = receipt_for_key(&tx, "person/fixture", "repair").unwrap();
    rebuild(&tx).unwrap();
    assert_eq!(serde_json::to_value(history_by_job(&tx, &id).unwrap()).unwrap(), flushed);
    assert_eq!(receipt_for_key(&tx, "person/fixture", "repair").unwrap(), receipt);
}
