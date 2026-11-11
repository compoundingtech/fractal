use super::*;

fn session(actor: &str) -> Extension<client_v0::ClientSession> {
    Extension(client_v0::ClientSession::for_tests_scoped(actor, actor, &["control.work", "read.projections"]))
}
fn config() -> Option<Extension<JobsConfig>> {
    Some(Extension(JobsConfig { enabled: true, allowed_classes: vec!["heavy".into()] }))
}
fn input(key: &str) -> Value {
    json!({"key":key,"execution_host":"node","request":{
        "source":{"kind":"content","sha256":vec![1;32]},
        "environment":{"kind":"exact","value":{}},
        "command":{"argv":["true"],"cwd":"."},
        "resources":{"cpu_millicores":1,"memory_bytes":1,"io_weight":1,"tasks_max":1,
            "expected_duration_ms":1,"run_timeout_ms":null,"priority":"background",
            "requested_class":"heavy","storage_request":null,"delegated_reservations":null}}})
}
fn state(root: &std::path::Path) -> AppState {
    let state = super::super::tests_shared::state(root);
    let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    state.store.bind_fleet("jobs-fixture").unwrap();
    state.store.pin_fleet_anchor(key.public()).unwrap();
    state.store.set_member_key(Some(key.clone())).unwrap();
    state.store.admit_fleet_anchor("jobs-fixture", key.public(), "listening").unwrap();
    state
}

#[tokio::test]
async fn receipt_retries_mismatch_and_requester_isolation() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let submit_as = |actor: &str, value| submit(State(state.clone()), session(actor), config(), Ok(Json(value)));
    let first = submit_as("person/a", input("same")).await.unwrap().0;
    assert_eq!(first["job"]["state"]["state"], "queued");
    assert_eq!(first["admission"], "not wired");
    let again = submit_as("person/a", input("same")).await.unwrap().0;
    assert_eq!(first, again);
    let mut changed = input("same");
    changed["request"]["command"]["argv"] = json!(["false"]);
    let error = submit_as("person/a", changed).await.unwrap_err();
    assert_eq!(error.code, "idempotency-digest-mismatch");
    assert_eq!(error.status, StatusCode::CONFLICT);
    let other = submit_as("person/other", input("same")).await.unwrap().0;
    assert_ne!(first["job"]["id"], other["job"]["id"]);
    let id = first["job"]["id"].as_str().unwrap();
    assert_eq!(show(State(state.clone()), session("person/other"), config(), AxumPath(id.into())).await.unwrap_err().status, StatusCode::NOT_FOUND);
    let history = read(&state, |tx| projection::history_by_job(tx, id)).unwrap();
    assert_eq!(history.len(), 1);
}

#[tokio::test]
async fn disabled_routes_fail_closed_and_staleness_is_typed() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests_shared::state(root.path());
    assert!(!JobsConfig::default().enabled);
    assert_eq!(enabled(Some(&JobsConfig::default())).unwrap_err().code, "jobs-disabled");
    assert!(enabled(Some(&JobsConfig { enabled: true, allowed_classes: vec![] })).is_err());
    assert_eq!(submit(State(state.clone()), session("person/a"), None, Ok(Json(input("x")))).await.unwrap_err().code, "jobs-disabled");
    assert_eq!(show(State(state.clone()), session("person/a"), None, AxumPath("job/x".into())).await.unwrap_err().code, "jobs-disabled");
    assert_eq!(list(State(state), session("person/a"), None, Query(ListQuery::default())).await.unwrap_err().code, "jobs-disabled");
    let error = read_error(JobsProjectionStale { subject: "job/x".into() }.into());
    assert_eq!(error.code, "jobs-projection-stale");
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn declarations_never_grant_admission_and_lists_keep_submission_order() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let mut ids = vec![];
    for (key, priority) in [("z", "normal"), ("a", "background")] {
        let mut raw = input(key);
        raw["request"]["resources"]["priority"] = json!(priority);
        let result = submit(State(state.clone()), session("person/a"), config(), Ok(Json(raw))).await.unwrap().0;
        assert_eq!(result["admission"], "not wired");
        assert_eq!(result["job"]["state"]["state"], "queued");
        ids.push(result["job"]["id"].clone());
    }
    for query in [ListQuery::default(), ListQuery { host: Some("node".into()), requester: None }] {
        let rows = list(State(state.clone()), session("person/a"), config(), Query(query)).await.unwrap().0;
        assert_eq!(rows["jobs"].as_array().unwrap().iter().map(|job| job["id"].clone()).collect::<Vec<_>>(), ids);
    }
    let cfg = config().unwrap().0;
    let mut raw = input("bad");
    raw["request"]["resources"]["priority"] = json!("urgent");
    assert_eq!(decode(raw, &cfg).err().unwrap().code, "job-priority-unknown");
    let mut raw = input("bad");
    raw["request"]["resources"]["requested_class"] = json!("elevated");
    assert_eq!(decode(raw, &cfg).err().unwrap().code, "job-class-not-allowed");
    let mut raw = input("bad");
    raw["requester"] = json!("person/intruder");
    assert_eq!(decode(raw, &cfg).err().unwrap().code, "invalid-request");
}

#[tokio::test]
async fn real_router_injects_configuration_and_preserves_disabled_reason() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests_shared::state(root.path());
    for (configuration, expected) in [
        (JobsConfig::default(), StatusCode::SERVICE_UNAVAILABLE),
        (config().unwrap().0, StatusCode::OK),
    ] {
        let app = super::super::router(state.clone()).layer(Extension(configuration));
        let request = Request::builder().uri("/v1/client/jobs")
            .header(client_v0::LOCAL_PERSON_HEADER, "person/a")
            .body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::SERVICE_UNAVAILABLE {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["code"], "jobs-disabled");
        }
    }
}

#[tokio::test]
async fn concurrent_submissions_resolve_one_durable_receipt() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let request = decode(input("race"), &config().unwrap().0).unwrap().request;
    let identity = IdempotencyKey {
        requester: "person/a".into(), key: "race".into(), request_digest: request.digest(),
    };
    let id = projection::job_id(&identity.requester, &identity.key);
    let declaration = read(&state, |tx| {
        let membership = smallclaims::store::fleet_membership_tx(tx)?;
        let MemberState::Current(member) = membership.state("node") else { unreachable!() };
        Ok(member.admitted_claim.clone())
    }).unwrap();
    let record = JobRecord::Submitted {
        binding: JobHostBinding {
            job_id: id.clone(), execution_host: "node".into(), authority_declaration: declaration,
        },
        identity: identity.clone(), request: Box::new(request),
    };
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let launch = || {
        let (state, barrier, identity, id, record) =
            (state.clone(), barrier.clone(), identity.clone(), id.clone(), record.clone());
        tokio::task::spawn_blocking(move || {
            let precheck = receipt(&state, &identity);
            // Reach the rendezvous even when a precheck fails, so neither worker is stranded.
            barrier.wait();
            assert!(matches!(precheck.unwrap(), IdempotencyDecision::NewJob));
            record_submission(&state, &identity, &id, record).unwrap()
        })
    };
    let (first, second) = tokio::join!(launch(), launch());
    // Exactly one worker had to recover from stale-subject: this proves CAS contention,
    // unlike a barrier before submit, where one worker can observe the other's receipt.
    assert_ne!(first.unwrap(), second.unwrap());
    assert_eq!(read(&state, |tx| projection::history_by_job(tx, &id)).unwrap().len(), 1);
    assert!(matches!(receipt(&state, &identity).unwrap(), IdempotencyDecision::ExistingJob(job)
        if job == id));
}

#[tokio::test]
async fn requester_private_routes_refuse_forged_or_unresolved_bound_callers() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let app = super::super::router(state.clone()).layer(config().unwrap())
        .layer(from_fn_with_state((state.store.clone(), None), guard_bound_actor));
    for (method, path) in [
        ("GET", "/v1/client/jobs"),
        ("GET", "/v1/client/jobs/job/fixture"),
        ("POST", "/v1/client/jobs"),
        ("POST", "/v1/client/jobs/job/fixture/cancel"),
    ] {
        let request = Request::builder().method(method).uri(path)
            .header(client_v0::LOCAL_PERSON_HEADER, "person/other")
            .header("content-type", "application/json")
            .extension(BoundAgent("agent/own".into()))
            .body(Body::from(input("forged").to_string())).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: Value = serde_json::from_slice(&bytes).unwrap();
        // The outer bound-actor guard rejects first; client-v0 normalizes its code.
        assert_eq!(error["code"], "forbidden");
        assert_eq!(error["message"], "this harness is `agent/own` and cannot act as `person/other`");
    }
    for method in ["GET", "POST"] {
        let request = Request::builder().method(method).uri("/v1/client/jobs")
            .extension(HarnessAncestry::Unavailable).body(Body::empty()).unwrap();
        assert!(!app.clone().oneshot(request).await.unwrap().status().is_success());
    }
}

#[tokio::test]
async fn pending_subject_is_a_typed_stale_error_through_the_router() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let mut state = super::super::tests_shared::state(root.path());
    let path = root.path().join("graph.db");
    state.store = Arc::new(Store::open(&path, "node").unwrap());
    // A separate connection models a repair commit whose projection is still pending.
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute("INSERT INTO job_pending(subject) VALUES('job/pending')", []).unwrap();
    let app = super::super::router(state).layer(config().unwrap());
    let request = Request::builder().uri("/v1/client/jobs")
        .header(client_v0::LOCAL_PERSON_HEADER, "person/a").body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "jobs-projection-stale");
}

fn record_event(state: &AppState, id: &str, event: fractal_jobs::Event) {
    let row = read(state, |tx| projection::job_by_id(tx, id)).unwrap().unwrap();
    let record = JobRecord::Transition { binding: row.binding, event };
    state.store.append_claim(&ClaimInput {
        subject: id.into(), kind: projection::KIND.into(), actor: Some("person/a".into()),
        fields: BTreeMap::from([("record".into(), serde_json::to_value(record).unwrap())]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[tokio::test]
async fn cancellation_is_requester_only_idempotent_and_preserves_receipts() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let original = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("withdraw")))).await.unwrap().0;
    let id = original["job"]["id"].as_str().unwrap();
    for (actor, code) in [("person/operator", "not-requester"), ("", "requester-unresolved")] {
        let error = cancel::cancel(State(state.clone()), session(actor), config(),
            AxumPath(format!("{id}/cancel"))).await.unwrap_err();
        assert_eq!(error.code, code);
    }
    let app = super::super::router(state.clone()).layer(config().unwrap());
    let request = Request::builder().method("POST").uri(format!("/v1/client/jobs/{id}/cancel"))
        .header(client_v0::LOCAL_PERSON_HEADER, "person/a").body(Body::empty()).unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::OK);
    let again = cancel::cancel(State(state.clone()), session("person/a"), config(),
        AxumPath(format!("{id}/cancel"))).await.unwrap().0;
    assert_eq!(again["job"]["state"]["state"], "withdrawn");
    let repeated = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("withdraw")))).await.unwrap().0;
    assert_eq!(again, repeated);
    assert_eq!(read(&state, |tx| projection::history_by_job(tx, id)).unwrap().len(), 2);
    let fresh = submit(State(state), session("person/a"), config(),
        Ok(Json(input("new-attempt")))).await.unwrap().0;
    assert_ne!(fresh["job"]["id"], again["job"]["id"]);
    assert_eq!(fresh["job"]["state"]["state"], "queued");
}

#[tokio::test]
async fn running_cancel_records_intent_without_claiming_a_whole_tree_stop() {
    use fractal_jobs::Event;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let submitted = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("running")))).await.unwrap().0;
    let id = submitted["job"]["id"].as_str().unwrap();
    for event in [Event::Prepare, Event::Prepared] {
        record_event(&state, id, event);
    }
    let row = read(&state, |tx| projection::job_by_id(tx, id)).unwrap().unwrap();
    let record = JobRecord::LaunchIntent {
        binding: row.binding,
        intent: fractal_jobs::launch::LaunchIntent {
            generation: fractal_jobs::launch::ExecutionGeneration(std::num::NonZeroU64::new(1).unwrap()),
            outcome_file: root.path().join("outcome.json").to_str().unwrap().into(),
        },
    };
    state.store.append_claim(&ClaimInput {
        subject: id.into(), kind: projection::KIND.into(), actor: Some("person/a".into()),
        fields: BTreeMap::from([("record".into(), serde_json::to_value(record).unwrap())]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    record_event(&state, id, Event::Started);
    assert_eq!(read(&state, |tx| projection::job_by_id(tx, id)).unwrap().unwrap().state,
        fractal_jobs::State::Running);
    let response = cancel::cancel(State(state.clone()), session("person/a"), config(),
        AxumPath(format!("{id}/cancel"))).await.unwrap().0;
    assert_eq!(response["job"]["state"]["state"], "cancel-requested");
    assert_eq!(response["admission"], "not wired");
    let error = cancel::cancel(State(state), session("person/a"), None,
        AxumPath(format!("{id}/cancel"))).await.unwrap_err();
    assert_eq!(error.code, "jobs-disabled");
}

#[tokio::test]
async fn explicit_lease_expiry_records_its_reason_and_preserves_the_submission_receipt() {
    use fractal_jobs::stop_intent::{self, StopDecision, StopInput, StopOrigin};
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let submitted = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("expiry")))).await.unwrap().0;
    let id = submitted["job"]["id"].as_str().unwrap();
    let row = read(&state, |tx| projection::job_by_id(tx, id)).unwrap().unwrap();
    let origin = StopOrigin::Requester { actor: Some("person/a") };
    let StopDecision::Record { event, reason } =
        stop_intent::decide(row.state, &row.requester, origin, StopInput::RequesterLeaseExpired)
            .unwrap() else { panic!("queued lease expiry must withdraw") };
    let record = JobRecord::Transition { binding: row.binding, event };
    let claim = state.store.append_claim(&ClaimInput {
        subject: id.into(), kind: projection::KIND.into(), actor: Some("person/a".into()),
        fields: BTreeMap::from([
            ("record".into(), serde_json::to_value(record).unwrap()),
            ("reason".into(), serde_json::to_value(reason).unwrap()),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert_eq!(claim.body["fields"]["reason"], "requester-lease-expired");
    let retry = submit(State(state), session("person/a"), config(),
        Ok(Json(input("expiry")))).await.unwrap().0;
    assert_eq!(retry["job"]["id"], submitted["job"]["id"]);
    assert_eq!(retry["job"]["state"]["state"], "withdrawn");
}

#[tokio::test]
async fn pre_unit6_claim_without_reason_validates_replicates_and_folds() {
    let root = tempfile::tempdir().unwrap();
    let source = state(root.path());
    let submitted = submit(State(source.clone()), session("person/a"), config(),
        Ok(Json(input("old-shape")))).await.unwrap().0;
    let id = submitted["job"]["id"].as_str().unwrap();
    let history = read(&source, |tx| projection::history_by_job(tx, id)).unwrap();
    let claim = source.store.claim_by_id(&history[0].claim_id).unwrap().unwrap();
    // Exactly the pre-unit-6 fields object: record only, with no reason field.
    let fields: BTreeMap<String, Value> =
        serde_json::from_value(claim.body["fields"].clone()).unwrap();
    assert_eq!(fields.len(), 1);
    assert!(fields.contains_key("record"));
    let schema = fractal_schema::registry();
    schema.validate_claim(&claim.subject, &claim.kind, &fields).unwrap();
    schema.validate_replicated_claim(&claim.subject, &claim.kind, &fields).unwrap();
    let key = read(&source, |tx| {
        let membership = smallclaims::store::fleet_membership_tx(tx)?;
        let MemberState::Current(member) = membership.state("node") else { unreachable!() };
        Ok(member.member_key.clone())
    }).unwrap();
    let target = Store::open_memory("receiver").unwrap();
    target.pin_fleet_anchor(&key).unwrap();
    let exchange = source.store.export_replication_exchange("jobs-fixture", &Default::default())
        .unwrap();
    target.receive_replication_exchange("node", "jobs-fixture", &exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    target.project_replication_backlog().unwrap();
    assert_eq!(target.claim_by_id(&claim.id).unwrap().unwrap().body, claim.body);
    let reader = target.readers.get();
    let tx = reader.unchecked_transaction().unwrap();
    assert_eq!(projection::job_by_id(&tx, id).unwrap().unwrap().state, fractal_jobs::State::Queued);
    assert_eq!(projection::receipt_for_key(&tx, "person/a", "old-shape").unwrap().unwrap().1, id);
}

#[tokio::test]
async fn concurrent_queued_cancels_write_one_transition_and_refuse_the_stale_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let submitted = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("cancel-race")))).await.unwrap().0;
    let id = submitted["job"]["id"].as_str().unwrap().to_owned();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let launch = || {
        let (state, id, barrier) = (state.clone(), id.clone(), barrier.clone());
        tokio::task::spawn_blocking(move || {
            let snapshot = read(&state, |tx| {
                Ok((projection::job_by_id(tx, &id)?,
                    smallclaims::store::latest_claim_id_tx(tx, &id)?))
            });
            // Never unwrap or assert before the rendezvous: failures cannot strand a peer.
            barrier.wait();
            let (row, head) = snapshot.unwrap();
            cancel::cancel_snapshot(&state, &session("person/a").0, &id, row.unwrap(), head)
        })
    };
    let (first, second) = tokio::join!(launch(), launch());
    let results = [first.unwrap(), second.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results.iter().find_map(|result| result.as_ref().err()).unwrap();
    assert_eq!(error.code, "job-state-changed");
    assert_eq!(error.status, StatusCode::CONFLICT);
    let history = read(&state, |tx| projection::history_by_job(tx, &id)).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].state, Some(fractal_jobs::State::Withdrawn));
}

#[tokio::test]
async fn cancellation_suffix_edges_do_not_mutate_the_job() {
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let submitted = submit(State(state.clone()), session("person/a"), config(),
        Ok(Json(input("suffix")))).await.unwrap().0;
    let id = submitted["job"]["id"].as_str().unwrap();
    let app = super::super::router(state.clone()).layer(config().unwrap());
    for (method, suffix) in [("POST", "/cancel/cancel"), ("GET", "/cancel"), ("POST", "")] {
        let request = Request::builder().method(method)
            .uri(format!("/v1/client/jobs/{id}{suffix}"))
            .header(client_v0::LOCAL_PERSON_HEADER, "person/a").body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "not-found");
    }
    let row = read(&state, |tx| projection::job_by_id(tx, id)).unwrap().unwrap();
    assert_eq!(row.state, fractal_jobs::State::Queued);
    assert_eq!(read(&state, |tx| projection::history_by_job(tx, id)).unwrap().len(), 1);
}
