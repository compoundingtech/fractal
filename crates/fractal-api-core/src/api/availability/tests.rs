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

fn latest_source(state: &AppState, kind: &str) -> Value {
    state.store.latest_claim("person/ada", Some(kind)).unwrap().unwrap().body["fields"]["source"].clone()
}

fn presence_source(state: &AppState) -> Value {
    latest_source(state, PRESENCE_KIND)
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
    assert_eq!(value["presence"], "unreported", "the person sees their own presence");
    // Every person-state claim records the daemon-assigned writer: this member, for a local caller.
    let host = json!(client_host_id(state.store.origin()));
    for kind in [fractal_schema::availability::STATUS_KIND, fractal_schema::availability::INTERRUPTIONS_KIND] {
        assert_eq!(latest_source(&state, kind), host, "{kind}");
    }
    let (status, cleared) = send(&app, "PUT", "/v1/client/availability", local("person/ada"), Some(json!({
        "status": {"value": null},
    }))).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!((&cleared["value"]["status"], &cleared["value"]["status_source"]), (&json!("available"), &json!("unreported")), "no device has reported");
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
        let (status, _) = send(&app, "POST", "/v1/client/presence", identity(), Some(json!({"state": "active", "observed_at": at(now)}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{actor}");
    }
    let unchanged = state.store.person_availability("person/ada", client_now_ms()).unwrap();
    assert_eq!(serde_json::to_value(unchanged).unwrap()["status_source"], "unreported");
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
        ("POST", "/v1/client/presence", json!({"state": "active", "observed_at": at(now)})),
    ] {
        assert_eq!(send(&app, method, path, bearer("reader"), Some(body)).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, before) = send(&app, "GET", "/v1/client/availability", bearer("writer"), None).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let before = &before["value"]["people"][0];
    assert_eq!((&before["status"], &before["status_source"], &before["presence"]), (&json!("available"), &json!("unreported"), &json!("unreported")));

    // The device reports its raw state and last input; the daemon derives presence and says
    // when to report again: the instant the device turns idle without newer input.
    let (status, reported) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(json!({
        "state": "active", "last_input_at": at(now - 2_000), "observed_at": at(now - 1_000),
    }))).await;
    assert_eq!(status, StatusCode::OK, "{reported}");
    let availability = &reported["value"]["availability"];
    assert_eq!((&availability["status"], &availability["status_source"]), (&json!("available"), &json!("presence")));
    assert_eq!((&availability["presence"], &availability["presence_observed_at"]), (&json!("present"), &json!(at(now - 1_000))));
    assert_eq!(reported["value"]["report_again_at"], json!(at(now - 2_000 + crate::store::availability::PRESENCE_IDLE_MS)));
    assert_eq!(presence_source(&state), "custom/client/writer");
    let (status, set) = send(&app, "PUT", "/v1/client/availability", bearer("writer"), Some(json!({
        "deadline_clock": {"value": "hold-routine", "until": at(now + 60 * MINUTE)},
    }))).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!((&set["value"]["deadline_clock"], &set["value"]["status"]), (&json!("hold-routine"), &json!("available")));
    assert_eq!(latest_source(&state, fractal_schema::availability::DEADLINE_CLOCK_KIND), "custom/client/writer");

    for body in [
        json!({"state": "inactive", "observed_at": at(now - 20 * MINUTE)}),
        json!({"state": "inactive", "observed_at": at(now + 10 * MINUTE)}),
        json!({"state": "active", "last_input_at": at(now + 1_000), "observed_at": at(now)}),
        json!({"state": "idle", "observed_at": at(now)}),
        json!({"presence": "present", "observed_at": at(now)}),
    ] {
        let (status, error) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(body.clone())).await;
        assert!(status.is_client_error() && status != StatusCode::FORBIDDEN, "{body}: {status} {error}");
    }
    let (status, _) = send(&app, "POST", "/v1/client/presence", bearer("writer"), Some(json!({
        "state": "active", "observed_at": at(now), "source": "custom/client/forged",
    }))).await;
    assert!(status.is_client_error(), "a client never names its presence source");

    // A local caller's reports come from this member.
    let (status, local_report) = send(&router(state.clone()), "POST", "/v1/client/presence", local("person/ada"), Some(json!({
        "state": "locked", "observed_at": at(now),
    }))).await;
    assert_eq!(status, StatusCode::OK, "{local_report}");
    assert_eq!(presence_source(&state), json!(client_host_id(state.store.origin())));
    assert_eq!(local_report["value"]["availability"]["presence"], "present", "a present device elsewhere still wins");
    assert!(local_report["value"]["report_again_at"].is_null(), "a locked device reports again only when its state changes");
}

#[tokio::test]
async fn availability_reads_show_raw_presence_only_to_the_person() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    let app = router(state.clone());
    let now = client_now_ms();
    let (status, _) = send(&app, "POST", "/v1/client/presence", local("person/ada"), Some(json!({
        "state": "active", "observed_at": at(now),
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

/// A local harness or peer that names a person is no person: it never writes the person's
/// availability or presence and never reads their raw presence.
#[tokio::test]
async fn a_local_peer_naming_the_person_neither_writes_nor_reads_raw_presence() {
    let root = tempfile::tempdir().unwrap();
    let state = super::super::tests::state(root.path());
    let now = client_now_ms();
    let person = || client_v0::ClientSession::local(Some("person/ada")).unwrap();
    let peer = || client_v0::ClientSession { local_peer: true, ..person() };
    let presence = || Json(serde_json::from_value::<PresenceReport>(json!({"state": "active", "observed_at": at(now)})).unwrap());
    let change = || Json(serde_json::from_value::<AvailabilitySet>(json!({"status": {"value": "away"}})).unwrap());

    let refused = report(State(state.clone()), Extension(peer()), presence()).await.unwrap_err();
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.message);
    let refused = set(State(state.clone()), Extension(peer()), change()).await.unwrap_err();
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.message);
    assert!(state.store.latest_claim("person/ada", Some(PRESENCE_KIND)).unwrap().is_none());
    assert!(state.store.latest_claim("person/ada", Some(fractal_schema::availability::STATUS_KIND)).unwrap().is_none());

    // The person's own session still writes.
    let Json(reported) = report(State(state.clone()), Extension(person()), presence()).await.unwrap();
    assert_eq!(reported["availability"]["presence"], "present", "{reported}");
    let Json(written) = set(State(state.clone()), Extension(person()), change()).await.unwrap();
    assert_eq!((&written["status"], &written["presence"]), (&json!("away"), &json!("present")), "{written}");

    let read = |session: client_v0::ClientSession, query: Option<&str>| {
        let state = state.clone();
        let query = AvailabilityQuery { person: query.map(str::to_owned) };
        async move { list(State(state), Extension(session), Query(query)).await.unwrap().1.0 }
    };
    for query in [None, Some("person/ada")] {
        let own = read(person(), query).await;
        assert_eq!(own["people"][0]["presence"], "present", "{own}");
        let coarse = read(peer(), query).await;
        let row = &coarse["people"][0];
        assert_eq!((&row["person"], &row["status"]), (&json!("person/ada"), &json!("away")), "{coarse}");
        assert!(row["presence"].is_null() && row["presence_observed_at"].is_null(), "{row}");
    }

    // Over the socket, an agent harness naming the person reads only coarsely too.
    let request = Request::builder().method("GET").uri("/v1/client/availability")
        .header("x-st3-person", "person/ada")
        .extension(BoundAgent("agent/worker".into()))
        .body(Body::empty()).unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap();
    let row = &body["value"]["people"][0];
    assert_eq!(row["person"], "person/ada", "{body}");
    assert!(row["presence"].is_null() && row["presence_observed_at"].is_null(), "{row}");
}
