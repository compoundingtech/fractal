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
    let submitted = submission("job/fixture", "fixture");
    let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
    insert(&tx, "job/fixture", "later", 3, &JobRecord::Transition {
        binding: binding.clone(), event: Event::Started });
    insert(&tx, "job/fixture", "first", 1, &submitted);
    insert(&tx, "job/fixture", "middle", 2, &JobRecord::Transition {
        binding: binding.clone(), event: Event::LaunchIntent });
    refold(&tx, "job/fixture").unwrap();
    let state: String = tx.query_row("SELECT state FROM jobs", [], |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JobState>(&state).unwrap(), JobState::Running);
    let mut conflict = binding.clone();
    conflict.execution_host = "different-host".into();
    insert(&tx, "job/fixture", "conflict", 4, &JobRecord::Transition { binding: conflict, event: Event::HostLost });
    insert(&tx, "job/fixture", "illegal", 5, &JobRecord::Transition { binding: binding.clone(), event: Event::Prepared });
    refold(&tx, "job/fixture").unwrap();
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
