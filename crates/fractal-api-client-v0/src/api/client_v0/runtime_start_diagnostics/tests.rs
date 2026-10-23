use super::super::*;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt as _;

const OPERATION: &str = "0194b2e0-1234-7000-8000-000000000001";
const SECRET: &str = "diagnostics-must-not-expose-this-secret";

fn authorize(state: &AppState, credential: &str, scopes: Value) {
    state.store.append_claim(&ClaimInput {
        subject: format!("custom/client/{credential}"),
        kind: "custom.client.pairing-completed".into(),
        actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("credential_hash".into(), json!(credential_digest(credential))),
            ("session_actor".into(), json!("client/runtime-start-reader")),
            ("person_id".into(), json!("person/robin")),
            ("scopes".into(), scopes),
            ("expires_at_unix_ms".into(), json!(client_now_ms() as u64 + 60_000)),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn seed(connection: &Connection, origin: &str) {
    let request = json!({
        "id":"local/start-request", "subject":"agent/example-gate", "origin":origin,
        "kind":"runtime.action.requested", "actor":null, "accepted_at_unix_ms":123456,
        "body":{"fields":{"action":"start", "operation":OPERATION,
            "runtime_id":"runtime/example-gate", "terminal":false, "env":SECRET},
            "evidence":["selected/example"], "unexpected":SECRET},
        "unexpected":SECRET
    });
    let body = json!({
        "member":{"runtime_id":"runtime/example-gate", "terminal":false,
            "command":SECRET, "env":{"SECRET":SECRET}},
        "selected_token":"selected/example", "desired_token":"desired/example",
        "expected_incarnation":"incarnation/example-prior", "start_request":"local/start-request",
        "start_request_position":17, "start_request_receipt":request, "reason":SECRET,
        "request":"restart/example"
    });
    let proof = json!({
        "version":1, "operation_id":OPERATION, "subject":"agent/example-gate",
        "owner":"owner/example", "origin":origin, "runtime_id":"runtime/example-gate",
        "terminal":false, "runtime_kind":"exec", "selected_token":"selected/example",
        "desired_token":"desired/example", "expected_incarnation":"incarnation/example-prior",
        "incarnation":null, "verification":"producer-closed", "start_request":"local/start-request",
        "start_request_receipt":request, "start_request_position":17,
        "control_request":null, "control_request_receipt":null, "requester_restart":"restart/example",
        "witness":{"kind":"before-backend", "unexpected":SECRET}, "unexpected":SECRET
    });
    let result = json!({
        "error":SECRET, "phase":"failed", "producer_closed":true,
        "closure_kind":"before-backend", "verification":"producer-closed",
        "closure_proof_id":"start-proof/example", "closure_proof":proof, "unexpected":SECRET
    });
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hdeaea3ec9912 owner=fractal_api_client_v0::api::client_v0::runtime_start_diagnostics::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_runtime_starts(id,subject,owner,body,revoked_by,finished,result,applied,cleanup_verified,dispatched,receipted)
         VALUES(?1,'agent/example-gate','owner/example',?2,'restart/example',1,?3,1,0,1,1)",
        params![OPERATION, body.to_string(), result.to_string()],
    ).unwrap();
    for (id, subject, finished, applied, dispatched, result) in [
        ("0194b2e0-1234-7000-8000-000000000002", "agent/example-retired", true, true, true,
            Some(json!({"phase":"retired", "verification":"operation-retired"}).to_string())),
        ("0194b2e0-1234-7000-8000-000000000003", "agent/example-pending", false, false, false, None),
        ("0194b2e0-1234-7000-8000-000000000004", "agent/example-running", false, false, true, None),
    ] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hf4488b956ae7 owner=fractal_api_client_v0::api::client_v0::runtime_start_diagnostics::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_runtime_starts(id,subject,owner,body,finished,applied,dispatched,result)
             VALUES(?1,?2,'owner/example','{}',?3,?4,?5,?6)",
            params![id, subject, finished, applied, dispatched, result],
        ).unwrap();
    }
}

async fn request(state: &AppState, method: &str, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = fabric_router(state.clone()).oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
    let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, value)
}

fn fingerprint(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "SELECT * FROM local_runtime_starts ORDER BY id",
        "SELECT * FROM local_runtime_start_observation_generation ORDER BY id",
        "SELECT * FROM local_runtime_start_observation_subjects ORDER BY subject",
        "SELECT * FROM local_runtime_start_evidence ORDER BY operation_id,observation_id",
        "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name",
    ].iter().map(|sql| {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h1c1e8afac510 owner=fractal_api_client_v0::api::client_v0::runtime_start_diagnostics::tests TODO=trace-statement-and-value-representation */ sql).unwrap();
        let count = statement.column_count();
        statement.query_map([], |row| (0..count).map(|index| row.get(index)).collect())
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }).collect()
}

#[tokio::test]
async fn authenticated_runtime_start_diagnostics_are_safe_paged_and_read_only() {
    let root = tempfile::tempdir().unwrap();
    let mut state = super::super::tests::test_state_named(root.path(), "runtime-start-diagnostics");
    state.owning_person = Some("person/robin".into());
    authorize(&state, "runtime-start-reader", json!(["read.projections"]));
    authorize(&state, "runtime-start-denied", json!([]));
    let connection = Connection::open(root.path().join("graph.db")).unwrap();
    crate::store::configure_projection_writer(&connection).unwrap();
    seed(&connection, state.store.origin());
    let before = fingerprint(&connection);
    let index = state.store.index().unwrap();
    for (token, expected) in [
        (None, StatusCode::FORBIDDEN),
        (Some("invalid"), StatusCode::FORBIDDEN),
        (Some("runtime-start-denied"), StatusCode::FORBIDDEN),
    ] {
        let (status, value) = request(&state, "GET", "/v1/client/runtime-starts", token).await;
        assert_eq!(status, expected, "{value}");
        assert!(value.get("value").is_none(), "{value}");
    }
    let (status, first) = request(&state, "GET", "/v1/client/runtime-starts?limit=2", Some("runtime-start-reader")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["api_version"], CLIENT_API_VERSION);
    assert_eq!(first["value"]["collection"], "runtime-starts");
    assert_eq!(first["snapshot"]["store_index"], index);
    assert_eq!(first["value"]["items"].as_array().unwrap().len(), 2);
    let cursor = first["value"]["page"]["next_cursor"].as_str().unwrap();
    let (status, second) = request(&state, "GET", &format!("/v1/client/runtime-starts?limit=2&cursor={}", urlencoding::encode(cursor)), Some("runtime-start-reader")).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["snapshot"], first["snapshot"]);
    let items = first["value"]["items"].as_array().unwrap().iter()
        .chain(second["value"]["items"].as_array().unwrap()).collect::<Vec<_>>();
    assert_eq!(items.len(), 4);
    for subject in ["agent/example-gate", "agent/example-retired", "agent/example-pending", "agent/example-running"] {
        assert!(items.iter().any(|item| item["subject"] == subject));
    }
    let item = items.iter().find(|item| item["operation_id"] == OPERATION).unwrap();
    assert_eq!(item["phase"], "revoked");
    assert_eq!(item["result_phase"], "failed");
    for field in ["dispatched", "finished", "applied", "receipted", "producer_closed"] {
        assert_eq!(item[field], true, "{field}: {item}");
    }
    assert_eq!(item["cleanup_verified"], false, "a read must not close even a retained witness");
    assert_eq!(item["result_kind"], "error");
    assert_eq!(item["closure_kind"], "before-backend");
    assert_eq!(item["closure_proof_id"], "start-proof/example");
    assert_eq!(item["provenance"]["start_request_position"], 17);
    assert_eq!(item["provenance"]["start_request_receipt"]["fields"]["operation"], OPERATION);
    let proof = &item["closure_proof"];
    assert_eq!(proof["operation_id"], OPERATION);
    assert_eq!(proof["subject"], "agent/example-gate");
    assert_eq!(proof["owner"], "owner/example");
    assert_eq!(proof["origin"], state.store.origin());
    assert_eq!(proof["runtime_id"], "runtime/example-gate");
    assert_eq!(proof["terminal"], false);
    assert_eq!(proof["witness"]["kind"], "before-backend");
    assert_eq!(proof["start_request_receipt"]["id"], "local/start-request");
    assert_eq!(proof["requester_restart"], "restart/example");
    assert_eq!(item["created_at_unix_ms"], u64::from_str_radix("0194b2e01234", 16).unwrap());
    assert_eq!(item["requested_at_unix_ms"], 123456);
    assert!(item["dispatched_at_unix_ms"].is_null());
    assert!(item["finished_at_unix_ms"].is_null());
    assert_eq!(item["timestamp_sources"]["created_at_unix_ms"], "operation-id-uuidv7");
    assert_eq!(item["timestamp_sources"]["finished_at_unix_ms"], "unavailable");
    assert!(!first.to_string().contains(SECRET));
    assert!(!second.to_string().contains(SECRET));
    let (_, repeated) = request(&state, "GET", "/v1/client/runtime-starts?limit=500", Some("runtime-start-reader")).await;
    assert_eq!(repeated["value"]["items"].as_array().unwrap().len(), 4);
    let (status, _) = request(&state, "POST", "/v1/client/runtime-starts", Some("runtime-start-reader")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(state.store.index().unwrap(), index);
    assert_eq!(fingerprint(&connection), before, "GET/auth failures/POST must not mutate ledger, evidence, schema or generations");
}
