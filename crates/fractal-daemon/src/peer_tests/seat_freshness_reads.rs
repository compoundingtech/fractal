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
use crate::AdhocWorkStore as _;
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
            status: Some("read"),
            latest: answered.get(&message.subject).and_then(|relayed| match relayed {
                Relayed::Answered { latest, .. } => latest.clone(),
                Relayed::Unknown { .. } => None,
            }),
        })
    );
    assert!(
        matches!(answered.get(&message.subject), Some(Relayed::Answered { latest: Some((receipt, _)), .. }) if receipt.starts_with("offset-receipt/")),
        "the newest of the seat's marks: {answered:?}"
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

/// Doctor's late-mail count reads a relay host's seat's mail from that host, as the backlog does:
/// mail it read is not late, and mail it did not answer for is unknown, by host, never late.
#[tokio::test(flavor = "multi_thread")]
async fn doctors_late_mail_counts_a_relay_hosts_seat_by_its_answer() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, cedar, _birch) = owner(root.path()).await;
    let later = smallclaims::store::now_ms() + 20_000;
    let late = |state: crate::api::AppState, seat: String| {
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            crate::api::unread_current_seat_counts(&state, &BTreeSet::from([seat.as_str()]), later, Some(&runtime))
                .unwrap()
        })
    };
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let answered = late(
        app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), url))),
        cedar.clone(),
    )
    .await
    .unwrap();
    assert_eq!(answered, (0, 0, BTreeMap::new()), "the host read it");

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
    let unknown = late(silent.clone(), cedar.clone()).await.unwrap();
    assert_eq!(
        unknown,
        (0, 0, BTreeMap::from([(OWNER.to_owned(), 1)])),
        "an unknown is not counted as late"
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

/// The owner of [`owner`], with two more messages to `cedar`: one its seat closed and one it has
/// not had yet, each sent with an idempotency key.
fn more_mail(owner: &Store, cedar: &str) {
    for (subject, key) in [
        ("message/00000000000000c2", "send-closed"),
        ("message/00000000000000c3", "send-unread"),
    ] {
        owner
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "message.sent".into(),
                actor: Some("agent/sender".into()),
                fields: serde_json::from_value(
                    json!({"from": "agent/sender", "to": cedar, "content": "hello", "status": "sent"}),
                )
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(key.into()),
            })
            .unwrap();
    }
    for kind in ["message.delivered", "message.read", "message.closed"] {
        message_marks::record(owner, "message/00000000000000c2", kind, cedar).unwrap();
    }
}

/// The subjects and statuses of a list of message views, and who said each status.
fn statuses(messages: &Value) -> Vec<(String, String, Option<String>)> {
    messages
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            (
                message["subject"].as_str().unwrap().to_owned(),
                message["status"].as_str().unwrap().to_owned(),
                message["status_from"]["state"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// A message view and every list of another host's seat's mail read that host's answer: the read,
/// the lists and pages of open mail, which drop what the host closed, the send receipt and the
/// client's message resources, each saying in `status_from` which host answered. Mail whose
/// host keeps no receipts of its own (`birch`, declared elsewhere) has no `status_from`.
#[tokio::test(flavor = "multi_thread")]
async fn views_and_lists_read_a_relay_hosts_seat_from_it() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, cedar, birch) = owner(root.path()).await;
    more_mail(&owner, &cedar);
    send(&owner, "message/00000000000000b1", &birch);
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let state = app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), url)));
    let client = crate::client::Client::unix(&serve(root.path(), state.clone()).await);
    let answered = (OWNER.to_owned(), "answered".to_owned());

    let read: Value = client.get("/v1/messages/read/message/00000000000000c1").await.unwrap();
    assert_eq!(read["status"], "read", "{read}");
    assert_eq!(read["status_from"], json!({"host": OWNER, "state": "answered"}));

    let open: Value = client.get(&format!("/v1/messages?to={cedar}")).await.unwrap();
    assert_eq!(
        statuses(&open),
        [
            ("message/00000000000000c1".into(), "read".into(), Some(answered.1.clone())),
            ("message/00000000000000c3".into(), "sent".into(), Some(answered.1.clone())),
        ],
        "the host closed c2, so a list of open mail leaves it out"
    );
    let all: Value = client
        .get(&format!("/v1/messages?to={cedar}&include_closed=true"))
        .await
        .unwrap();
    assert!(
        statuses(&all).contains(&("message/00000000000000c2".into(), "closed".into(), Some(answered.1.clone()))),
        "{all}"
    );
    let page: Value = client
        .get(&format!("/v1/messages/page?to={cedar}&limit=2"))
        .await
        .unwrap();
    let paged = statuses(&page["items"]);
    assert!(
        paged.iter().all(|(subject, status, from)| subject != "message/00000000000000c2"
            && status != "closed"
            && from.as_deref() == Some("answered")),
        "{page}"
    );
    let mut seen = paged.into_iter().map(|(subject, ..)| subject).collect::<Vec<_>>();
    if let Some(cursor) = page["next_cursor"].as_str() {
        let next: Value = client
            .get(&format!("/v1/messages/page?to={cedar}&limit=2&cursor={cursor}"))
            .await
            .unwrap();
        seen.extend(statuses(&next["items"]).into_iter().map(|(subject, ..)| subject));
    }
    seen.sort();
    assert_eq!(seen, ["message/00000000000000c1", "message/00000000000000c3"], "{page}");
    let sent: Value = client
        .get("/v1/messages/page?from=agent/sender&include_closed=true")
        .await
        .unwrap();
    let sent = statuses(&sent["items"]);
    assert!(
        sent.contains(&("message/00000000000000b1".into(), "sent".into(), None)),
        "a seat whose host keeps claims reads from them: {sent:?}"
    );
    assert!(sent.contains(&("message/00000000000000c2".into(), "closed".into(), Some(answered.1.clone()))));

    let receipt: Value = client.get("/v1/messages/by-key?key=send-closed").await.unwrap();
    assert_eq!(receipt["status"], "closed", "{receipt}");
    assert_eq!(receipt["status_from"]["host"], OWNER);

    // The client's message resources: state, status_from, the delivery the host answered, and
    // history for the mail the host closed.
    let mut resources = crate::api::client_message_resources(&viewer, None, true, None).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    crate::api::relay_client_message_items(&state, &mut resources, now, false).await.unwrap();
    let resource = |id: &str| resources.iter().find(|resource| resource["id"] == id).unwrap().clone();
    let read = resource("message/00000000000000c1");
    assert_eq!(read["state"], "read", "{read}");
    assert_eq!(read["status_from"], json!({"host": OWNER, "state": "answered"}));
    assert_eq!(read["delivery"]["phase"], "read", "{read}");
    assert_eq!(read["operational"]["layer"], "current");
    let local = crate::api::client_message_resources(&viewer, None, true, None).unwrap();
    let local = local.iter().find(|resource| resource["id"] == "message/00000000000000c1").unwrap();
    assert_eq!(
        (&read["revision"], &read["updated_at"]),
        (&local["revision"], &local["updated_at"]),
        "a host's answer never feeds the revision, which a client takes as monotonic: {read}"
    );
    let closed = resource("message/00000000000000c2");
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["operational"], json!({"layer": "history", "actionable": false, "reasons": ["closed"]}));
    assert!(resource("message/00000000000000b1").get("status_from").is_none());
    let mut current = crate::api::client_message_resources(&viewer, None, false, None).unwrap();
    crate::api::relay_client_message_items(&state, &mut current, now, true).await.unwrap();
    assert!(
        current.iter().all(|resource| resource["id"] != "message/00000000000000c2"),
        "current messages leave out what the host closed"
    );

    // A paging client's current pages, one message each: once the reads above have recorded the
    // host's close, the selector leaves it out, so no page comes back empty and the walk reaches
    // every other message once (#4866).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !crate::store::relayed_closes::closed(&viewer.readers.get(), "message/00000000000000c2").unwrap() {
        assert!(Instant::now() < deadline, "the background writer never recorded the host's close");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    while viewer.maintain_client_message_selectors().unwrap() {}
    let mut query = crate::api::ClientListQuery { limit: Some(1), ..Default::default() };
    let mut snapshot = crate::api::new_client_snapshot(&state);
    let (mut walked, mut empty) = (Vec::new(), 0);
    for _ in 0..10 {
        let (axum::Extension(next), axum::Json(page)) =
            crate::api::client_reads::client_messages_sql_page_for_tests(&state, snapshot.clone(), &query)
                .await
                .unwrap();
        snapshot = next;
        walked.extend(page.items.iter().map(|item| item["id"].as_str().unwrap().to_owned()));
        empty += usize::from(page.items.is_empty() && page.page.next_cursor.is_some());
        match page.page.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => break,
        }
    }
    walked.sort();
    assert_eq!(
        walked,
        ["message/00000000000000b1", "message/00000000000000c1", "message/00000000000000c3"]
    );
    assert_eq!(empty, 0, "the selector counts the host's close, so it leaves no empty page");
}

/// A view does not wait for a host that does not answer: it waits a second, then shows this
/// member's own records with `status_from` unknown and drops nothing, and for half a minute it does
/// not ask that host again.
#[tokio::test(flavor = "multi_thread")]
async fn a_view_waits_a_second_for_a_silent_host_and_then_does_not_ask_it() {
    let root = tempfile::tempdir().unwrap();
    let (owner, _url, cedar, _birch) = owner(root.path()).await;
    more_mail(&owner, &cedar);
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let state = app(viewer, VIEWER, root.path(), Some(relay(root.path(), hung().await)));
    let client = crate::client::Client::unix(&serve(root.path(), state).await);

    let started = Instant::now();
    let read: Value = client.get("/v1/messages/read/message/00000000000000c1").await.unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= message_reads::VIEW_DEADLINE && waited < Duration::from_secs(3),
        "{waited:?}"
    );
    assert_eq!(read["status"], "sent", "only this member's own records: {read}");
    assert_eq!(read["status_from"]["host"], OWNER);
    assert_eq!(read["status_from"]["state"], "unknown");
    assert!(read["status_from"]["reason"].as_str().unwrap().contains(OWNER), "{read}");

    let started = Instant::now();
    let open: Value = client.get(&format!("/v1/messages?to={cedar}")).await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
    assert_eq!(
        statuses(&open).into_iter().map(|(subject, _, from)| (subject, from)).collect::<Vec<_>>(),
        ["c1", "c2", "c3"].map(|name| (format!("message/00000000000000{name}"), Some("unknown".to_owned()))),
        "an unknown message stays open"
    );
}

/// A fault's target states and a work wake read another host's seat from it, at serve time: a
/// fault about mail the host's seat read shows it read since the host's mark, and a wake whose
/// message the seat read there is consumed, or delivered while the seat works in the wake's
/// incarnation. A claim still wins, and mail whose host keeps no receipts (`birch`) stays as this
/// member's records say.
#[tokio::test(flavor = "multi_thread")]
async fn a_faults_targets_and_a_wake_read_a_relay_hosts_seat_from_it() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, cedar, birch) = owner(root.path()).await;
    send(&owner, "message/00000000000000b1", &birch);
    send(&owner, "message/00000000000000d1", &cedar);
    message_marks::record(&owner, "message/00000000000000d1", "message.delivered", &cedar).unwrap();
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    let state = app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), url)));

    let mut items = vec![json!({"id": "attention/fault", "target_states": [
        {"id": "message/00000000000000c1", "state": "sent", "since": "2026-01-01T00:00:00Z"},
        {"id": "message/00000000000000b1", "state": "sent"},
        {"id": "mission-run/example", "state": "running"},
    ]})];
    crate::api::relay_attention_target_states(&state, &mut items).await.unwrap();
    let states = &items[0]["target_states"];
    assert_eq!(states[0]["state"], "read", "{states}");
    assert_ne!(states[0]["since"], "2026-01-01T00:00:00Z", "the host's mark is its since: {states}");
    assert_eq!(states[1], json!({"id": "message/00000000000000b1", "state": "sent"}));
    assert_eq!(states[2], json!({"id": "mission-run/example", "state": "running"}));

    let worker = declare(&viewer, "viewer-seat", VIEWER);
    let step = viewer
        .start_work(&crate::model::WorkStartRequest {
            actor: worker,
            title: "Inspect a fixture".into(),
            idempotency_key: "wake-step".into(),
        })
        .unwrap();
    let wake = |message: &str, acknowledged_by: Option<&str>, delivery_acknowledges: bool| {
        let mut step = step.clone();
        step.wake = Some(crate::model::WorkWakeView {
            assignee: cedar.clone(),
            assignee_state: "working".into(),
            incarnation_id: "incarnation".into(),
            attempts: 1,
            last_attempt_at_unix_ms: None,
            acknowledged_by: acknowledged_by.map(str::to_owned),
            failure: None,
            asks: crate::model::WakeAsks { messages: vec![message.into()], delivery_acknowledges, staging_acknowledges: false },
        });
        step
    };
    let mut steps = [
        wake("message/00000000000000c1", None, false),
        wake("message/00000000000000c1", Some("turn"), true),
        wake("message/00000000000000c1", Some("claim"), true),
        wake("message/00000000000000d1", None, true),
        wake("message/00000000000000d1", None, false),
        wake("message/00000000000000b1", Some("turn"), true),
    ];
    steps[5].wake.as_mut().unwrap().assignee = birch.clone();
    crate::api::message_reads::read_work_wakes(&state, steps.iter_mut()).await.unwrap();
    assert_eq!(
        steps.iter().map(|step| step.wake.as_ref().unwrap().acknowledged_by.as_deref()).collect::<Vec<_>>(),
        [Some("consumed"), Some("consumed"), Some("claim"), Some("delivery"), None, Some("turn")]
    );
}

/// A reminder group of two to `cedar`, `message/00000000000000e1` (version 1) and its newer
/// `message/00000000000000e2` (version 2), whose winner `e2` the seat closed on its host.
fn reminders(owner: &Store, cedar: &str) {
    for (subject, version) in [("message/00000000000000e1", 1), ("message/00000000000000e2", 2)] {
        append(
            owner,
            subject,
            "message.sent",
            Some("agent/sender"),
            json!({
                "from": "agent/sender", "to": cedar, "content": "stand-up", "status": "sent",
                "tags": ["reminder:standup", format!("version:{version}")],
            }),
        );
    }
    for kind in ["message.delivered", "message.read", "message.closed"] {
        message_marks::record(owner, "message/00000000000000e2", kind, cedar).unwrap();
    }
}

/// The ids and layers of a paging client's first page of current or all messages.
async fn client_page(state: &crate::api::AppState, history: bool) -> Vec<(String, Value)> {
    let query = crate::api::ClientListQuery { history, limit: Some(50), ..Default::default() };
    let (_, axum::Json(page)) = crate::api::client_reads::client_messages_sql_page_for_tests(
        state,
        crate::api::new_client_snapshot(state),
        &query,
    )
    .await
    .unwrap();
    page.items
        .iter()
        .map(|item| (item["id"].as_str().unwrap().to_owned(), item["operational"].clone()))
        .collect()
}

/// Two members of this build: the seat's host closed a reminder's winner as a mark, which never
/// replicates. The other member's read hears the close from the host and queues it without waiting
/// for its writer, held here for the whole read; that read leaves the closed winner out of current
/// mail, as before. Once the close is recorded, the older reminder is current in the paging and
/// the non-paging lists, the closed one is history even with the host silent, and no revision moves
/// (#4866).
#[tokio::test(flavor = "multi_thread")]
async fn a_hosts_close_of_a_reminders_winner_makes_the_next_one_current_on_another_member() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, cedar, _birch) = owner(root.path()).await;
    reminders(&owner, &cedar);
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    while viewer.maintain_client_message_selectors().unwrap() {}
    let state = app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), url)));
    let (winner, older) = ("message/00000000000000e2", "message/00000000000000e1");
    let ids = |page: &[(String, Value)]| page.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let before = crate::api::client_message_resources(&viewer, None, true, None).unwrap();

    // The first read, with this member's writer held throughout.
    let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let holder = {
        let viewer = viewer.clone();
        let (held, release) = (held.0, release.1);
        std::thread::spawn(move || {
            let _writer = viewer.connection.write();
            held.send(()).unwrap();
            release.recv().unwrap();
        })
    };
    held.1.recv().unwrap();
    let started = Instant::now();
    let first = tokio::time::timeout(Duration::from_secs(5), client_page(&state, false))
        .await
        .expect("a read waited for the writer");
    assert!(started.elapsed() < message_reads::VIEW_DEADLINE * 2, "{:?}", started.elapsed());
    assert!(
        !ids(&first).contains(&winner.to_owned()) && !ids(&first).contains(&older.to_owned()),
        "the first read leaves the closed winner out, and the older one is still superseded: {first:?}"
    );
    assert!(!crate::store::relayed_closes::closed(&viewer.readers.get(), winner).unwrap());
    release.0.send(()).unwrap();
    holder.join().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !crate::store::relayed_closes::closed(&viewer.readers.get(), winner).unwrap() {
        assert!(Instant::now() < deadline, "the background writer never recorded the host's close");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        owner.claims_for(winner, Some("message.closed")).unwrap().is_empty()
            && viewer.claims_for(winner, Some("message.closed")).unwrap().is_empty(),
        "the host's close is a mark, and nothing replicated it"
    );

    // The next read: the older reminder is current, and the winner the host closed is history.
    let current = client_page(&state, false).await;
    assert!(ids(&current).contains(&older.to_owned()), "{current:?}");
    assert!(!ids(&current).contains(&winner.to_owned()), "{current:?}");
    let all = client_page(&state, true).await;
    let layer = |page: &[(String, Value)], id: &str| page.iter().find(|(item, _)| item == id).unwrap().1.clone();
    assert_eq!(layer(&all, older), json!({"layer": "current", "actionable": true, "reasons": []}));
    assert_eq!(layer(&all, winner), json!({"layer": "history", "actionable": false, "reasons": ["closed"]}));
    // The non-paging client's lists pick the same winner.
    let listed = crate::api::client_message_resources(&viewer, None, false, None).unwrap();
    assert_eq!(
        listed.iter().filter(|item| item["id"] == older || item["id"] == winner).map(|item| item["id"].clone()).collect::<Vec<_>>(),
        [json!(older)]
    );
    // No revision moves: a relayed close never feeds one.
    let after = crate::api::client_message_resources(&viewer, None, true, None).unwrap();
    let revision = |items: &[Value], id: &str| {
        let item = items.iter().find(|item| item["id"] == id).unwrap();
        (item["revision"].clone(), item["updated_at"].clone())
    };
    for id in [winner, older] {
        assert_eq!(revision(&after, id), revision(&before, id), "{id}");
    }
    let unpatched = after.iter().find(|item| item["id"] == winner).unwrap();
    assert_eq!(unpatched["operational"]["reasons"], json!(["closed"]), "{unpatched}");

    // With the host silent, once its kept answer has lapsed, the close still counts, and the
    // message's state is this member's own records.
    tokio::time::sleep(Duration::from_millis(5_200)).await;
    let silent = app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), hung().await)));
    let all = client_page(&silent, true).await;
    assert_eq!(layer(&all, winner), json!({"layer": "history", "actionable": false, "reasons": ["closed"]}));
    let current = client_page(&silent, false).await;
    assert!(ids(&current).contains(&older.to_owned()) && !ids(&current).contains(&winner.to_owned()), "{current:?}");
    let resource = crate::api::client_message_resources(&viewer, None, true, None).unwrap();
    let resource = resource.iter().find(|item| item["id"] == winner).unwrap();
    assert_eq!(resource["state"], "sent", "this member's own records: {resource}");
}

/// A staging the host records as a mark reads as staged on another member, below every mark: in
/// the message's status and delivery, in a work wake whose driver's staging is its delivery, and
/// in doctor's held mail, where held mail the host's seat took any step with goes, as it does from
/// the host's own held flag. Mail the host did not stage stays held.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_hosts_staging_mark_reads_as_staged() {
    let root = tempfile::tempdir().unwrap();
    let (owner, url, cedar, _birch) = owner(root.path()).await;
    for subject in ["message/00000000000000e1", "message/00000000000000e2"] {
        append(
            &owner,
            subject,
            "message.sent",
            Some("agent/sender"),
            json!({"from": "agent/sender", "to": cedar, "content": "hello", "status": "sent",
                   "tags": [crate::silent::SILENT_TAG]}),
        );
    }
    message_marks::record(&owner, "message/00000000000000e1", message_marks::STAGED, &cedar).unwrap();
    let viewer = Arc::new(Store::open(&root.path().join("viewer.db"), VIEWER).unwrap());
    viewer
        .import_replication(OWNER, &owner.export_replication(0).unwrap())
        .unwrap();
    // Held mail is indexed as the replicated mail is projected.
    viewer.project_replication_backlog().unwrap();
    let state = app(viewer.clone(), VIEWER, root.path(), Some(relay(root.path(), url)));
    let staged = BTreeMap::from([(OWNER.to_owned(), vec!["message/00000000000000e1".to_owned()])]);
    let answered = message_reads::read(&state, staged, message_reads::STATUS_DEADLINE).await;
    assert_eq!(
        answered.get("message/00000000000000e1").and_then(Relayed::status),
        Some("staged"),
        "{answered:?}"
    );
    let socket = serve(root.path(), state.clone()).await;
    let status: Value = crate::client::Client::unix(&socket)
        .get("/v1/messages/delivery/message/00000000000000e1")
        .await
        .unwrap();
    assert_eq!(status["delivery"]["phase"], "staged", "{status}");

    let worker = declare(&viewer, "viewer-seat", VIEWER);
    let step = viewer
        .start_work(&crate::model::WorkStartRequest {
            actor: worker,
            title: "Inspect a fixture".into(),
            idempotency_key: "staged-wake-step".into(),
        })
        .unwrap();
    let wake = |staging_acknowledges: bool| {
        let mut step = step.clone();
        step.wake = Some(crate::model::WorkWakeView {
            assignee: cedar.clone(),
            assignee_state: "working".into(),
            incarnation_id: "incarnation".into(),
            attempts: 1,
            last_attempt_at_unix_ms: None,
            acknowledged_by: None,
            failure: None,
            asks: crate::model::WakeAsks {
                messages: vec!["message/00000000000000e1".into()],
                delivery_acknowledges: true,
                staging_acknowledges,
            },
        });
        step
    };
    let mut steps = [wake(true), wake(false)];
    crate::api::message_reads::read_work_wakes(&state, steps.iter_mut()).await.unwrap();
    assert_eq!(
        steps.iter().map(|step| step.wake.as_ref().unwrap().acknowledged_by.as_deref()).collect::<Vec<_>>(),
        [Some("delivery"), None],
        "a staging is a delivery only for a driver whose staging is its delivery"
    );

    let later = smallclaims::store::now_ms() + 60_000;
    let runtime = tokio::runtime::Handle::current();
    let seats = vec![(cedar.clone(), 2, 0)];
    let held = tokio::task::spawn_blocking(move || {
        crate::api::relayed_held_mail(&state, seats, later, Some(&runtime)).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        held.iter().map(|(seat, count, _)| (seat.as_str(), *count)).collect::<Vec<_>>(),
        [(cedar.as_str(), 1)],
        "the staged message is offered; the other is still held"
    );
}
