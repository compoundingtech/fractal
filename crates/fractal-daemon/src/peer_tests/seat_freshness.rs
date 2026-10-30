//! Seat freshness answered by the seat's host through its replication worker (contract C3 of the
//! claims-store breakup); these run the API, so they live in fractal-daemon.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use smallclaims::sync::{FleetAuth, FleetContext, PeerState, peer_router};
use tokio::sync::watch;
use tower::ServiceExt as _;

use crate::config::{Config, PeerConfig};
use crate::model::ClaimInput;
use crate::store::{RelayedFreshness, Store};
use fractal_peer::peer::*;

const OWNER: &str = "conversation-owner";
const SEAT: &str = "agent/cedar";

fn app(root: &Path, node: &str) -> crate::api::AppState {
    crate::api::AppState {
        owning_person: None,
        store: Arc::new(Store::open(&root.join(format!("{node}.db")), node).unwrap()),
        notify: Arc::new(tokio::sync::Notify::new()),
        event_notify: watch::channel(0_u64).0,
        node: node.into(),
        state_dir: root.into(),
        pty_root: root.join("pty"),
        pty_binary: "pty".into(),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        native_session_home: None,
        planner_default: crate::model::PlannerSpec::default(),
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
        harness_question_sink: None,
    }
}

fn append(store: &Store, subject: &str, kind: &str, actor: Option<&str>, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

async fn serve(root: &Path, state: crate::api::AppState) -> PathBuf {
    let socket = root.join("owner.sock");
    let served = socket.clone();
    tokio::spawn(async move { crate::api::serve_unix(&served, crate::api::router(state)).await });
    for _ in 0..200 {
        if tokio::net::UnixStream::connect(&socket).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    socket
}

/// A gateway relay whose only peer is a worker serving `routes` for the owner's API.
async fn gateway_to(root: &Path, owner_socket: &Path, routes: axum::Router<PeerState<MainBackend>>) -> ClientRelay {
    let peer = PeerState::new(
        MainBackend::new(owner_socket.to_path_buf()),
        OWNER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from(["conversation-gateway".into()])),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, peer_router(peer, routes)).await });
    let secret = root.join("fleet-secret");
    fs::write(&secret, [7_u8; 32]).unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    ClientRelay::from_config(&Config {
        node: "conversation-gateway".into(),
        fleet_id: Some("fleet-test".into()),
        shared_secret_file: Some(secret),
        peers: vec![PeerConfig { name: OWNER.into(), url: format!("http://{address}") }],
        ..Default::default()
    })
    .unwrap()
    .unwrap()
}

/// The seat's host answers, signed, when it last heard each seat it holds a local report of: the
/// newest local re-stamp, which never replicated. A gateway's card then hears the seat from that
/// answer while it stands.
#[tokio::test]
async fn a_seats_host_answers_when_it_heard_its_seats_and_the_gateway_hears_them_from_it() {
    let root = tempfile::tempdir().unwrap();
    let owner = app(root.path(), OWNER);
    append(&owner.store, SEAT, "runtime.observed", Some(SEAT), json!({"status":"running", "runtime_id":"native", "incarnation_id":"one"}));
    append(&owner.store, SEAT, "harness.observed", Some(SEAT), json!({"state":"idle", "driver":"codex", "incarnation_id":"one"}));
    let index = owner.store.index().unwrap();
    append(&owner.store, SEAT, "harness.observed", Some(SEAT), json!({"state":"idle", "driver":"codex", "incarnation_id":"one"}));
    assert_eq!(owner.store.index().unwrap(), index, "the re-stamp stayed in the host's local log");
    let (incarnation, heard) = owner.store.local_harness_newest(SEAT).unwrap().unwrap();
    assert_eq!(incarnation, "one");
    let gateway_store = Store::open_memory("conversation-gateway").unwrap();
    gateway_store.import_replication(OWNER, &owner.store.export_replication(0).unwrap()).unwrap();
    let socket = serve(root.path(), owner).await;
    let relay = gateway_to(root.path(), &socket, fractal_routes(crate::api::raw_terminal_owner_splice)).await;

    let request = SeatFreshnessRequest { agents: vec![SEAT.into(), "agent/unknown".into()] };
    let reply = relay.seat_freshness(OWNER, &request).await.unwrap().expect("the host serves the route");
    assert_eq!(
        reply.seats,
        [SeatHeard { agent: SEAT.into(), incarnation_id: "one".into(), heard_at_unix_ms: heard as u64 }],
        "a seat the host holds no report of is left out"
    );
    assert!(
        relay.seat_freshness(OWNER, &SeatFreshnessRequest { agents: vec!["message/x".into()] }).await.is_err(),
        "only seats are asked about"
    );

    // The gateway's replicated report is an hour old; the host heard the seat just now.
    let now = smallclaims::store::now_ms();
    let mut harness = gateway_store.observed_harness_at(SEAT, gateway_store.index().unwrap()).unwrap().unwrap();
    harness.observed_at_unix_ms = now - 3_600_000;
    assert_eq!(crate::api::heard_at(&gateway_store, SEAT, &harness, now).unwrap(), now - 3_600_000);
    gateway_store.extension::<RelayedFreshness>().record(
        reply.seats.into_iter().map(|seat| (seat.agent, seat.incarnation_id, u128::from(seat.heard_at_unix_ms))),
        now,
    );
    assert_eq!(crate::api::heard_at(&gateway_store, SEAT, &harness, now).unwrap(), heard);
    let mut other = harness.clone();
    other.incarnation_id = "zero".into();
    assert_eq!(
        crate::api::heard_at(&gateway_store, SEAT, &other, now).unwrap(),
        now - 3_600_000,
        "the answer is about one incarnation"
    );
    assert_eq!(
        crate::api::heard_at(&gateway_store, SEAT, &harness, now + RELAYED_FRESHNESS_TTL).unwrap(),
        now - 3_600_000,
        "an answer older than its standing says nothing"
    );
}

const RELAYED_FRESHNESS_TTL: u128 = crate::store::RELAYED_FRESHNESS_TTL_MS;

/// A request without the fleet's signature is refused before the host's API is asked.
#[tokio::test]
async fn an_unsigned_seat_freshness_request_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let socket = serve(root.path(), app(root.path(), OWNER)).await;
    let peer = PeerState::new(
        MainBackend::new(socket),
        OWNER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from(["conversation-gateway".into()])),
    );
    let body = serde_json::to_vec(&SeatFreshnessRequest { agents: vec![SEAT.into()] }).unwrap();
    let response = peer_router(peer, fractal_routes(crate::api::raw_terminal_owner_splice))
        .oneshot(Request::builder().method("POST").uri(SEAT_FRESHNESS_PATH).body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(to_bytes(response.into_body(), 4_096).await.is_ok());
}

/// Mixed versions, both ways. A member is asked only once its own replicated `daemon.started`
/// advertises the route, so an older member is never sent it and its seats keep their replicated
/// re-stamps; a worker that predates the route answers `None`, never an error to a card.
#[tokio::test]
async fn an_older_member_is_never_asked_and_an_older_worker_answers_none() {
    let root = tempfile::tempdir().unwrap();
    let owner = Store::open_memory(OWNER).unwrap();
    let gateway = Store::open_memory("conversation-gateway").unwrap();
    let source = format!("version 2\nagent \"cedar\" {{ host \"{OWNER}\"; workspace \"/tmp\"; command \"true\" }}\n");
    let intent = crate::parse_intent(&source, "conversation-gateway").unwrap();
    let mission = gateway
        .mission(&intent, crate::model::IntentInput { kdl: source.clone(), source_name: None })
        .unwrap();
    gateway.apply(&intent, &mission.subject_tokens, "declare-cedar").unwrap();
    let declared = gateway
        .desired_subjects()
        .unwrap()
        .into_iter()
        .filter(|desired| desired.member.as_ref().is_some_and(|member| member.host == OWNER))
        .map(|desired| desired.subject)
        .collect::<Vec<_>>();
    assert_eq!(declared.len(), 1, "the declaration places one seat on the owner");
    let members = [OWNER.to_owned()];
    let targets = || crate::api::seat_freshness::seat_freshness_targets(&gateway, "conversation-gateway", &members).unwrap();

    // The owner runs an older build: its daemon.started names no seat_freshness.
    append(&owner, &format!("daemon/{OWNER}"), "daemon.started", None, json!({"status":"running", "features":{"schedule_standby":1}}));
    gateway.import_replication(OWNER, &owner.export_replication(0).unwrap()).unwrap();
    assert!(targets().is_empty(), "an older member is not asked");
    // The same advertisement written by another member does not count for the owner.
    append(&gateway, &format!("daemon/{OWNER}"), "daemon.started", None, json!({"status":"running", "features":{"seat_freshness":1}}));
    assert!(targets().is_empty(), "only the member's own advertisement counts");
    // It upgrades.
    append(&owner, &format!("daemon/{OWNER}"), "daemon.started", None, json!({"status":"running", "features":{"seat_freshness":1}}));
    gateway.import_replication(OWNER, &owner.export_replication(0).unwrap()).unwrap();
    assert_eq!(targets().into_iter().collect::<Vec<_>>(), [(OWNER.to_owned(), declared)]);

    // A worker from before the route answers Axum's unsigned 404: no answer, and no error.
    let socket = serve(root.path(), app(root.path(), OWNER)).await;
    let relay = gateway_to(root.path(), &socket, axum::Router::new()).await;
    assert_eq!(
        relay.seat_freshness(OWNER, &SeatFreshnessRequest { agents: vec![SEAT.into()] }).await.unwrap(),
        None
    );
}
