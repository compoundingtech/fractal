//! Seat freshness pushed by the seat's host through the receiver's replication worker (contract
//! C3 of the claims-store breakup); these run the API, so they live in fractal-daemon.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use smallclaims::sync::{FleetAuth, FleetContext, PeerState, peer_router};
use tokio::sync::watch;
use tower::ServiceExt as _;

use crate::api::seat_freshness::{SeatFreshnessPushed, push_seat_freshness};
use crate::config::{Config, PeerConfig};
use crate::model::ClaimInput;
use crate::store::{RelayedFreshness, Store};
use fractal_peer::peer::*;

/// The member that receives pushes; its worker serves the route.
const RECEIVER: &str = "conversation-owner";
/// The seats' host, which pushes.
const HOST: &str = "conversation-gateway";

fn app(store: Arc<Store>, node: &str, root: &Path) -> crate::api::AppState {
    crate::api::AppState {
        owning_person: None,
        store,
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

/// Declare `name` on `host` in `store`, and return its subject.
fn declare(store: &Store, name: &str, host: &str) -> String {
    let source = format!("version 2\nagent \"{name}\" {{ host \"{host}\"; workspace \"/tmp\"; command \"true\" }}\n");
    let intent = crate::parse_intent(&source, HOST).unwrap();
    let mission = store
        .mission(&intent, crate::model::IntentInput { kdl: source.clone(), source_name: None })
        .unwrap();
    store.apply(&intent, &mission.subject_tokens, &format!("declare-{name}")).unwrap();
    store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .find(|desired| desired.subject.ends_with(name))
        .unwrap()
        .subject
}

fn report(store: &Store, seat: &str, incarnation: &str) {
    append(store, seat, "harness.observed", Some(seat), json!({"state":"idle", "driver":"codex", "incarnation_id":incarnation}));
}

/// `member` advertises seat freshness in its own `daemon.started`, as `store` replicates it.
fn advertise(member: &Store, store: &Store, feature: bool) {
    let features = if feature { json!({"seat_freshness":1}) } else { json!({"schedule_standby":1}) };
    append(member, &format!("daemon/{}", member.origin()), "daemon.started", None, json!({"status":"running", "features":features}));
    store.import_replication(member.origin(), &member.export_replication(0).unwrap()).unwrap();
}

async fn serve(root: &Path, state: crate::api::AppState) -> PathBuf {
    let socket = root.join(format!("{}.sock", state.node));
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

/// A worker for the receiver's API at `socket`, serving `routes`; its URL.
async fn worker(socket: &Path, routes: axum::Router<PeerState<MainBackend>>) -> String {
    let peer = PeerState::new(
        MainBackend::new(socket.to_path_buf()),
        RECEIVER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from([HOST.into()])),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, peer_router(peer, routes)).await });
    format!("http://{address}")
}

/// A peer that accepts connections and never answers.
async fn hung() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    format!("http://{address}")
}

/// The host's relay, reaching each `(name, url)`.
fn relay(root: &Path, peers: &[(&str, String)]) -> ClientRelay {
    let secret = root.join("fleet-secret");
    fs::write(&secret, [7_u8; 32]).unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    ClientRelay::from_config(&Config {
        node: HOST.into(),
        fleet_id: Some("fleet-test".into()),
        shared_secret_file: Some(secret),
        peers: peers.iter().map(|(name, url)| PeerConfig { name: (*name).into(), url: url.clone() }).collect(),
        ..Default::default()
    })
    .unwrap()
    .unwrap()
}

/// The host pushes, signed, when it heard its seats; the receiver records a seat only when its
/// declaration places the seat on the member that pushed it, and its cards then hear the seat.
#[tokio::test]
async fn a_member_records_the_freshness_a_host_pushes_for_its_own_seats_only() {
    let root = tempfile::tempdir().unwrap();
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    let ours = declare(&receiver, "cedar", HOST);
    let theirs = declare(&receiver, "birch", "elsewhere");
    let url = worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, fractal_routes(crate::api::raw_terminal_owner_splice)).await;
    let relay = relay(root.path(), &[(RECEIVER, url)]);
    let heard = |agent: &str, at: u64| SeatHeard { agent: agent.into(), incarnation_id: "one".into(), heard_at_unix_ms: at };
    let push = SeatFreshnessPush { seats: vec![heard(&ours, 1_000), heard(&theirs, 1_000)] };
    let ack = relay.push_seat_freshness(RECEIVER, &push, Duration::from_secs(10)).await.unwrap().unwrap();
    assert_eq!(ack.recorded, std::slice::from_ref(&ours), "a member speaks only for the seats it hosts");
    let relayed = receiver.extension::<RelayedFreshness>();
    assert_eq!(relayed.heard_at(&ours, "one"), Some(1_000));
    assert_eq!(relayed.heard_at(&theirs, "one"), None);
    assert_eq!(relayed.heard_at(&ours, "zero"), None, "the time is of one incarnation");
    // A host whose clock runs ahead cannot keep a seat fresh: a time past now plus a minute is
    // taken as that.
    let ahead = (smallclaims::store::now_ms() + 3_600_000) as u64;
    let before = smallclaims::store::now_ms();
    relay.push_seat_freshness(RECEIVER, &SeatFreshnessPush { seats: vec![SeatHeard { agent: ours.clone(), incarnation_id: "skewed".into(), heard_at_unix_ms: ahead }] }, Duration::from_secs(10)).await.unwrap();
    let clamped = relayed.heard_at(&ours, "skewed").unwrap();
    assert!(clamped <= smallclaims::store::now_ms() + 60_000 && clamped >= before + 60_000, "{clamped}");
    relay.push_seat_freshness(RECEIVER, &SeatFreshnessPush { seats: vec![heard(&ours, 1_000)] }, Duration::from_secs(10)).await.unwrap();
    // An older push never moves the time back.
    relay.push_seat_freshness(RECEIVER, &SeatFreshnessPush { seats: vec![heard(&ours, 500)] }, Duration::from_secs(10)).await.unwrap();
    assert_eq!(relayed.heard_at(&ours, "one"), Some(1_000));
    assert!(
        relay.push_seat_freshness(RECEIVER, &SeatFreshnessPush { seats: vec![heard("message/x", 1)] }, Duration::from_secs(10)).await.is_err(),
        "only seats are pushed"
    );
}

/// A round reads the host's seats in one statement and pushes each member only what changed;
/// nothing changed sends nothing, and a member that restarted gets every seat again.
#[tokio::test]
async fn a_round_pushes_what_changed_and_everything_to_a_restarted_member() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    let birch = declare(&host, "birch", HOST);
    report(&host, &cedar, "one");
    report(&host, &birch, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
    let receiver_member = Store::open_memory(RECEIVER).unwrap();
    advertise(&receiver_member, &host, true);
    let url = worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, fractal_routes(crate::api::raw_terminal_owner_splice)).await;
    let relay = relay(root.path(), &[(RECEIVER, url)]);
    let members = [HOST.to_owned(), RECEIVER.to_owned()];
    let mut pushed = SeatFreshnessPushed::default();
    let mut round = async || push_seat_freshness(&host, &relay, HOST, &members, &mut pushed, Duration::from_secs(10)).await.unwrap();

    assert_eq!(round().await.pushed.get(RECEIVER), Some(&2), "the first round pushes every seat");
    let relayed = receiver.extension::<RelayedFreshness>();
    let (_, cedar_heard) = host.local_harness_newest(&cedar).unwrap().unwrap();
    assert_eq!(relayed.heard_at(&cedar, "one"), Some(cedar_heard));
    assert!(round().await.pushed.is_empty(), "nothing changed, nothing is sent");
    std::thread::sleep(Duration::from_millis(2));
    report(&host, &cedar, "one");
    assert_eq!(round().await.pushed.get(RECEIVER), Some(&1), "only the seat heard again");
    let (_, again) = host.local_harness_newest(&cedar).unwrap().unwrap();
    assert_eq!(relayed.heard_at(&cedar, "one"), Some(again));
    // The receiver restarts: a new daemon.started, and nothing it was pushed.
    advertise(&receiver_member, &host, true);
    assert_eq!(round().await.pushed.get(RECEIVER), Some(&2), "a new start gets every seat");
}

/// A push can reach a member before the seat's declaration does: the member records nothing and
/// says so, the seat stays owed, and the host's next round delivers it once the declaration has.
#[tokio::test]
async fn a_seat_pushed_before_its_declaration_arrives_is_delivered_after_it_does() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    report(&host, &cedar, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    advertise(&Store::open_memory(RECEIVER).unwrap(), &host, true);
    let url = worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, fractal_routes(crate::api::raw_terminal_owner_splice)).await;
    let relay = relay(root.path(), &[(RECEIVER, url)]);
    let members = [HOST.to_owned(), RECEIVER.to_owned()];
    let mut pushed = SeatFreshnessPushed::default();
    let mut round = async || push_seat_freshness(&host, &relay, HOST, &members, &mut pushed, Duration::from_secs(10)).await.unwrap();

    let early = round().await;
    assert!(early.pushed.is_empty() && early.failed.is_empty(), "nothing recorded, and nothing failed");
    assert_eq!(receiver.extension::<RelayedFreshness>().heard_at(&cedar, "one"), None);
    // The declaration replicates; the host has heard nothing new.
    receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
    assert_eq!(round().await.pushed.get(RECEIVER), Some(&1), "the seat was still owed");
    let (_, heard) = host.local_harness_newest(&cedar).unwrap().unwrap();
    assert_eq!(receiver.extension::<RelayedFreshness>().heard_at(&cedar, "one"), Some(heard));
    assert!(round().await.pushed.is_empty(), "and now it is delivered");
}

/// The same ordering through the running pusher, with nothing driven by hand: a seat the member
/// left unrecorded stays owed, so the pusher retries on its own and delivers it once the
/// declaration arrives, though the host commits nothing more.
#[tokio::test]
async fn the_pusher_retries_an_owed_seat_until_its_declaration_arrives() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    report(&host, &cedar, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    advertise(&Store::open_memory(RECEIVER).unwrap(), &host, true);
    let url = worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, fractal_routes(crate::api::raw_terminal_owner_splice)).await;
    let (_, heard) = host.local_harness_newest(&cedar).unwrap().unwrap();
    let index = host.index().unwrap();
    crate::api::seat_freshness::spawn_seat_freshness_push(
        host.clone(),
        relay(root.path(), &[(RECEIVER, url)]),
        HOST.into(),
        crate::api::seat_freshness::SeatFreshnessPace {
            coalesce: Duration::from_millis(20),
            answer: Duration::from_secs(5),
            retry: Duration::from_millis(100),
            longest_retry: Duration::from_millis(200),
        },
    );
    let relayed = receiver.extension::<RelayedFreshness>();
    // The pusher's first round reaches the member before the declaration does.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(relayed.heard_at(&cedar, "one"), None);
    receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while relayed.heard_at(&cedar, "one").is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(relayed.heard_at(&cedar, "one"), Some(heard), "the owed seat arrived with no new commit");
    assert_eq!(host.index().unwrap(), index, "the host committed nothing meanwhile");
}

/// Pushes go to every member at once: members that never answer cost one timeout for the round,
/// not one each, and a healthy member is pushed in the same round.
#[tokio::test]
async fn members_that_never_answer_do_not_hold_back_a_healthy_one() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    report(&host, &cedar, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
    let mut peers = vec![(RECEIVER, worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, fractal_routes(crate::api::raw_terminal_owner_splice)).await)];
    let silent = ["silent-1", "silent-2", "silent-3", "silent-4"];
    for name in silent {
        peers.push((name, hung().await));
        advertise(&Store::open_memory(name).unwrap(), &host, true);
    }
    advertise(&Store::open_memory(RECEIVER).unwrap(), &host, true);
    let relay = relay(root.path(), &peers);
    let members = peers.iter().map(|(name, _)| (*name).to_owned()).collect::<Vec<_>>();
    let timeout = Duration::from_secs(2);
    let started = Instant::now();
    let round = push_seat_freshness(&host, &relay, HOST, &members, &mut SeatFreshnessPushed::default(), timeout).await.unwrap();
    assert!(started.elapsed() < timeout * 2, "one timeout for the round, not one per member: {:?}", started.elapsed());
    assert_eq!(round.pushed.get(RECEIVER), Some(&1));
    assert_eq!(round.failed, silent.iter().map(|name| (*name).to_owned()).collect());
    assert!(receiver.extension::<RelayedFreshness>().heard_at(&cedar, "one").is_some());
}

/// A push without the fleet's signature is refused before the receiver's API is asked.
#[tokio::test]
async fn an_unsigned_seat_freshness_push_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let receiver = Arc::new(Store::open_memory(RECEIVER).unwrap());
    let socket = serve(root.path(), app(receiver, RECEIVER, root.path())).await;
    let peer = PeerState::new(
        MainBackend::new(socket),
        RECEIVER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from([HOST.into()])),
    );
    let body = serde_json::to_vec(&SeatFreshnessPush::default()).unwrap();
    let response = peer_router(peer, fractal_routes(crate::api::raw_terminal_owner_splice))
        .oneshot(Request::builder().method("POST").uri(SEAT_FRESHNESS_PATH).body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(to_bytes(response.into_body(), 4_096).await.is_ok());
}

/// Mixed versions, both ways. A member is pushed only once its own replicated `daemon.started`
/// advertises the route, so an older member is never sent it; a worker that predates the route
/// answers `None`, which is not a failure to retry.
#[tokio::test]
async fn an_older_member_is_never_pushed_and_an_older_worker_answers_none() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    report(&host, &cedar, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    let older_worker = worker(&serve(root.path(), app(receiver, RECEIVER, root.path())).await, axum::Router::new()).await;
    let relay = relay(root.path(), &[(RECEIVER, older_worker)]);
    let members = [HOST.to_owned(), RECEIVER.to_owned()];
    let receiver_member = Store::open_memory(RECEIVER).unwrap();
    let mut pushed = SeatFreshnessPushed::default();
    let mut round = async || push_seat_freshness(&host, &relay, HOST, &members, &mut pushed, Duration::from_secs(10)).await.unwrap();

    advertise(&receiver_member, &host, false);
    let older = round().await;
    assert!(older.pushed.is_empty() && older.failed.is_empty(), "an older member is not pushed");
    // An advertisement another member wrote does not count for it.
    append(&host, &format!("daemon/{RECEIVER}"), "daemon.started", None, json!({"status":"running", "features":{"seat_freshness":1}}));
    assert!(round().await.pushed.is_empty());
    // It advertises, but its worker predates the route.
    advertise(&receiver_member, &host, true);
    let ahead = round().await;
    assert!(ahead.pushed.is_empty() && ahead.failed.is_empty(), "no answer, and nothing to retry");
}

/// Re-stamps stay local only once every other member hears this node: each advertises the route
/// in its own `daemon.started` and a push from this node reached it, and this node is not a
/// dial-out member nobody can dial.
#[test]
fn the_fleet_hears_a_node_only_when_every_other_member_can_be_pushed_to() {
    let node = Store::open_memory("conversation-gateway").unwrap();
    let sources = [("alder", Store::open_memory("alder").unwrap()), ("birch", Store::open_memory("birch").unwrap())];
    let advertise = |member: &str, feature: bool| {
        let source = &sources.iter().find(|(name, _)| *name == member).unwrap().1;
        let features = if feature { json!({"seat_freshness":1}) } else { json!({"schedule_standby":1}) };
        append(source, &format!("daemon/{member}"), "daemon.started", None, json!({"status":"running", "features":features}));
        node.import_replication(member, &source.export_replication(0).unwrap()).unwrap();
    };
    let members = ["conversation-gateway".to_owned(), "alder".to_owned(), "birch".to_owned()];
    let reached = |alder: Option<bool>, birch: Option<bool>| {
        [("alder", alder), ("birch", birch)]
            .into_iter()
            .filter_map(|(name, reached)| reached.map(|reached| (name.to_owned(), reached)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let hears = |dial_out, reached: &std::collections::BTreeMap<String, bool>| {
        crate::api::seat_freshness::fleet_hears_seats(&node, "conversation-gateway", &members, dial_out, reached).unwrap()
    };
    advertise("alder", true);
    advertise("birch", false);
    assert!(!hears(false, &reached(Some(true), Some(true))), "birch runs an older build and reads re-stamp ages");
    advertise("birch", true);
    assert!(hears(false, &reached(Some(true), Some(true))), "every other member advertises and was reached");
    assert!(!hears(false, &reached(Some(true), Some(false))), "birch advertises but no push reaches it");
    assert!(!hears(false, &reached(Some(true), None)), "birch was never reached");
    assert!(!hears(true, &reached(Some(true), Some(true))), "nobody can dial a dial-out member to ask it");
}

/// Through the running pusher: a member that advertises the route but cannot be pushed to (its
/// worker lacks it) keeps this node's re-stamps replicating; one the push reaches lets them stay
/// local.
#[tokio::test]
async fn the_pusher_keeps_re_stamps_replicating_for_a_member_it_cannot_reach() {
    for (routes, expected) in [
        (axum::Router::new(), false),
        (fractal_routes(crate::api::raw_terminal_owner_splice), true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
        let cedar = declare(&host, "cedar", HOST);
        report(&host, &cedar, "one");
        let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
        receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
        advertise(&Store::open_memory(RECEIVER).unwrap(), &host, true);
        let url = worker(&serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await, routes).await;
        host.set_fleet_hears_seats(!expected).unwrap();
        crate::api::seat_freshness::spawn_seat_freshness_push(
            host.clone(),
            relay(root.path(), &[(RECEIVER, url)]),
            HOST.into(),
            crate::api::seat_freshness::SeatFreshnessPace {
                coalesce: Duration::from_millis(20),
                answer: Duration::from_secs(5),
                retry: Duration::from_millis(100),
                longest_retry: Duration::from_millis(200),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while host.fleet_hears_seats().unwrap() != expected && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(host.fleet_hears_seats().unwrap(), expected, "a worker that serves the route: {expected}");
    }
}

/// A member stops counting the moment a push to it fails, though it was reached before: its
/// re-stamps replicate again until a push reaches it again.
#[tokio::test]
async fn a_member_whose_pushes_start_failing_gets_replicated_re_stamps_again() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Store::open(&root.path().join("host.db"), HOST).unwrap());
    let cedar = declare(&host, "cedar", HOST);
    report(&host, &cedar, "one");
    let receiver = Arc::new(Store::open(&root.path().join("receiver.db"), RECEIVER).unwrap());
    receiver.import_replication(HOST, &host.export_replication(0).unwrap()).unwrap();
    advertise(&Store::open_memory(RECEIVER).unwrap(), &host, true);
    let socket = serve(root.path(), app(receiver.clone(), RECEIVER, root.path())).await;
    let url = worker(&socket, fractal_routes(crate::api::raw_terminal_owner_splice)).await;
    crate::api::seat_freshness::spawn_seat_freshness_push(
        host.clone(),
        relay(root.path(), &[(RECEIVER, url)]),
        HOST.into(),
        crate::api::seat_freshness::SeatFreshnessPace {
            coalesce: Duration::from_millis(20),
            answer: Duration::from_secs(5),
            retry: Duration::from_millis(100),
            longest_retry: Duration::from_millis(200),
        },
    );
    let wait_for = |expected: bool| {
        let host = host.clone();
        async move {
            let deadline = Instant::now() + Duration::from_secs(10);
            while host.fleet_hears_seats().unwrap() != expected && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            host.fleet_hears_seats().unwrap()
        }
    };
    assert!(wait_for(true).await, "the first push reached the member");
    // The member's daemon goes away: its worker answers each push with an error.
    fs::remove_file(&socket).unwrap();
    std::thread::sleep(Duration::from_millis(2));
    report(&host, &cedar, "one");
    assert!(!wait_for(false).await, "a failed push stops the member counting");
}
