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
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let launch = || {
        let state = state.clone();
        let barrier = barrier.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            runtime.block_on(submit(State(state), session("person/a"), config(), Ok(Json(input("race")))))
                .unwrap().0["job"]["id"].clone()
        })
    };
    let (first, second) = tokio::join!(launch(), launch());
    let first = first.unwrap();
    assert_eq!(first, second.unwrap());
    let history = read(&state, |tx| projection::history_by_job(tx, first.as_str().unwrap())).unwrap();
    assert_eq!(history.len(), 1);
}
