#![cfg(unix)]
//! The driver exports its runtime fence before spawning a provider.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use fractal_daemon::api::AppState;
use fractal_daemon::model::{ClaimInput, PlannerSpec};
use fractal_daemon::store::Store;
use tokio::sync::{Notify, watch};

fn fixture_state(root: &std::path::Path, store: Arc<Store>) -> AppState {
    AppState {
        browser_pairing_login: None, seat_admission: Default::default(), owning_person: None,
        store, notify: Arc::new(Notify::new()), event_notify: watch::channel(0_u64).0,
        node: "orchid".into(), api_socket: root.join("api.sock"),
        state_dir: root.into(), pty_root: root.join("pty"), pty_binary: PathBuf::from("pty"),
        fleet_id: None, configured_peers: vec![], client_relay: None,
        native_session_home: None, planner_default: PlannerSpec::default(),
        harness_question_sink: None, assistant_receipts: Default::default(),
        private_notes: Default::default(),
    }
}

async fn provider_incarnation(
    inherited: Option<&str>,
    resume: bool,
    subject: &'static str,
    expected: &str,
) {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("api.sock");
    let store = Arc::new(Store::open_memory("orchid").unwrap());
    store
        .append_claim(&ClaimInput {
            subject: "agent/garden/worker".into(),
            kind: "runtime.observed".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("status".into(), json!("running")),
                ("incarnation_id".into(), json!("worker:current")),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let state = fixture_state(root.path(), store);
    let listener_socket = socket.clone();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix(&listener_socket, fractal_daemon::api::router(state)).await
    });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let resume_path =
        resume.then(|| {
            fractal_drivers::reexec::write_state(root.path(), "driver", &json!({
        "driver":"exec", "subject":"agent/garden/worker", "incarnation":"worker:current",
        "session":{"kind":"provider", "pid":0, "session":"provider-session", "seq":0},
        "loop_state":{"ready":false, "harness_record_started":false,
            "published_timeline":[], "delivery_episode":0}
    })).unwrap()
        });
    let startup_resume_path = resume_path.clone();
    let inherited = inherited.map(str::to_owned);
    let socket_arg = socket.clone();
    let home = root.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = fractal_daemon::test_support::command(test_env!("CARGO_BIN_EXE_fractal-fixture"));
        command
            .env_clear()
            .env("PTY_ROOT", home.join("pty"))
            .env("HOME", home);
        if let Some(incarnation) = inherited {
            command.env("ST3_INCARNATION", incarnation);
        }
        if let Some(path) = startup_resume_path {
            command.env(fractal_drivers::reexec::DRIVER_RESUME_ENV, path);
        }
        command
            .arg("--endpoint")
            .arg(socket_arg)
            .args([
                "driver",
                "exec",
                "--subject",
                subject,
                "--",
                env!("ST3_FIXTURE_BASH"),
                "--noprofile",
                "--norc",
                "-c",
                "printf '%s' \"$ST3_INCARNATION\"",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    if let Some(path) = resume_path {
        assert!(
            path.exists(),
            "startup must leave the adoption record for the driver"
        );
    }
}

#[tokio::test]
async fn fresh_driver_exports_its_incarnation_in_the_provider_environment() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    provider_incarnation(None, false, "agent/garden/worker", "worker:current").await;
}

#[tokio::test]
async fn fresh_driver_replaces_an_inherited_incarnation() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    provider_incarnation(
        Some("worker:predecessor"),
        false,
        "agent/garden/worker",
        "worker:current",
    )
    .await;
}

#[tokio::test]
async fn reexecuted_driver_refreshes_its_environment_from_the_saved_runtime_fence() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    provider_incarnation(
        Some("worker:predecessor"),
        true,
        "agent/garden/worker",
        "worker:current",
    )
    .await;
}

#[tokio::test]
async fn exec_gates_start_without_an_agent_runtime_fence() {
    if fractal_daemon::test_support::supervise_test() {
        return;
    }
    provider_incarnation(None, false, "gate-operation/catalog/check", "").await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn custom_shell_claim_without_incarnation_precedes_reconcile_application() {
    use fractal_daemon::{MissionsStore, RuntimeStartsStore, SeatsStoreCore};
    use fractal_daemon::model::MissionRunRequest;
    use fractal_daemon::reconcile::{NativeRuntime, Reconciler, RuntimeControl};
    use std::sync::atomic::Ordering;
    if fractal_daemon::test_support::supervise_test() { return; }
    const SEAT: &str = "agent/garden/worker";
    fn bound(_: u32, _: Option<&fractal_runtime::PeerProcessIdentity>) -> fractal_daemon::api::HarnessAncestry {
        fractal_daemon::api::HarnessAncestry::Bound(SEAT.into())
    }
    let root = tempfile::tempdir().unwrap();
    let shell = env!("ST3_FIXTURE_BASH");
    let cli = test_env!("CARGO_BIN_EXE_fractal-fixture");
    let environment: BTreeMap<String, String> = std::env::vars().collect();
    let pty = fractal_runtime::resolve_executable("pty", &environment).expect("pinned test environment needs pty");
    let script = root.path().join("claim.sh");
    std::fs::write(&script, r#"
unset ST3_INCARNATION FRACTAL_INCARNATION
while [ ! -f "$FIXTURE_ROOT/release" ]; do sleep 0.01; done
IFS= read -r step < "$FIXTURE_ROOT/step"
"$FIXTURE_CLI" --endpoint "$FIXTURE_ROOT/api.sock" --json work claim "$step" --as agent/garden/worker > "$FIXTURE_ROOT/result" 2> "$FIXTURE_ROOT/error"
printf '%s' "$?" > "$FIXTURE_ROOT/exit"
while :; do sleep 60; done
"#).unwrap();
    let command = format!("exec {shell:?} --noprofile --norc {:?}", script.to_str().unwrap());
    let source = format!(r#"version 2
agent "garden/worker" {{
  workspace {root:?}
  command {command:?}
  restart "never"
  env {{
    FIXTURE_ROOT {root:?}
    FIXTURE_CLI {cli:?}
    PTY_ROOT {pty_root:?}
    PATH {path:?}
  }}
}}
mission "catalog" state="ready" {{
  goal "Claim work before the next reconcile pass."
  step "choose" {{ assigned-to "agent/garden/worker"; goal "Choose." }}
}}
"#, root=root.path().to_str().unwrap(), pty_root=root.path().join("pty").to_str().unwrap(),
        path=std::env::var("PATH").unwrap());
    let store = Arc::new(Store::open_memory("orchid").unwrap());
    store.apply_internal(&fractal_daemon::parse_intent(&source, "orchid").unwrap(), "shell-cli").unwrap();
    let run = store.create_mission_run(&MissionRunRequest {
        mission:"catalog".into(), revision:None, workspace:root.path().display().to_string(),
        requester:Some("person/ada".into()), mode:Some("run".into()),
        inputs:BTreeMap::new(), idempotency_key:"run".into(),
    }).unwrap();
    let step = &run.steps[0].subject;
    store.set_step_state(step, "ready", None).unwrap();
    std::fs::write(root.path().join("step"), format!("{step}\n")).unwrap();
    let state = fixture_state(root.path(), store.clone());
    let socket = state.api_socket.clone();
    let server_store = store.clone();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix_with_ancestor(
            &socket, None, fractal_daemon::api::router(state), Some(server_store), bound,
        ).await
    });
    let runtime = Arc::new(NativeRuntime::new(root.path(), Some(&root.path().join("pty")), &pty));
    let member = store.desired_subject_with_writer(SEAT).unwrap().unwrap().0.member.unwrap();
    struct Cleanup(Arc<NativeRuntime>, String, tokio::task::JoinHandle<anyhow::Result<()>>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.kill(&self.1, true, None);
            let _ = self.0.remove(&self.1, true);
            self.2.abort();
        }
    }
    let _cleanup = Cleanup(runtime.clone(), member.runtime_id.clone(), server);
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "orchid".into(), Arc::new(Notify::new()));
    reconciler.wait_for_test_starts.store(false, Ordering::Release);
    // One pass dispatches the custom shell. No pass applies its worker completion.
    tokio::task::spawn_blocking(move || reconciler.reconcile_once()).await.unwrap().unwrap();
    let operation = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(operation) = store.runtime_start_for(SEAT).unwrap()
                && operation.finished { break operation; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("custom shell launch worker did not finish");
    assert!(!operation.applied);
    assert!(!operation.receipted);
    let result = operation.result.as_ref().unwrap();
    assert_eq!(result["phase"], "completed", "{result}");
    assert!(result["runtime_pid"].is_number(), "{result}; identity: {:?}; snapshot: {:?}",
        runtime.start_process_identity(&member, result["incarnation"].as_str().unwrap()),
        fractal_runtime::PtyRuntime::new(root.path().join("pty")).snapshot());
    let incarnation = result["incarnation"].as_str().unwrap();
    assert!(store.latest_claim(SEAT, Some("runtime.observed")).unwrap().is_none());
    std::fs::write(root.path().join("release"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !root.path().join("exit").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("custom shell CLI claim did not return");
    assert_eq!(std::fs::read_to_string(root.path().join("exit")).unwrap(), "0",
        "{}", std::fs::read_to_string(root.path().join("error")).unwrap());
    let claimed = store.step_run(step).unwrap().unwrap();
    assert_eq!(claimed.status, "claimed");
    assert_eq!(claimed.claim_incarnation.as_deref(), Some(incarnation));
    let pending = store.runtime_start_for(SEAT).unwrap().unwrap();
    assert!(!pending.applied);
    assert!(!pending.receipted);
}
