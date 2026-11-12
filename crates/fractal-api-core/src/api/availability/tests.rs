use super::*;
use axum::body::to_bytes;
use fractal_schema::availability::PRESENCE_KIND;
use sha2::Sha256;

const MINUTE: u128 = 60_000;

fn at(ms: u128) -> String {
    crate::store::availability::rfc3339(ms).unwrap()
}

fn local(actor: &str) -> (&'static str, String) {
    ("x-st3-person", actor.into())
}

fn bearer(credential: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {credential}"))
}

async fn send(app: &Router, method: &str, path: &str, (header, value): (&str, String), body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path).header(header, value);
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A device paired to person/ada with exactly `scopes`.
fn pair(state: &AppState, credential: &str, scopes: &[&str]) {
    state.store.append_client_operation_claim(&ClaimInput {
        subject: format!("custom/client/{credential}"),
        kind: "custom.client.pairing-completed".into(),
        actor: Some("person/ada".into()),
        fields: BTreeMap::from([
            ("credential_hash".into(), json!(hex::encode(Sha256::digest(credential.as_bytes())))),
            ("session_actor".into(), json!(format!("client/{credential}"))),
            ("person_id".into(), json!("person/ada")),
            ("scopes".into(), json!(scopes)),
            ("expires_at_unix_ms".into(), json!(u64::try_from(client_now_ms()).unwrap() + 60_000)),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
}

fn presence_source(state: &AppState) -> Value {
    state.store.latest_claim("person/ada", Some(PRESENCE_KIND)).unwrap().unwrap().body["fields"]["source"].clone()
}

#[tokio::test]
async fn availability_writes_act_as_the_session_person_and_refuse_agents() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    let app = router(state.clone());
    let now = client_now_ms();
    let (status, set) = send(&app, "PUT", "/v1/client/availability", local("person/ada"), Some(json!({
        "status": {"value": "away", "until": at(now + 10 * MINUTE)},
        "interruptions": {"value": "urgent"},
    }))).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    let value = &set["value"];
    assert_eq!(value["person"], "person/ada");
    assert_eq!((&value["status"], &value["status_source"], &value["interruptions"]), (&json!("away"), &json!("override"), &json!("urgent")));
    assert!(value["status_until"].is_string() && value["interruptions_until"].is_null());
    assert_eq!(value["presence"], "unknown", "the person sees their own presence");
    let (status, cleared) = send(&app, "PUT", "/v1/client/availability", local("person/ada"), Some(json!({
        "status": {"value": null},
    }))).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!((&cleared["value"]["status"], &cleared["value"]["status_source"]), (&json!("away"), &json!("presence")));
    assert_eq!(cleared["value"]["interruptions"], "urgent", "an absent axis is unchanged");

    for body in [
        json!({}),
        json!({"status": {"value": null, "until": at(now + MINUTE)}}),
        json!({"interruptions": {"value": "all", "until": at(now - MINUTE)}}),
        json!({"deadline_clock": {"value": "hold-all", "until": "2099-01-01T00:00:00+01:00"}}),
    ] {
        let (status, error) = send(&app, "PUT", "/v1/client/availability", local("person/ada"), Some(body.clone())).await;
        assert_eq!((status, &error["code"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")), "{body}: {error}");
    }
    // The body never names a person, and a status change always names its value.
    for body in [
        json!({"person": "person/avery", "status": {"value": "away"}}),
        json!({"status": {"until": at(now + MINUTE)}}),
        json!({"interruptions": {"value": "focus"}}),
    ] {
        let (status, _) = send(&app, "PUT", "/v1/client/availability", local("person/ada"), Some(body.clone())).await;
        assert!(status.is_client_error() && status != StatusCode::FORBIDDEN, "{body}: {status}");
    }

    // A local agent session holds every scope, yet never writes a person's availability.
    for actor in ["agent/worker", "client/local/read-only"] {
        let identity = || if actor.starts_with("agent/") { local(actor) } else { ("x-st3-features", String::new()) };
        let (status, _) = send(&app, "PUT", "/v1/client/availability", identity(), Some(json!({"status": {"value": "available"}}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor}");
        let (status, _) = send(&app, "POST", "/v1/client/presence", identity(), Some(json!({"presence": "present", "observed_at": at(now)}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor}");
    }
    let unchanged = state.store.person_availability("person/ada", client_now_ms()).unwrap();
    assert_eq!(serde_json::to_value(unchanged).unwrap()["status_source"], "presence");
    assert!(state.store.latest_claim("person/ada", Some(PRESENCE_KIND)).unwrap().is_none());
}

#[tokio::test]
async fn paired_writes_need_their_scope_and_presence_names_the_pairing_grant() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    pair(&state, "writer", &["read.projections", "write.availability", "write.presence"]);
    pair(&state, "reader", &["read.projections"]);
    let app = fabric_router(state.clone());
    let now = client_now_ms();
    for (method, path, body) in [
        ("PUT", "/v1/client/availability", json!({"status": {"value": "away"}})),
        ("POST", "/v1/client/presence", json!({"presence": "present", "observed_at": at(now)})),
    ] {
        assert_eq!(send(&app, method, path, bearer("reader"), Some(body)).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, before) = send(&app, "GET", "/v1/client/availability", bearer("writer"), None).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!((&before["value"]["people"][0]["status"], &before["value"]["people"][0]["presence"]), (&json!("away"), &json!("unknown")));

    let (status, reported) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(json!({
        "presence": "present", "observed_at": at(now - 1_000),
    }))).await;
    assert_eq!(status, StatusCode::OK, "{reported}");
    assert_eq!((&reported["value"]["status"], &reported["value"]["status_source"]), (&json!("available"), &json!("presence")));
    assert_eq!((&reported["value"]["presence"], &reported["value"]["presence_observed_at"]), (&json!("present"), &json!(at(now - 1_000))));
    assert_eq!(presence_source(&state), "custom/client/writer");
    let (status, set) = send(&app, "PUT", "/v1/client/availability", bearer("writer"), Some(json!({
        "deadline_clock": {"value": "hold-routine", "until": at(now + 60 * MINUTE)},
    }))).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!((&set["value"]["deadline_clock"], &set["value"]["status"]), (&json!("hold-routine"), &json!("available")));

    for observed in [now - 20 * MINUTE, now + 10 * MINUTE] {
        let (status, error) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(json!({
            "presence": "idle", "observed_at": at(observed),
        }))).await;
        assert_eq!((status, &error["code"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")), "{error}");
    }
    let (status, _) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(json!({
        "presence": "present", "observed_at": at(now), "source": "custom/client/forged",
    }))).await;
    assert!(status.is_client_error(), "a client never names its presence source");

    // A local caller's reports come from this member.
    let (status, local_report) = send(&router(state.clone()), "POST", "/v1/client/presence", local("person/ada"), Some(json!({
        "presence": "locked", "observed_at": at(now),
    }))).await;
    assert_eq!(status, StatusCode::OK, "{local_report}");
    assert_eq!(presence_source(&state), json!(client_host_id(state.store.origin())));
    assert_eq!(local_report["value"]["presence"], "present", "a fresh present report from another source still wins");
}

#[tokio::test]
async fn availability_reads_show_raw_presence_only_to_the_person() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    let app = router(state.clone());
    let now = client_now_ms();
    let (status, _) = send(&app, "POST", "/v1/client/presence", local("person/ada"), Some(json!({
        "presence": "present", "observed_at": at(now),
    }))).await;
    assert_eq!(status, StatusCode::OK);

    let (status, own) = send(&app, "GET", "/v1/client/availability", local("person/ada"), None).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    let own = own["value"]["people"].as_array().unwrap().clone();
    assert_eq!((own.len(), &own[0]["person"], &own[0]["presence"]), (1, &json!("person/ada"), &json!("present")));
    assert!(own[0]["presence_observed_at"].is_string());

    let coarse = |people: &Value| {
        let row = &people["value"]["people"][0];
        assert_eq!((&row["person"], &row["status"], &row["status_source"]), (&json!("person/ada"), &json!("available"), &json!("presence")));
        assert!(row["presence"].is_null() && row["presence_observed_at"].is_null(), "{row}");
        assert_eq!(people["value"]["people"].as_array().unwrap().len(), 1);
    };
    for reader in ["person/avery", "agent/worker"] {
        let (status, other) = send(&app, "GET", "/v1/client/availability?person=person/ada", local(reader), None).await;
        assert_eq!(status, StatusCode::OK, "{other}");
        coarse(&other);
    }
    // Without `person`, an agent reads the people whose directive notes it reads.
    fractal_store::store::directive_notes::tests_shared_seats::declare(
        &state.store, "version 2\nagent \"garden/helper\" { workspace \"/tmp\"; command \"true\"; }", "person/ada", "helper",
    );
    let (status, related) = send(&app, "GET", "/v1/client/availability", local("agent/garden/helper"), None).await;
    assert_eq!(status, StatusCode::OK, "{related}");
    coarse(&related);
    let (status, unrelated) = send(&app, "GET", "/v1/client/availability", local("agent/worker"), None).await;
    assert_eq!((status, &unrelated["value"]["people"]), (StatusCode::OK, &json!([])));

    let (status, _) = send(&app, "GET", "/v1/client/availability?person=agent/worker", local("person/ada"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = send(&app, "GET", "/v1/client/availability", ("x-st3-features", String::new()), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an anonymous local reader is no person or agent");
}

#[tokio::test]
async fn a_paired_device_writes_the_persons_note_and_a_related_agent_reads_it() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    fractal_store::store::directive_notes::tests_shared_seats::enable(&state.store);
    fractal_store::store::directive_notes::tests_shared_seats::declare(
        &state.store, "version 2\nagent \"garden/helper\" { workspace \"/tmp\"; command \"true\"; }", "person/ada", "helper",
    );
    pair(&state, "writer", &["read.projections", "write.notes"]);
    pair(&state, "reader", &["read.projections", "write.availability"]);
    let app = fabric_router(state.clone());
    let note = json!({"text": "Travelling until Monday; only ask blockers.", "expires_at": "2099-01-01T00:00:00Z"});

    assert_eq!(send(&app, "PUT", "/v1/client/notes", bearer("reader"), Some(note.clone())).await.0, StatusCode::FORBIDDEN);
    // A local agent session holds every scope, yet never writes a person's note.
    let (status, _) = send(&router(state.clone()), "PUT", "/v1/client/notes", local("agent/garden/helper"), Some(note.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The body never names the person or the author, and `text` is always present.
    for body in [
        json!({"person": "person/avery", "text": "Approved"}),
        json!({"actor": "person/ada", "text": "Approved"}),
        json!({"expires_at": "2099-01-01T00:00:00Z"}),
    ] {
        let (status, _) = send(&app, "PUT", "/v1/client/notes", bearer("writer"), Some(body.clone())).await;
        assert!(status.is_client_error() && status != StatusCode::FORBIDDEN, "{body}: {status}");
    }
    for body in [json!({"text": "   "}), json!({"text": "Later", "expires_at": "2099-01-01T00:00:00+01:00"})] {
        let (status, error) = send(&app, "PUT", "/v1/client/notes", bearer("writer"), Some(body.clone())).await;
        assert_eq!((status, &error["code"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")), "{body}: {error}");
    }
    assert!(state.store.directive_notes("agent/garden/helper").unwrap().is_empty());

    let (status, written) = send(&app, "PUT", "/v1/client/notes", bearer("writer"), Some(note)).await;
    assert_eq!(status, StatusCode::OK, "{written}");
    let written = &written["value"]["note"];
    assert_eq!((&written["person"], &written["author"]), (&json!("person/ada"), &json!("person/ada")));
    let (status, read) = send(&router(state.clone()), "GET", "/v1/client/notes", local("agent/garden/helper"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["value"]["notes"], json!([written]));

    let (status, cleared) = send(&app, "PUT", "/v1/client/notes", bearer("writer"), Some(json!({"text": null}))).await;
    assert_eq!((status, &cleared["value"]["note"]), (StatusCode::OK, &Value::Null), "{cleared}");
    assert!(state.store.directive_notes("agent/garden/helper").unwrap().is_empty());
}
