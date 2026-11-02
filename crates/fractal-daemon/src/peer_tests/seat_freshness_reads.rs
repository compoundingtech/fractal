//! Reads a seat's host answers on the seat-freshness relay (db break-up step 4b): how far its
//! seats got with a message, asked by another member that keeps no receipt of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use smallclaims::sync::{FleetAuth, FleetContext, PeerState, peer_router};
use tokio::sync::watch;

use crate::api::message_reads::{self, Relayed};
use crate::config::{Config, PeerConfig};
use crate::model::ClaimInput;
use crate::store::Store;
use crate::store::message_marks;
use fractal_peer::peer::*;

/// The seats' host: it keeps their marks and answers reads.
const OWNER: &str = "conversation-owner";
/// The member that asks.
const VIEWER: &str = "conversation-gateway";

fn app(
    store: Arc<Store>,
    node: &str,
    root: &Path,
    client_relay: Option<ClientRelay>,
) -> crate::api::AppState {
    crate::api::AppState {
        seat_admission: Default::default(),
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
        client_relay,
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
    let source = format!(
        "version 2\nagent \"{name}\" {{ host \"{host}\"; workspace \"/tmp\"; command \"true\" }}\n"
    );
    let intent = crate::parse_intent(&source, OWNER).unwrap();
    let mission = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source.clone(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &mission.subject_tokens, &format!("declare-{name}"))
        .unwrap();
    store
        .desired_subjects()
        .unwrap()
        .into_iter()
        .find(|desired| desired.subject.ends_with(name))
        .unwrap()
        .subject
}

fn send(store: &Store, subject: &str, to: &str) {
    append(
        store,
        subject,
        "message.sent",
        Some("agent/sender"),
        json!({"from": "agent/sender", "to": to, "content": "hello", "status": "sent"}),
    );
}

/// The owner advertises `features` in its own `daemon.started`.
fn advertise(owner: &Store, features: Value) {
    append(
        owner,
        &format!("daemon/{}", owner.origin()),
        "daemon.started",
        None,
        json!({"status": "running", "features": features}),
    );
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

/// The owner's worker for its API at `socket`; its URL.
async fn worker(socket: &Path) -> String {
    let peer = PeerState::new(
        MainBackend::new(socket.to_path_buf()),
        OWNER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from([VIEWER.into()])),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            peer_router(peer, fractal_routes(crate::api::raw_terminal_owner_splice)),
        )
        .await
    });
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

/// The viewer's relay, reaching the owner at `url`.
fn relay(root: &Path, url: String) -> ClientRelay {
    let secret = root.join("fleet-secret");
    fs::write(&secret, [7_u8; 32]).unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    ClientRelay::from_config(&Config {
        node: VIEWER.into(),
        fleet_id: Some("fleet-test".into()),
        shared_secret_file: Some(secret),
        peers: vec![PeerConfig {
            name: OWNER.into(),
            url,
        }],
        ..Default::default()
    })
    .unwrap()
    .unwrap()
}

/// The owner, with `cedar` on it and `birch` elsewhere, a message to each that both seats marked
/// read, and the owner's own API served behind its worker; the worker's URL.
async fn owner(root: &Path) -> (Arc<Store>, String, String, String) {
    let owner = Arc::new(Store::open(&root.join("owner.db"), OWNER).unwrap());
    let cedar = declare(&owner, "cedar", OWNER);
    let birch = declare(&owner, "birch", "elsewhere");
    send(&owner, "message/00000000000000c1", &cedar);
    for (seat, kind) in [
        (&cedar, "message.delivered"),
        (&cedar, "message.read"),
        (&birch, "message.read"),
    ] {
        message_marks::record(&owner, "message/00000000000000c1", kind, seat).unwrap();
    }
    advertise(
        &owner,
        json!({"seat_freshness": 1, "seat_freshness_reads": 1, "message_offsets": 1}),
    );
    let url = worker(&serve(root, app(owner.clone(), OWNER, root, None)).await).await;
    (owner, url, cedar, birch)
}

/// The owner answers, signed, every message asked, in order: whether it holds it and the marks of
/// the seats its declarations place on it, never another seat's.
#[tokio::test]
async fn a_host_answers_its_own_seats_marks_for_each_message_asked() {
    let root = tempfile::tempdir().unwrap();
    let (_owner, url, cedar, _birch) = owner(root.path()).await;
    let relay = relay(root.path(), url);
    let asked = vec![
        "message/00000000000000c1".to_owned(),
        "message/00000000000000ff".to_owned(),
    ];
    let answers = relay
        .read_message_marks(OWNER, &asked, Duration::from_secs(10))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answers.messages.len(), 2);
    let held = &answers.messages[0];
    assert!(held.held);
    let mut marks = held
        .marks
        .iter()
        .map(|mark| (mark.kind.as_str(), mark.consumer.as_str()))
        .collect::<Vec<_>>();
    marks.sort_unstable();
    assert_eq!(
        marks,
        [
            ("message.delivered", cedar.as_str()),
            ("message.read", cedar.as_str())
        ],
        "a host speaks only for its own seats"
    );
    assert!(
        held.marks
            .iter()
            .all(|mark| mark.receipt.starts_with("offset-receipt/"))
    );
    assert!(
        !answers.messages[1].held && answers.messages[1].marks.is_empty(),
        "a message it does not hold"
    );
    // A read is bounded and names messages only.
    let many = (0..=MAX_READ_MESSAGES)
        .map(|number| format!("message/{number:016x}"))
        .collect::<Vec<_>>();
    assert!(
        relay
            .read_message_marks(OWNER, &many, Duration::from_secs(10))
            .await
            .is_err()
    );
    assert!(
        relay
            .read_message_marks(OWNER, &["agent/cedar".to_owned()], Duration::from_secs(10))
            .await
            .is_err()
    );
}

/// A viewer reads a relay host's seat from that host: the further of its own state and the
/// host's answer. A host that does not answer reads as unknown within the deadline, never as
/// sent, and a host that does not keep its seats' receipts is read from claims, as before.
#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_reads_a_relay_hosts_seat_from_it_and_unknown_when_it_does_not_answer() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, _cedar, _birch) = owner(root.path()).await;
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let message = viewer.message("message/00000000000000c1").unwrap().unwrap();
    assert_eq!(
        message.status, "sent",
        "the viewer holds no receipt of its own"
    );
    assert_eq!(
        message_reads::relay_host(&viewer, &message)
            .unwrap()
            .as_deref(),
        Some(OWNER)
    );

    let state = app(
        viewer.clone(),
        VIEWER,
        root.path(),
        Some(relay(root.path(), url)),
    );
    let asks = BTreeMap::from([(OWNER.to_owned(), vec![message.subject.clone()])]);
    let answered = message_reads::read(&state, asks.clone(), message_reads::STATUS_DEADLINE).await;
    assert_eq!(
        answered.get(&message.subject),
        Some(&Relayed::Answered {
            host: OWNER.into(),
            held: true,
            status: Some("read")
        })
    );
    let socket = serve(root.path(), state.clone()).await;
    let client = crate::client::Client::unix(&socket);
    let status: Value = client
        .get(&format!("/v1/messages/delivery/{}", message.subject))
        .await
        .unwrap();
    assert_eq!(status["delivery"]["phase"], "read", "{status}");
    assert_eq!(status["delivery"]["state"], "read", "{status}");

    // The same host, unreachable from a viewer that has asked it nothing yet: unknown, in about
    // the deadline.
    let quiet = Arc::new(Store::open(&root.path().join("quiet.db"), VIEWER).unwrap());
    quiet
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let silent = app(
        quiet.clone(),
        "silent-viewer",
        root.path(),
        Some(relay(root.path(), hung().await)),
    );
    let started = Instant::now();
    let unknown = message_reads::read(&silent, asks.clone(), Duration::from_millis(500)).await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        matches!(unknown.get(&message.subject), Some(Relayed::Unknown { host, .. }) if host == OWNER)
    );
    // The next check a few seconds later does not wait for the silent host again.
    let started = Instant::now();
    let again = message_reads::read(&silent, asks, Duration::from_secs(2)).await;
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(
        matches!(again.get(&message.subject), Some(Relayed::Unknown { host, reason }) if host == OWNER && reason.contains("did not answer")),
        "{again:?}"
    );
    let socket = serve(root.path(), silent).await;
    let status: Value = crate::client::Client::unix(&socket)
        .get(&format!("/v1/messages/delivery/{}", message.subject))
        .await
        .unwrap();
    assert_eq!(status["delivery"]["phase"], "unknown", "{status}");
    assert!(
        status["delivery"]["reason"]
            .as_str()
            .unwrap()
            .contains(OWNER),
        "{status}"
    );

    // An answer is kept a few seconds: asking again does not ask the host.
    let kept = message_reads::read(
        &state,
        BTreeMap::from([(OWNER.to_owned(), vec![message.subject.clone()])]),
        Duration::from_millis(1),
    )
    .await;
    assert_eq!(
        kept.get(&message.subject).and_then(Relayed::status),
        Some("read")
    );

    // A host that stops advertising `message_offsets` keeps lifecycle claims: read from them.
    advertise(
        &owner,
        json!({"seat_freshness": 1, "seat_freshness_reads": 1}),
    );
    quiet
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(message_reads::relay_host(&quiet, &message).unwrap(), None);
    let status: Value = crate::client::Client::unix(&socket)
        .get(&format!("/v1/messages/delivery/{}", message.subject))
        .await
        .unwrap();
    assert_eq!(status["delivery"]["phase"], "sent", "{status}");
}

/// A host whose advertisement is ahead of its build answers no read: its worker returns the
/// unsigned 404 of a build without the route. The viewer reads that as unknown, naming the host,
/// and never as sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_whose_worker_lacks_the_route_reads_as_unknown() {
    let root = tempfile::tempdir().unwrap();
    let (owner, _url, _cedar, _birch) = owner(root.path()).await;
    let peer = PeerState::new(
        MainBackend::new(root.path().join("absent.sock")),
        OWNER.into(),
        FleetAuth::test("fleet-test", &[7; 32]),
        FleetContext::legacy(BTreeSet::from([VIEWER.into()])),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(
        async move { axum::serve(listener, peer_router(peer, axum::Router::new())).await },
    );
    let relay = relay(root.path(), format!("http://{address}"));
    let subject = "message/00000000000000c1".to_owned();
    assert_eq!(
        relay
            .read_message_marks(
                OWNER,
                std::slice::from_ref(&subject),
                Duration::from_secs(10)
            )
            .await
            .unwrap(),
        None
    );
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let state = app(viewer, VIEWER, root.path(), Some(relay));
    let read = message_reads::read(
        &state,
        BTreeMap::from([(OWNER.to_owned(), vec![subject.clone()])]),
        message_reads::STATUS_DEADLINE,
    )
    .await;
    assert!(
        matches!(read.get(&subject), Some(Relayed::Unknown { host, reason }) if host == OWNER && reason.contains("does not answer reads")),
        "{read:?}"
    );
}

/// The backlog counts a relay host's seat's unread mail by what its host answers: mail it read is
/// not counted, and mail it did not answer for is unknown, never counted as unread. A host that
/// keeps lifecycle claims is counted from them, as before.
#[tokio::test(flavor = "multi_thread")]
async fn the_mail_backlog_counts_a_relay_hosts_seat_by_its_answer() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, _cedar, _birch) = owner(root.path()).await;
    let later = smallclaims::store::now_ms() + 2 * 3_600_000;
    let backlog = |state: crate::api::AppState| {
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            crate::api::mail_backlog::report_with_unknown(&state, later, Some(&runtime)).unwrap()
        })
    };
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let (answered, unknown) = backlog(app(
        viewer.clone(),
        VIEWER,
        root.path(),
        Some(relay(root.path(), url)),
    ))
    .await
    .unwrap();
    assert_eq!((answered.count, unknown.len()), (0, 0), "the host read it");

    let quiet = Arc::new(Store::open(&root.path().join("quiet.db"), VIEWER).unwrap());
    quiet
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let silent = app(
        quiet.clone(),
        VIEWER,
        root.path(),
        Some(relay(root.path(), hung().await)),
    );
    let (counted, unknown) = backlog(silent.clone()).await.unwrap();
    assert_eq!(counted.count, 0, "an unknown is not counted as unread");
    assert_eq!(unknown, BTreeMap::from([(OWNER.to_owned(), 1)]));

    advertise(
        &owner,
        json!({"seat_freshness": 1, "seat_freshness_reads": 1}),
    );
    quiet
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let (claims, unknown) = backlog(silent).await.unwrap();
    assert_eq!(
        (claims.count, unknown.len()),
        (1, 0),
        "no read claim: unread, and nobody was asked"
    );
}

/// The host checks a read itself: an unsigned one is refused before its API is asked, and a
/// signed one it cannot answer (an unknown operation, too many messages, a subject that is not a
/// message) gets a signed 400 naming why, which the asker can tell from a missing route.
#[tokio::test]
async fn a_host_refuses_an_unsigned_read_and_signs_its_refusal_of_a_bad_one() {
    let root = tempfile::tempdir().unwrap();
    let (_owner, url, _cedar, _birch) = owner(root.path()).await;
    let http = reqwest::Client::new();
    let target = format!("{url}{SEAT_FRESHNESS_READS_PATH}");
    let unsigned = http
        .post(&target)
        .header("content-type", "application/json")
        .body(r#"{"op":"message_marks","messages":[]}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned.status(), reqwest::StatusCode::UNAUTHORIZED);
    let auth = FleetAuth::test("fleet-test", &[7; 32]);
    let many = json!({"op": "message_marks", "messages": (0..=MAX_READ_MESSAGES).map(|number| format!("message/{number:016x}")).collect::<Vec<_>>()});
    for body in [
        json!({"op": "consumer_marks", "consumer": "agent/cedar"}),
        many,
        json!({"op": "message_marks", "messages": ["agent/cedar"]}),
        json!({"op": "message_marks", "messages": [], "since": 0}),
    ] {
        let body = serde_json::to_vec(&body).unwrap();
        let response = http
            .post(&target)
            .headers(auth.request_headers_method("POST", SEAT_FRESHNESS_READS_PATH, VIEWER, &body).unwrap())
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&body));
        let headers = response.headers().clone();
        let answer = response.bytes().await.unwrap();
        auth.verify(&headers, "RESPONSE", SEAT_FRESHNESS_READS_PATH, &answer, Some(OWNER), Some(&FleetAuth::body_digest(&body)))
            .expect("the refusal is signed by the host");
    }
}

/// A batched read asks each host in requests of at most `MAX_READ_MESSAGES`, at most
/// `MAX_MESSAGES_PER_HOST` messages of one host and at most `MAX_HOSTS` hosts; whatever is past a
/// bound reads as unknown, saying which bound, and is never counted as unread.
#[tokio::test(flavor = "multi_thread")]
async fn a_batched_read_asks_each_host_in_bounded_requests() {
    let root = tempfile::tempdir().unwrap();
    let sizes = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let counted = sizes.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let routes = axum::Router::new().route(
        SEAT_FRESHNESS_READS_PATH,
        axum::routing::post(move |body: axum::body::Bytes| {
            let counted = counted.clone();
            async move {
                let read: SeatFreshnessRead = serde_json::from_slice(&body).unwrap();
                let SeatFreshnessRead::MessageMarks { messages } = read;
                counted.lock().unwrap().push(messages.len());
                axum::http::StatusCode::NOT_FOUND
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, routes).await });
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    let state = app(viewer, VIEWER, root.path(), Some(relay(root.path(), format!("http://{address}"))));
    let over = message_reads::MAX_MESSAGES_PER_HOST + 52;
    let mut asks = BTreeMap::from([(
        OWNER.to_owned(),
        (0..over).map(|number| format!("message/{number:016x}")).collect::<Vec<_>>(),
    )]);
    for host in 0..message_reads::MAX_HOSTS {
        asks.insert(format!("other-{host:02}"), vec![format!("message/f{host:015x}")]);
    }
    let read = message_reads::read(&state, asks, message_reads::STATUS_DEADLINE).await;

    assert_eq!(
        *sizes.lock().unwrap(),
        vec![MAX_READ_MESSAGES; message_reads::MAX_MESSAGES_PER_HOST.div_ceil(MAX_READ_MESSAGES)],
        "the host is asked in full requests, up to its bound"
    );
    assert_eq!(read.len(), over + message_reads::MAX_HOSTS, "every message asked has an answer");
    let reasons = read
        .values()
        .map(|relayed| match relayed {
            Relayed::Unknown { reason, .. } if reason.contains("at most") => reason.clone(),
            Relayed::Unknown { .. } => "no answer".to_owned(),
            other => panic!("{other:?}"),
        })
        .fold(BTreeMap::<String, usize>::new(), |mut counts, reason| {
            *counts.entry(reason).or_default() += 1;
            counts
        });
    assert_eq!(
        reasons,
        BTreeMap::from([
            ("no answer".to_owned(), message_reads::MAX_MESSAGES_PER_HOST + message_reads::MAX_HOSTS - 1),
            (format!("one read asks at most {} messages of a host", message_reads::MAX_MESSAGES_PER_HOST), 52),
            (format!("one read asks at most {} hosts", message_reads::MAX_HOSTS), 1),
        ])
    );
}
