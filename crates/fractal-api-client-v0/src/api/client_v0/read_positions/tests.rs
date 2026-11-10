use super::*;
use super::super::tests::test_state_named;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt;

const ADA: &str = "person/ada";
const SEAT: &str = "agent/fleet/fixture-seat";
const T1: &str = "2026-11-08T10:00:01.000Z";
const T2: &str = "2026-11-08T10:00:02.000Z";
const T3: &str = "2026-11-08T10:00:03.000Z";

fn request(state: &AppState, key: &str, owner: &str, conversation: &str, at: &str) -> ActionRequest {
    serde_json::from_value(json!({
        "api_version":CLIENT_API_VERSION, "id":format!("action/{key}"),
        "type":"read-position.advance", "idempotency_key":format!("read-position-test-{key}"),
        "fence":{"snapshot_id":new_client_snapshot(state).id},
        "parameters":{"owner":owner,"conversation":conversation,"at":at}
    })).unwrap()
}

async fn invoke(state: &AppState, session: &ClientSession, request: ActionRequest) -> Result<Value, ApiError> {
    action(State(state.clone()), Extension(new_client_snapshot(state)), Extension(session.clone()), Json(request))
        .await
        .map(|response| response.0)
}

fn claims(state: &AppState) -> usize {
    state.store.claims_for("read-position/person/ada", Some(schema::KIND)).unwrap().len()
}

#[tokio::test]
async fn advances_only_move_forward_and_report_the_stored_position() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "read-position-forward");
    let session = ClientSession::local(Some(ADA)).unwrap();
    let first = request(&state, "first", ADA, SEAT, T2);
    let accepted = invoke(&state, &session, first.clone()).await.unwrap();
    assert_eq!(accepted["status"], "completed");
    assert_eq!(accepted["read_position_at"], T2);
    assert_eq!(accepted["affected_ids"], json!([format!("read-position/person/ada/{SEAT}")]));
    assert_eq!(claims(&state), 1);
    // Older and equal advances complete without a claim and name the stored, larger position.
    for (key, at) in [("older", T1), ("equal", T2)] {
        let stale = invoke(&state, &session, request(&state, key, ADA, SEAT, at)).await.unwrap();
        assert_eq!(stale["status"], "completed", "{key}");
        assert_eq!(stale["read_position_at"], T2, "{key}");
        assert_eq!(claims(&state), 1, "{key} appended no claim");
        assert_eq!(state.store.read_positions(ADA, state.store.index().unwrap()).unwrap()[0]["at"], T2);
    }
    let later = invoke(&state, &session, request(&state, "later", ADA, SEAT, T3)).await.unwrap();
    assert_eq!(later["read_position_at"], T3);
    assert_eq!(claims(&state), 2);
    // An exact retry replays its receipt; the position stays at the larger value.
    let replay = invoke(&state, &session, first).await.unwrap();
    assert_eq!(replay["operation_id"], accepted["operation_id"]);
    assert_eq!(claims(&state), 2);
    let rows = state.store.read_positions(ADA, state.store.index().unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["at"], T3);
    assert_eq!(rows[0]["conversation"], SEAT);
}

#[tokio::test]
async fn advances_validate_their_parameters() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "read-position-validation");
    let session = ClientSession::local(Some(ADA)).unwrap();
    for (key, conversation, at) in [
        ("no-family", "seat", T1),
        ("whitespace", "agent/a b", T1),
        ("not-a-time", SEAT, "yesterday"),
        ("before-epoch", SEAT, "1969-12-31T23:59:59.000Z"),
    ] {
        let error = invoke(&state, &session, request(&state, key, ADA, conversation, at)).await.unwrap_err();
        assert_eq!(error.code, "validation-failed", "{key}: {error:?}");
    }
    let mut unknown = request(&state, "unknown-field", ADA, SEAT, T1);
    unknown.parameters["extra"] = json!(true);
    assert_eq!(invoke(&state, &session, unknown).await.unwrap_err().code, "validation-failed");
    assert_eq!(claims(&state), 0);
}

#[tokio::test]
async fn other_people_can_neither_read_nor_advance_a_persons_positions() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "read-position-cross-person");
    let ada = ClientSession::local(Some(ADA)).unwrap();
    let robin = ClientSession::local(Some("person/robin")).unwrap();
    invoke(&state, &ada, request(&state, "ada-own", ADA, SEAT, T1)).await.unwrap();
    let refused = invoke(&state, &robin, request(&state, "robin-for-ada", ADA, SEAT, T3)).await.unwrap_err();
    assert_eq!(refused.code, "forbidden");
    assert_eq!(claims(&state), 1);
    let response = crate::api::router(state.clone()).oneshot(Request::builder()
        .uri("/v1/client/read-positions?person=person%2Fada")
        .header(LOCAL_PERSON_HEADER, "person/robin").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let read_slots = Arc::new(tokio::sync::Semaphore::new(1));
    let subscription: CollectionSubscribe = serde_json::from_value(json!({
        "kind":"subscribe","id":"read-positions","collection":"read-positions","person":ADA,"limit":10
    })).unwrap();
    assert_eq!(
        collection_items(&state, &robin, &subscription, read_slots.clone().acquire_owned().await.unwrap()).await.unwrap_err().code,
        "forbidden"
    );
    let mut omitted = subscription.clone();
    omitted.person = None;
    assert!(collection_items(&state, &ada, &omitted, read_slots.acquire_owned().await.unwrap()).await.is_err());
    // A trusted local fleet agent acts for the person, as for arrangements.
    let agent = ClientSession::local(Some("agent/fleet/fixture-helper")).unwrap();
    let advanced = invoke(&state, &agent, request(&state, "agent-for-ada", ADA, SEAT, T2)).await.unwrap();
    assert_eq!(advanced["read_position_at"], T2);
}

#[tokio::test]
async fn list_pages_keep_their_snapshot_across_later_advances() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "read-position-pages");
    let session = ClientSession::local(Some(ADA)).unwrap();
    for (key, conversation) in [("one", "agent/fleet/fixture-one"), ("two", "agent/fleet/fixture-two"), ("three", "session/three")] {
        invoke(&state, &session, request(&state, key, ADA, conversation, T1)).await.unwrap();
    }
    let app = crate::api::router(state.clone());
    let get_page = |uri: String| {
        let app = app.clone();
        async move {
            let response = app.oneshot(Request::builder().uri(uri).header(LOCAL_PERSON_HEADER, ADA)
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            serde_json::from_slice::<Value>(&to_bytes(response.into_body(), CLIENT_MAX_RESPONSE_BYTES).await.unwrap()).unwrap()
        }
    };
    let first = get_page("/v1/client/read-positions?person=person%2Fada&limit=1".into()).await;
    assert_eq!(first["value"]["collection"], "read-positions");
    invoke(&state, &session, request(&state, "during-pages", ADA, "agent/fleet/fixture-one", T3)).await.unwrap();
    let mut page = first.clone();
    let mut seen = vec![];
    loop {
        assert_eq!(page["snapshot"]["id"], first["snapshot"]["id"]);
        let item = &page["value"]["items"][0];
        assert_eq!(item["kind"], "read-position");
        assert_eq!(item["owner"], ADA);
        seen.push((item["conversation"].as_str().unwrap().to_owned(), item["at"].as_str().unwrap().to_owned()));
        let Some(cursor) = page["value"]["page"]["next_cursor"].as_str() else { break; };
        page = get_page(format!("/v1/client/read-positions?person=person%2Fada&limit=1&cursor={cursor}")).await;
    }
    assert_eq!(seen, [
        ("agent/fleet/fixture-one".to_owned(), T1.to_owned()),
        ("agent/fleet/fixture-two".to_owned(), T1.to_owned()),
        ("session/three".to_owned(), T1.to_owned()),
    ]);
    let fresh = get_page("/v1/client/read-positions?person=person%2Fada".into()).await;
    assert_eq!(fresh["value"]["items"][0]["at"], T3);
    assert_eq!(fresh["value"]["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn streams_are_invalidated_by_any_change_to_the_persons_positions() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state_named(root.path(), "read-position-stream");
    let session = ClientSession::local(Some(ADA)).unwrap();
    // Only read positions move the read-positions collection, and they move no other.
    assert!(!collection_ignores("read-positions", schema::KIND));
    assert!(collection_ignores("read-positions", "arrangement.edited"));
    assert!(!collection_ignores("read-positions", "custom.client.pairing-revoked"));
    for collection in ["missions", "attention", "agents", "work", "glasses", "arrangements"] {
        assert!(collection_ignores(collection, schema::KIND), "{collection}");
    }
    invoke(&state, &session, request(&state, "first-row", ADA, "agent/fleet/fixture-a", T1)).await.unwrap();
    let read_slots = Arc::new(tokio::sync::Semaphore::new(1));
    // A one-row window: an advance outside its rows still changes what it pins.
    let subscription: CollectionSubscribe = serde_json::from_value(json!({
        "kind":"subscribe","id":"read-positions","collection":"read-positions","person":ADA,"limit":1
    })).unwrap();
    let windows = collection_windows::Windows::attach(&state.store).unwrap();
    let before = windows.changes(&state.store).unwrap();
    invoke(&state, &session, request(&state, "second-row", ADA, "session/z", T1)).await.unwrap();
    let (_, rows, has_more, pinned) = collection_items(&state, &session, &subscription, read_slots.clone().acquire_owned().await.unwrap()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["conversation"], "agent/fleet/fixture-a");
    assert!(has_more);
    let after = windows.changes(&state.store).unwrap();
    assert!(collection_windows::Windows::changed("read-positions", &before, &after));
    invoke(&state, &session, request(&state, "outside-window", ADA, "session/z", T3)).await.unwrap();
    let (_, unchanged_rows, _, moved) = collection_items(&state, &session, &subscription, read_slots.clone().acquire_owned().await.unwrap()).await.unwrap();
    assert_eq!(unchanged_rows, rows, "the visible row did not change");
    assert_ne!(moved, pinned, "the pinned owner revision moved, so the window sends a frame");
    assert!(matches!(moved, Some(CollectionRevision::Owner(_))));
    let later = windows.changes(&state.store).unwrap();
    assert!(collection_windows::Windows::changed("read-positions", &after, &later));
    // A stale advance appends nothing, so the pinned revision stays.
    invoke(&state, &session, request(&state, "stale", ADA, "session/z", T2)).await.unwrap();
    let (_, _, _, stale) = collection_items(&state, &session, &subscription, read_slots.acquire_owned().await.unwrap()).await.unwrap();
    assert_eq!(stale, moved);
    assert!(!state.store.read_positions_changed(state.store.index().unwrap(), state.store.index().unwrap()).unwrap());
}
