use super::*;
use crate::api::{AppState, router, serve_unix_with_ready};
use crate::client::{Client, Endpoint};
use std::sync::atomic::AtomicBool;

struct PausedGateWriter {
    store: Arc<Store>,
    first: AtomicBool,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl FaultInjection for PausedGateWriter {
    fn fault(&self, scope: &str, _: &str) -> Option<String> {
        if scope == "gate-write" && !self.first.swap(true, Ordering::SeqCst) {
            self.store.hold_writer_for_test(|| {
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
            });
        }
        None
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partially_progressed_readiness_precedes_slow_gate_writes() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&root.path().join("state.sqlite3"), "node").unwrap());
    let source = r#"version 2
agent "worker" { workspace "/tmp"; command "true"; restart "never" }
resource "marker" { kind "custom.st3.document-source" }
mission "progressed" state="ready" {
  goal "Continue after a completed dependency."
  step "start" { agentless }
  step "next" { assigned-to "agent/node.worker"; depends-on { step "start" completed } }
  step "finish" { agentless; depends-on { step "next" completed } }
}
mission "gated" state="ready" {
  concurrent-runs
  goal "Retain older evaluations without delaying readiness."
  step "check" { agentless; gate "marker holds" { field "state" "resource/marker" is "ready" } }
}
mission "fresh" state="ready" {
  goal "Retain first-readiness priority."
  step "work" { assigned-to "agent/node.worker" }
}
"#;
    apply_source(&store, source, "priority-publish");
    store
        .append_claim(&ClaimInput {
            subject: "resource/marker".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("state".into(), Value::String("ready".into()))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("priority-marker".into()),
        })
        .unwrap();
    let start = |mission: &str, key: &str| {
        store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: root.path().display().to_string(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: key.into(),
            })
            .unwrap()
    };
    // The partially progressed run is older than every ready gate writer.
    let progressed = start("progressed", "priority-progressed");
    let first = progressed
        .steps
        .iter()
        .find(|step| step.step == "start")
        .unwrap();
    let next = progressed
        .steps
        .iter()
        .find(|step| step.step == "next")
        .unwrap();
    store
        .set_step_state(&first.subject, "completed", None)
        .unwrap();
    assert_eq!(
        store
            .step_run(&next.subject)
            .unwrap()
            .unwrap()
            .readiness_epoch,
        0
    );
    let mut gated = Vec::new();
    for index in 0..8 {
        let run = start("gated", &format!("priority-gate-{index}"));
        store
            .set_step_state(&run.steps[0].subject, "ready", None)
            .unwrap();
        gated.push(run);
    }
    let fresh = start("fresh", "priority-fresh");
    let notify = Arc::new(Notify::new());
    let state = AppState {
        store: store.clone(),
        notify: notify.clone(),
        event_notify: tokio::sync::watch::channel(0).0,
        node: "node".into(),
        state_dir: root.path().into(),
        pty_root: root.path().join("pty"),
        pty_binary: "unused-pty".into(),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
    };
    let socket = root.path().join("st3.sock");
    let server_socket = socket.clone();
    let (listening, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        serve_unix_with_ready(&server_socket, router(state), || {
            let _ = listening.send(());
        })
        .await
        .unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    let client = Client::new(Endpoint::Unix(socket));
    let (entered, paused) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        notify,
    )
    .with_fault_injection(Arc::new(PausedGateWriter {
        store: store.clone(),
        first: AtomicBool::new(false),
        entered,
        release: std::sync::Mutex::new(released),
    }));
    let evaluator =
        tokio::task::spawn_blocking(move || reconciler.evaluate_mission_runs().unwrap());
    tokio::task::spawn_blocking(move || paused.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    // Capture readiness at the actual gate-write boundary, without polling or a latency race.
    let observed = client
        .get::<Value>(&format!("/v1/client/work/{}", next.subject))
        .await;
    let first_ready = client
        .get::<Value>(&format!("/v1/client/work/{}", fresh.steps[0].subject))
        .await;
    release.send(()).unwrap();
    evaluator.await.unwrap();
    server.abort();
    let observed = observed.unwrap();
    let first_ready = first_ready.unwrap();
    assert_eq!(
        first_ready["state"], "ready",
        "first readiness keeps its priority"
    );
    assert_eq!(
        observed["state"], "ready",
        "satisfied dependent readiness must precede newer gate writers: {observed}"
    );
    assert_eq!(observed["readiness_epoch"], 1);
    for run in gated {
        assert_eq!(
            store.mission_run(&run.id).unwrap().unwrap().steps[0].status,
            "completed"
        );
    }
}
