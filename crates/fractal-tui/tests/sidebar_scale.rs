//! Pre-activation daemon gates. D-dependent tests are explicitly ignored, never simulated.
use fractal_daemon::{ArrangementsStore, model::ClaimInput, store::Store};
use fractal_sidebar::{client::{MembershipPage, membership_page}, paging::{Effect, Event, Paging, Scope, Status}};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::{Duration, Instant}};
const CONTAINER: &str = "arrangement/person/avery/00000000-0000-7000-8000-000000010000";
fn claim(subject: &str, kind: &str, fields: Value) -> ClaimInput {
    ClaimInput { subject: subject.into(), kind: kind.into(), actor: Some("person/avery".into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None }
}
fn unrelated(store: &Store, n: usize) {
    store.append_claim(&claim(&format!("message/unrelated-{n}"), "message.sent",
        json!({"status":"sent","from":"person/avery","to":"person/avery","content":"unrelated"}))).unwrap();
}
fn place(i: usize) -> Value { json!({"op":"place","member":format!("message/scale-{i:05}"),"bucket":null,"key":"a0"}) }
fn fixture(store: &Store, count: usize) {
    store.edit_arrangement(&claim(CONTAINER, "arrangement.edited", json!({"owner":"person/avery","version":2,"operations":[{"op":"create","name":"Scale"}]})), &BTreeMap::new()).unwrap();
    for i in 0..=count {
        store.append_claim(&claim(&format!("message/scale-{i:05}"), "message.sent",
            json!({"status":"sent","from":"person/avery","to":"person/avery","content":"Scale"}))).unwrap();
    }
    for start in (0..count).step_by(1024) {
        store.edit_ordered_memberships(&claim(CONTAINER, "ordered-membership.edited",
            json!({"owner":"person/avery","operations":(start..(start+1024).min(count)).map(place).collect::<Vec<_>>()})), &BTreeMap::new()).unwrap();
    }
}
#[test]
fn sidebar_scale_real_server_refuses_4097th_live_member_atomically() {
    let store = Store::open_memory("sidebar-scale-limit").unwrap(); fixture(&store, 4096);
    assert_eq!(store.ordered_membership_count(CONTAINER).unwrap(), 4096);
    let index = store.index().unwrap();
    let error = store.edit_ordered_memberships(&claim(CONTAINER, "ordered-membership.edited",
        json!({"owner":"person/avery","operations":[place(4096)]})), &BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("limit"), "{error}");
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(store.ordered_membership_count(CONTAINER).unwrap(), 4096);
}
#[allow(clippy::assertions_on_constants, reason = "ignored latency gate must compile in debug but reject debug measurements at runtime")]
async fn socket_load(count: usize, sustained: bool) {
    assert!(!cfg!(debug_assertions), "measure --release only");
    let root = tempfile::tempdir().unwrap(); let socket = root.path().join("scale.sock");
    let store = Arc::new(Store::open(&root.path().join("graph.db"), "sidebar-scale").unwrap()); fixture(&store, count);
    let state = fractal_daemon::api::AppState { browser_pairing_login: None, seat_admission: Default::default(), owning_person: None, store: store.clone(),
        notify: Arc::new(tokio::sync::Notify::new()), event_notify: tokio::sync::watch::channel(0_u64).0,
        node: "sidebar-scale".into(), api_socket: root.path().join("api.sock"), state_dir: root.path().into(), pty_root: root.path().join("pty"), pty_binary: "pty".into(),
        fleet_id: None, configured_peers: vec![], client_relay: None, native_session_home: None, harness_question_sink: None,
        assistant_receipts: Default::default(), private_notes: Default::default(), planner_default: Default::default() };
    let path = socket.clone();
    let (ready, listening) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix_with_ready(&path, fractal_daemon::api::router(state), || { let _ = ready.send(()); }).await
    });
    if listening.await.is_err() {
        server.await.expect("private scale daemon task failed").expect("private scale daemon startup failed");
        panic!("private scale daemon exited without signaling readiness");
    }
    let http = reqwest::Client::builder().unix_socket(socket).build().unwrap();
    let (person, uuid) = CONTAINER.strip_prefix("arrangement/person/").unwrap().split_once('/').unwrap();
    let memberships_url = format!("http://localhost/v1/client/arrangements/{person}/{uuid}/memberships");
    let mut samples = Vec::new();
    for round in 0..20 {
        let mut paging = Paging::new(None);
        // Discover real host/projection identity, never synthesize the daemon epoch.
        let bytes = http.get("http://localhost/v1/client/capabilities").header("x-st3-person", "person/avery").send().await.unwrap().bytes().await.unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        let scope = Scope { owner: "person/avery".into(), container: CONTAINER.into(), endpoint: "local".into(),
            host: response["snapshot"]["host_id"].as_str().unwrap().into(), projection: response["snapshot"]["projection_version"].as_str().unwrap().into() };
        let start = Instant::now(); let mut effects = paging.reduce(Event::Refresh(scope, round)); let mut requests = 0;
        loop {
            let (token, cursor) = match effects.as_slice() { [Effect::SubscribeAndRead(token)] => (token.clone(), None),
                [Effect::Read(token, cursor)] => (token.clone(), Some(cursor.clone())), [Effect::PersistBase] => break, other => panic!("unexpected effects {other:?}") };
            if sustained { unrelated(&store, round as usize * 100 + requests); }
            let mut request = http.get(&memberships_url).header("x-st3-person", "person/avery")
                .query(&[("person", "person/avery"), ("limit", "200"), ("paging_contract", "collection-v1")]);
            if let Some(cursor) = cursor { request = request.query(&[("cursor", cursor)]); }
            let response = request.send().await.unwrap(); let status = response.status(); let bytes = response.bytes().await.unwrap();
            assert!(status.is_success(), "requires D #4739: {status} {}", String::from_utf8_lossy(&bytes));
            let envelope: fractal_client::Envelope<MembershipPage> = serde_json::from_slice(&bytes).expect("D must supply collection frontier/epoch");
            effects = paging.reduce(membership_page(token, envelope)); requests += 1;
            assert!(requests <= 21, "no unbounded restart or repeated cursor");
        }
        samples.push(start.elapsed());
        assert_eq!(paging.status, Status::Ready); assert!(!paging.stale());
        let complete = paging.complete().unwrap(); assert_eq!(complete.members.len(), count);
        assert_eq!(complete.frontier.live_count, count as u64);
        assert!(complete.members.windows(2).all(|pair| pair[0].member < pair[1].member));
    }
    server.abort(); let _ = server.await;
    samples.sort_unstable(); let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    println!("local_socket_initial_load sustained={sustained} members={count} samples=20 p95_ms={:.3}", p95.as_secs_f64()*1000.0);
    assert!(p95 <= Duration::from_secs(1), "initial load p95 {p95:?}");
}
#[tokio::test]
#[ignore = "requires D #4739 collection-v1 socket paging; explicit release initial-load p95 gate"]
async fn sidebar_scale_socket_initial_load() { for count in [1001, 4096] { socket_load(count, false).await; } }
#[tokio::test]
#[ignore = "requires D #4739 collection-scoped cursors; unrelated write between every initial-load page"]
async fn sidebar_scale_socket_sustained_unrelated_writes() { for count in [1001, 4096] { socket_load(count, true).await; } }
