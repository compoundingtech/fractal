//! Real subprocess/kernel-connector seam; physical terminal stats are fixture data, not OMP.
use super::*;
use fractal_schema::harness_questions::{Question, native_id_digest};
use std::io::{BufRead as _, Write as _};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

const TEST: &str = "api::mailbox::questions::lifecycle_tests::native_session_lifetimes_survive_transport_and_end_with_the_driver";
const SOCKET: &str = "ST_TEST_QUESTION_LIFECYCLE_SOCKET";

fn open() -> Request {
    Request { agent: "agent/example/question".into(), runtime_incarnation: "runtime-one".into(),
        event: Observation::Open { session: "native-session".into(),
            native_id: native_id_digest("PRIVATE_NATIVE_CALL"),
            questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }] } }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn native_session_lifetimes_survive_transport_and_end_with_the_driver() {
    if let Some(path) = std::env::var_os(SOCKET) {
        let mut socket = std::os::unix::net::UnixStream::connect(path).unwrap();
        let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
        socket.write_all(b"ready\n").unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            match line.trim() {
                "close" | "answer" => {
                    let status = if line.trim() == "answer" { CloseStatus::AnsweredElsewhere }
                        else { CloseStatus::Cancelled };
                    let event = Observation::Closed { session: "native-session".into(),
                        native_id: native_id_digest("PRIVATE_NATIVE_CALL"), status };
                    writeln!(socket, "{}", serde_json::to_string(&event).unwrap()).unwrap();
                }
                "stop" => break,
                _ => panic!("unexpected lifecycle control"),
            }
        }
        return;
    }
    for scenario in ["switch", "shutdown", "lost-switch-close", "lost-shutdown-close", "abrupt", "terminal-wins"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peer.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture", "driver", "omp"])
            .env(SOCKET, &path).env("ST_AGENT", "agent/example/question")
            .env("ST_OMP_WEB_ANSWERS", "1").spawn().unwrap();
        let (socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await.unwrap().unwrap();
        let connector = crate::api::local_peer_process(&socket).unwrap();
        assert_eq!(connector.pid(), child.id());
        connector.validate().unwrap();
        let mut control = tokio::io::BufReader::new(socket);
        let mut ready = String::new();
        control.read_line(&mut ready).await.unwrap();
        assert_eq!(ready, "ready\n");
        let database = root.path().join("claims.sqlite3");
        let mut state = crate::api::tests_shared::state(root.path());
        state.store = Arc::new(Store::open(&database, "node").unwrap());
        let mut request = open();
        let dir = authority::source_agent_dir(&state,
            &Fence::new(&request.agent, &request.runtime_incarnation, "questions"));
        fractal_drivers::harness_events::enable(&dir, &request.runtime_incarnation).unwrap();
        let sequence = fractal_drivers::harness_state::claim(&dir, "example/question", "omp", "owner").unwrap();
        use fractal_drivers::harness_state::{Activity, BlockedOn, InputBuffer, Observation as ActivityObservation, Writer};
        let mut writer = Writer::new(&dir, "example/question", "omp", Some("question-runtime".into()))
            .with_ownership("owner", sequence);
        writer.observe(ActivityObservation::new(Activity::Active, BlockedOn::Human, InputBuffer::Unknown)).unwrap();
        let session_dir = dir.parent().unwrap().join("sessions").join("omp");
        std::fs::create_dir_all(&session_dir).unwrap();
        fractal_drivers::omp_session::record_channel_binding(
            &session_dir, "example/question", "question-runtime", "owner", "native-session", None, None,
        ).unwrap();
        fractal_drivers::omp_session::confirm_channel_binding(
            &session_dir, &dir, "example/question", "question-runtime", "owner", sequence, "native-session", None,
        ).unwrap();
        let birth = fractal_runtime::process_start_token;
        let generation = QuestionGeneration {
            question: subject(&request.agent, request.event.session(), request.event.native_id()),
            agent: request.agent.clone(), runtime_incarnation: request.runtime_incarnation.clone(),
            session: "native-session".into(), owner_session: "owner".into(), owner_sequence: sequence,
            witness: QuestionWitness { runtime_name: "question-runtime".into(), created_at: "fixture".into(),
                daemon: (std::process::id(), birth(std::process::id()).unwrap()),
                terminal: (connector.pid(), birth(connector.pid()).unwrap()),
                wrapper: (connector.pid(), birth(connector.pid()).unwrap()), bound: false },
        };
        apply(&state.store, &request, &generation).unwrap();
        let id = generation.question.clone();
        // Recover through a fresh Store handle: the owner lease is durable, not a daemon map.
        state.store = Arc::new(Store::open(&database, "node").unwrap());
        assert_eq!(state.store.harness_question_generation(&id).unwrap(), Some(generation.clone()));
        let index = state.store.index().unwrap();
        let reconcile = || fractal_drivers::harness_state::with_record_lock(&dir, || {
            super::super::question_lifecycle::reconcile_generation(&state.store, &generation, || {
                authority::question_generation_current(&state, &generation)
            })
        });
        // This fixture has no physical PTY stats endpoint. Unavailable proof is NOT loss.
        assert!(reconcile().is_err());
        assert_eq!(state.store.index().unwrap(), index);
        assert!(state.store.harness_question_held(&id).unwrap().unwrap().resolution.is_none());
        let mut exited = false;
        if matches!(scenario, "switch" | "shutdown" | "terminal-wins") {
            let command = if scenario == "terminal-wins" { b"answer\n".as_slice() } else { b"close\n".as_slice() };
            control.get_mut().write_all(command).await.unwrap();
            let mut line = String::new();
            control.read_line(&mut line).await.unwrap();
            request.event = serde_json::from_str(&line).unwrap();
            apply(&state.store, &request, &generation).unwrap();
        }
        if matches!(scenario, "switch" | "lost-switch-close") {
            // Pending successor alone proves the predecessor ended; readiness is not needed.
            fractal_drivers::omp_session::record_channel_binding(
                &session_dir, "example/question", "question-runtime", "owner", "successor-session", None, None,
            ).unwrap();
        } else if scenario == "lost-shutdown-close" {
            writer.observe(ActivityObservation::new(Activity::Ended, BlockedOn::None, InputBuffer::Unknown)
                .with_exit("clean-shutdown")).unwrap();
        } else if matches!(scenario, "abrupt" | "terminal-wins") {
            child.kill().unwrap();
            child.wait().unwrap();
            exited = true;
            assert!(connector.validate().is_err());
        }
        if scenario == "abrupt" {
            // Exercise the actual startup worker against the reopened database, not a
            // reader-only projection or a reconnect-triggered cleanup.
            let worker = super::super::question_lifecycle::start(&state);
            let retired = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if state.store.harness_question_held(&id).unwrap().unwrap().resolution.is_some()
                        && state.store.harness_question_generation(&id).unwrap().is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await;
            worker.abort();
            retired.unwrap();
        } else {
            let changed = reconcile().unwrap();
            assert_eq!(changed, matches!(scenario, "lost-switch-close" | "lost-shutdown-close"));
        }
        let closed = state.store.harness_question_held(&id).unwrap().unwrap().resolution.unwrap();
        assert_eq!(closed.status, if scenario == "terminal-wins" { Status::AnsweredElsewhere } else { Status::Cancelled });
        assert!(closed.answers.is_empty());
        assert!(closed.receipt.is_none());
        assert!(state.store.harness_question_generation(&id).unwrap().is_none());
        let index = state.store.index().unwrap();
        assert!(!reconcile().unwrap());
        assert_eq!(state.store.index().unwrap(), index);
        if !exited {
            control.get_mut().write_all(b"stop\n").await.unwrap();
        }
        // Child::wait retains the exit status, including when abrupt-loss setup reaped it.
        let status = child.wait().unwrap();
        if !exited { assert!(status.success()); }
    }
}

#[test]
fn orphan_registration_and_reconciliation_errors_never_invent_an_answer() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("claims.sqlite3"), "node").unwrap();
    let request = open();
    let generation = QuestionGeneration {
        question: subject(&request.agent, request.event.session(), request.event.native_id()),
        agent: request.agent.clone(), runtime_incarnation: request.runtime_incarnation.clone(),
        session: "native-session".into(), owner_session: "owner".into(), owner_sequence: 1,
        witness: QuestionWitness { runtime_name: "runtime".into(), created_at: "created".into(),
            daemon: (1, 1), terminal: (2, 2), wrapper: (2, 2), bound: false },
    };
    store.retain_harness_question_generation(&generation).unwrap();
    let index = store.index().unwrap();
    assert!(!super::super::question_lifecycle::reconcile_generation(&store, &generation, || panic!("no question yet")).unwrap());
    assert_eq!(store.index().unwrap(), index);
    apply(&store, &request, &generation).unwrap();
    let index = store.index().unwrap();
    assert!(super::super::question_lifecycle::reconcile_generation(&store, &generation,
        || anyhow::bail!("temporarily unavailable")).is_err());
    assert_eq!(store.index().unwrap(), index);
    assert!(store.harness_question_held(&generation.question).unwrap().unwrap().resolution.is_none());
    let mut foreign = generation.clone();
    foreign.witness.wrapper.1 += 1;
    assert!(store.retain_harness_question_generation(&foreign).is_err());
    assert!(!super::super::question_lifecycle::reconcile_generation(&store, &foreign,
        || panic!("foreign generation cannot reconcile")).unwrap());
    assert_eq!(store.index().unwrap(), index);
}

#[tokio::test]
async fn startup_retires_only_unleased_local_observations() {
    let root = tempfile::tempdir().unwrap();
    let mut state = crate::api::tests_shared::state(root.path());
    state.store = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "node").unwrap());
    let request = open();
    let dir = authority::source_agent_dir(&state,
        &Fence::new(&request.agent, &request.runtime_incarnation, "questions"));
    fractal_drivers::harness_events::enable(&dir, &request.runtime_incarnation).unwrap();
    let local = state.store.observe_harness_question_digest(&QuestionObservation {
        agent: request.agent.clone(), runtime_incarnation: request.runtime_incarnation.clone(),
        session: request.event.session().into(), native_id: request.event.native_id().into(),
        questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
    }).unwrap();
    let foreign = Store::open_memory("foreign").unwrap();
    let remote = foreign.observe_harness_question_digest(&QuestionObservation {
        agent: "agent/remote/question".into(), runtime_incarnation: "remote-runtime".into(),
        session: "remote-session".into(), native_id: native_id_digest("remote-call"),
        questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
    }).unwrap();
    const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    foreign.bind_fleet(FLEET).unwrap();
    state.store.bind_fleet(FLEET).unwrap();
    let exchange = foreign.export_replication_exchange(FLEET, &Default::default()).unwrap();
    state.store.receive_replication_exchange("foreign", FLEET, &exchange).unwrap();
    state.store.validate_replication_backlog().unwrap();
    state.store.apply_replication_repairs().unwrap();
    assert!(state.store.project_replication_backlog().unwrap());
    let worker = super::super::question_lifecycle::start(&state);
    let retired = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if state.store.harness_question_held(&local.id).unwrap().unwrap().resolution.is_some() { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    worker.abort();
    retired.unwrap();
    let resolution = state.store.harness_question_held(&local.id).unwrap().unwrap().resolution.unwrap();
    assert_eq!(resolution.status, Status::Cancelled);
    assert!(resolution.answers.is_empty());
    assert!(resolution.receipt.is_none());
    assert!(state.store.harness_question_held(&remote.id).unwrap().unwrap().resolution.is_none());
}
