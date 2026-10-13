//! Start, checkpoint, transfer, acknowledge and close through an isolated durable daemon.
#![cfg(unix)]
use serde_json::{Value, json};
use st3::{api::AppState, model::ClaimInput, store::Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Notify, watch};

struct NoRuntime;

impl st3::reconcile::RuntimeControl for NoRuntime {
    fn snapshot_ptys(&self) -> anyhow::Result<Vec<st3::reconcile::RuntimeObservation>> {
        Ok(Vec::new())
    }
    fn observe_exec(&self, _: &str) -> anyhow::Result<Option<st3::reconcile::RuntimeObservation>> {
        Ok(None)
    }
    fn start(&self, _: &st3::model::MemberSpec) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&self, _: &str, _: bool, _: Option<&str>) -> anyhow::Result<()> {
        Ok(())
    }
    fn kill(&self, _: &str, _: bool, _: Option<&str>) -> anyhow::Result<()> {
        Ok(())
    }
    fn remove(&self, _: &str, _: bool) -> anyhow::Result<()> {
        Ok(())
    }
    fn screen(&self, _: &str) -> anyhow::Result<String> {
        Ok(String::new())
    }
    fn send_key(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn read_exec_log(&self, _: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

fn finish_run(store: &Arc<Store>, run: &str) {
    let reconciler = st3::reconcile::Reconciler::new(
        store.clone(),
        Arc::new(NoRuntime),
        "example".into(),
        Arc::new(Notify::new()),
    );
    for _ in 0..20 {
        reconciler.reconcile_once().unwrap();
        if store.mission_run(run).unwrap().unwrap().status == "completed" {
            return;
        }
    }
    panic!(
        "minimal work run did not complete: {:?}",
        store.mission_run(run).unwrap()
    );
}

const ALDER: &str = "agent/example/alder";
const BIRCH: &str = "agent/example/birch";

struct Daemon {
    root: PathBuf,
    store: Arc<Store>,
    server: Option<tokio::task::JoinHandle<()>>,
}
impl Daemon {
    fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            store: Arc::new(Store::open(&root.join("graph.sqlite3"), "example").unwrap()),
            server: None,
        }
    }
    fn socket(&self) -> PathBuf {
        self.root.join("api.sock")
    }
    async fn start(&mut self) {
        self.start_with_claim_outage(false).await;
    }
    async fn start_with_claim_outage(&mut self, outage: bool) {
        let state = AppState {
            store: self.store.clone(),
            notify: Arc::new(Notify::new()),
            event_notify: watch::channel(0_u64).0,
            node: "example".into(),
            state_dir: self.root.clone(),
            pty_root: self.root.join("pty"),
            pty_binary: "pty".into(),
            fleet_id: None,
            configured_peers: vec![],
            client_relay: None,
            native_session_home: None,
            planner_default: st3::model::PlannerSpec::default(),
        };
        let socket = self.socket();
        let self_socket = socket.clone();
        self.server = Some(tokio::spawn(async move {
            let app = st3::api::router(state).layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let socket = socket.clone();
                    async move {
                        let disconnect = outage && request.method() == axum::http::Method::POST
                            && request.uri().path().starts_with("/v1/work/claim/");
                        let response = next.run(request).await;
                        if disconnect && response.status().is_success() {
                            // Leave a refused socket after the authoritative commit,
                            // before its response reaches the work-command process.
                            std::fs::remove_file(&socket).unwrap();
                            drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
                        }
                        response
                    }
                },
            ));
            st3::api::serve_unix(&self_socket, app)
                .await
                .unwrap();
        }));
        for _ in 0..200 {
            if std::os::unix::net::UnixStream::connect(self.socket()).is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("isolated daemon did not start");
    }
    async fn restart(&mut self) {
        self.stop().await;
        self.store = Arc::new(Store::open(&self.root.join("graph.sqlite3"), "example").unwrap());
        self.store.replay_replication_graph().unwrap();
        self.start().await;
    }
    async fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
            let _ = server.await;
        }
    }
    async fn cli(&self, args: &[&str]) -> std::process::Output {
        let mut command =
            st3::test_support::async_command(test_bin!("st3-fixture"));
        command
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .arg("--endpoint")
            .arg(self.socket())
            .args(["--daemon-wait", "0", "--json"])
            .args(args);
        command.output().await.unwrap()
    }
    async fn ok(&self, args: &[&str]) -> Value {
        let output = self.cli(args).await;
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    async fn fails(&self, args: &[&str], message: &str) {
        let output = self.cli(args).await;
        assert!(!output.status.success(), "unexpected success for {args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "expected {message}: {stderr}");
    }
    fn declare(&self, person: Option<&str>) {
        let source = "version 2\nagent \"example/alder\" { workspace \"/tmp\"; command \"true\"; }\nagent \"example/birch\" { workspace \"/tmp\"; command \"true\"; }";
        let intent = st3::parse_intent(source, "example").unwrap();
        if let Some(person) = person {
            let preview = self.store.mission(&intent, st3::model::IntentInput {
                kdl: source.into(), source_name: None,
            }).unwrap();
            self.store.apply_as(&intent, &preview.subject_tokens, "seats", Some(person)).unwrap();
        } else {
            self.store.apply_internal(&intent, "seats").unwrap();
        }
        for (actor, incarnation) in [(ALDER, "alder-one"), (BIRCH, "birch-one")] {
            for (kind, fields) in [
                (
                    "runtime.observed",
                    json!({"runtime_id": actor.trim_start_matches("agent/"), "incarnation_id": incarnation, "status": "running", "reachability": "local"}),
                ),
                (
                    "harness.observed",
                    json!({"incarnation_id": incarnation, "state": "ready"}),
                ),
            ] {
                self.store
                    .append_claim(&ClaimInput {
                        subject: actor.into(),
                        kind: kind.into(),
                        actor: None,
                        fields: serde_json::from_value(fields).unwrap(),
                        evidence: vec![],
                        expected_subject: None,
                        idempotency_key: None,
                    })
                    .unwrap();
            }
        }
    }
    fn note(&self, recipient: &str) -> String {
        let notes = self.store.messages(Some(recipient), true).unwrap();
        assert_eq!(notes.len(), 1, "one durable note per transfer");
        assert!(notes[0].content.contains("Next: check the fixture"));
        notes[0].subject.clone()
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

#[tokio::test]
async fn post_work_refreshes_trace_on_claim_and_clears_it_on_complete() {
    check_trace_claim_end(false, false, TraceEnd::Complete).await;
}

#[tokio::test]
async fn ad_hoc_post_work_refreshes_trace_on_claim_and_clears_it_on_complete() {
    check_trace_claim_end(true, false, TraceEnd::Complete).await;
}

#[tokio::test]
async fn post_work_reports_committed_result_when_trace_publication_fails() {
    check_trace_claim_end(true, true, TraceEnd::Complete).await;
}

#[derive(Clone, Copy)]
enum TraceEnd { Complete, Release, Outage }

#[tokio::test]
async fn post_work_release_refreshes_trace_to_idle() {
    check_trace_claim_end(false, false, TraceEnd::Release).await;
}

#[tokio::test]
async fn post_work_capture_does_not_inherit_daemon_outage_wait() {
    check_trace_claim_end(false, false, TraceEnd::Outage).await;
}

async fn check_trace_claim_end(ad_hoc: bool, broken_trace: bool, end: TraceEnd) {
    use st3::agent_trace::issuer::{self, Facts};
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(root.path());
    daemon.store.set_node_key(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0)).unwrap();
    daemon.store.ensure_principal_key("person/avery").unwrap();
    daemon.declare(Some("person/avery"));
    let root_source = r#"version 2
agent "example/alder" { accountable "person/avery"; workspace "/tmp"; command "true"; }
"#;
    let root_intent = st3::parse_intent(root_source, "example").unwrap();
    let root_preview = daemon.store.mission(&root_intent, st3::model::IntentInput {
        kdl: root_source.into(), source_name: None,
    }).unwrap();
    daemon.store.apply_as(&root_intent, &root_preview.subject_tokens,
        "trace-root", Some("person/avery")).unwrap();
    daemon.store.seal_local_batches().unwrap();
    daemon.store.judge_claims(true).unwrap();
    assert_eq!(daemon.store.trace_principal(ALDER).unwrap().as_deref(), Some("person/avery"));
    let source = r#"version 2
mission "trace-check" state="ready" {
    goal "Check runtime trace lifecycle."
    step "work" { assigned-to "agent/example/alder"; }
}
"#;
    let intent = st3::parse_intent(source, "example").unwrap();
    let preview = daemon.store.mission(&intent, st3::model::IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    daemon.store.apply_as(&intent, &preview.subject_tokens, "trace-mission", Some("person/avery")).unwrap();
    daemon.start_with_claim_outage(matches!(end, TraceEnd::Outage)).await;
    let opened = if ad_hoc {
        daemon.ok(&["work", "start", "Check trace lifecycle", "--as", ALDER]).await
    } else {
        daemon.ok(&["missions", "start", "trace-check", "--workspace",
            root.path().to_str().unwrap(), "--as", "person/avery"]).await
    };
    let run = if ad_hoc { opened["run"].as_str().unwrap() }
        else { opened["mission_run"]["subject"].as_str().unwrap() };
    let reconciler = st3::reconcile::Reconciler::new(
        daemon.store.clone(), Arc::new(NoRuntime), "example".into(), Arc::new(Notify::new()),
    );
    reconciler.reconcile_once().unwrap();
    // The process-free reconciler advances mission readiness but has no physical PTY.
    // Restore the fixture's explicit live harness observation before claiming work.
    for (kind, fields) in [
        ("runtime.observed", json!({"runtime_id":"example/alder",
            "incarnation_id":"alder-one","status":"running","reachability":"local"})),
        ("harness.observed", json!({"incarnation_id":"alder-one","state":"ready"})),
    ] {
        daemon.store.append_claim(&ClaimInput {
            subject: ALDER.into(), kind: kind.into(), actor: None,
            fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
            expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    let run_view = daemon.store.mission_run_steps(run, false).unwrap().unwrap();
    let step = run_view.steps[0].subject.as_str();
    let path = root.path().join("agent-trace.json");
    let runtime = issuer::build(&Facts {
        agent: ALDER.into(),
        principal: "person/avery".into(),
        incarnation: "alder-one".into(),
        harness_tool: "omp".into(),
        harness_version: None,
        model_route: None,
        model_effort: None,
        session: None,
        host: Some("example".into()),
        worktree: None,
        mission_name: None,
    }, issuer::ClaimPurpose::Idle).unwrap();
    if broken_trace {
        std::fs::create_dir(&path).unwrap();
    } else {
        issuer::write_atomic(&path, &runtime).unwrap();
    }
    let actions = if matches!(end, TraceEnd::Outage) {
        vec!["claim"]
    } else {
        vec!["claim", if matches!(end, TraceEnd::Release) { "release" } else { "complete" }]
    };
    for action in actions {
        let mut command = st3::test_support::async_command(test_bin!("st3-fixture"));
        command.env("HOME", root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("ST_AGENT", ALDER)
            .env("AGENT_TRACE_FILE", &path)
            .arg("--endpoint").arg(daemon.socket())
            .env("ST3_DAEMON_WAIT", "30")
            .args(["--json", "work", action, step,
                "--as", ALDER, "--incarnation", "alder-one",
                "--summary", "Trace lifecycle checked", "--evidence", "file:trace"]);
        let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
            .await.expect("advisory capture inherited the 30-second daemon wait").unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
        let committed: st3::model::StepRunView = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(committed.subject, step);
        assert_eq!(committed.status, match action {
            "claim" => "claimed",
            "release" => "ready",
            _ => "verifying",
        });
        if matches!(end, TraceEnd::Outage) {
            let log = std::fs::read_to_string(
                root.path().join("state/st3/driver-api-warnings.log")).unwrap();
            assert!(log.contains("agent_trace_omitted"), "{log}");
            assert!(log.contains("Connection refused"), "{log}");
            assert!(!path.exists(), "failed capture leaves no stale trace");
            continue;
        }
        if broken_trace {
            let log = std::fs::read_to_string(
                root.path().join("state/st3/driver-api-warnings.log")).unwrap();
            let diagnostic: Value = serde_json::from_str(
                log.lines().next_back().unwrap().splitn(3, ' ').nth(2).unwrap()).unwrap();
            assert_eq!(diagnostic["event"], "agent_trace_omitted");
            assert_eq!(diagnostic["subject"], ALDER);
            assert!(diagnostic["error"].as_str().is_some_and(|error| !error.is_empty()));
            assert!(diagnostic["cleanup_error"].as_str().is_some_and(|error| !error.is_empty()));
            assert!(path.is_dir(), "a failed trace must not replace unrelated directory contents");
            continue;
        }
        let runtime: st3::agent_trace::Runtime =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let value = serde_json::to_value(&runtime).unwrap();
        if action == "claim" {
            assert_eq!(value["why"]["_tag"], "bound");
            assert_eq!(value["how"]["issuance"]["_tag"], "st-issued");
            assert_eq!(value["how"]["issuance"]["step_run"], step);
            assert_eq!(value["why"]["mission"]["run"], run);
            assert_eq!(value["why"]["step"]["run"],
                run_view.steps[0].generation.strip_prefix("run-generation/").unwrap());
            assert!(value["why"]["mission"]["name"].is_string());
            assert!(value["why"]["step"]["name"].is_string());
        } else {
            assert_eq!(value["why"], json!({"_tag": "idle"}),
                "completion must not retain mission or step");
            assert!(runtime.how.issuance.is_none(), "completion must not retain a lease");
        }
    }
}

#[tokio::test]
async fn spontaneous_work_survives_restart_handoff_acknowledgment_and_close() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(root.path());
    daemon.declare(None);
    daemon.start().await;
    let title = "Inspect ${literal} fixture";
    let start = [
        "work",
        "start",
        title,
        "--as",
        ALDER,
        "--idempotency-key",
        "fixture-start",
    ];
    let opened = daemon.ok(&start).await;
    let step = opened["subject"].as_str().unwrap();
    assert_eq!(opened["title"], title);
    assert_eq!(opened["assigned_to"], ALDER);
    assert_eq!(daemon.ok(&start).await["subject"], step);
    daemon
        .fails(
            &[
                "work",
                "start",
                "Another title",
                "--as",
                ALDER,
                "--idempotency-key",
                "fixture-start",
            ],
            "another task",
        )
        .await;
    daemon
        .ok(&[
            "work",
            "claim",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
        ])
        .await;
    daemon
        .fails(
            &["work", "start", "Independent work", "--as", ALDER],
            "finish or release",
        )
        .await;
    daemon
        .ok(&[
            "work",
            "progress",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
            "--summary",
            "Current: fixture found",
            "--evidence",
            "file:fixture",
        ])
        .await;
    daemon.restart().await;
    assert_eq!(daemon.ok(&start).await["subject"], step);
    let handoff = [
        "work",
        "handoff",
        step,
        "--as",
        ALDER,
        "--incarnation",
        "alder-one",
        "--to",
        BIRCH,
        "--note",
        "Next: check the fixture",
        "--idempotency-key",
        "fixture-handoff",
    ];
    daemon
        .fails(
            &[
                "work",
                "handoff",
                step,
                "--as",
                ALDER,
                "--incarnation",
                "stale",
                "--to",
                BIRCH,
                "--note",
                "Next: check the fixture",
            ],
            "not the current live incarnation",
        )
        .await;
    let transferred = daemon.ok(&handoff).await;
    assert_eq!(transferred["assigned_to"], BIRCH);
    assert!(transferred["claimant"].is_null());
    assert_eq!(daemon.ok(&handoff).await["subject"], step);
    let note = daemon.note(BIRCH);
    daemon
        .ok(&["conversations", "read", &note, "--as", BIRCH])
        .await;
    assert_eq!(daemon.store.message(&note).unwrap().unwrap().status, "read");
    daemon
        .fails(
            &[
                "work",
                "claim",
                step,
                "--as",
                BIRCH,
                "--incarnation",
                "birch-one",
            ],
            "acknowledge",
        )
        .await;
    daemon
        .fails(
            &[
                "work",
                "progress",
                step,
                "--as",
                ALDER,
                "--incarnation",
                "alder-one",
            ],
            "not available",
        )
        .await;
    daemon.restart().await;
    assert_eq!(daemon.note(BIRCH), note);
    let shown = daemon.ok(&["work", "show", step]).await;
    assert!(shown.to_string().contains("Next: check the fixture"));
    daemon
        .fails(
            &[
                "work",
                "acknowledge",
                step,
                "--message",
                &note,
                "--as",
                ALDER,
            ],
            "current recipient",
        )
        .await;
    daemon
        .fails(
            &[
                "work",
                "acknowledge",
                step,
                "--message",
                "message/old",
                "--as",
                BIRCH,
            ],
            "current recipient",
        )
        .await;
    daemon
        .ok(&[
            "work",
            "acknowledge",
            step,
            "--message",
            &note,
            "--as",
            BIRCH,
        ])
        .await;
    daemon
        .ok(&[
            "work",
            "acknowledge",
            step,
            "--message",
            &note,
            "--as",
            BIRCH,
        ])
        .await;
    assert_eq!(
        daemon
            .store
            .claims_for(&note, Some("message.read"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(daemon.store.message(&note).unwrap().unwrap().status, "read");
    daemon.restart().await;
    assert!(
        daemon
            .ok(&["work", "show", step])
            .await
            .to_string()
            .contains("(acknowledged)")
    );
    daemon
        .ok(&[
            "work",
            "claim",
            step,
            "--as",
            BIRCH,
            "--incarnation",
            "birch-one",
        ])
        .await;
    daemon
        .fails(
            &[
                "work",
                "complete",
                step,
                "--as",
                BIRCH,
                "--incarnation",
                "birch-one",
            ],
            "evidence",
        )
        .await;
    let submitted = daemon
        .ok(&[
            "work",
            "complete",
            step,
            "--as",
            BIRCH,
            "--incarnation",
            "birch-one",
            "--summary",
            "Fixture checked",
            "--evidence",
            "file:verified-fixture",
        ])
        .await;
    assert_eq!(submitted["status"], "verifying");
    finish_run(&daemon.store, opened["run"].as_str().unwrap());
    daemon.restart().await;
    let run = daemon
        .store
        .mission_run(opened["run"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "completed");
    assert!(
        run.steps
            .iter()
            .all(|s| s.status == "completed" && s.claimant.is_none())
    );
    let evidence = daemon
        .store
        .claims_for(step, Some("work.submitted"))
        .unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].body["evidence"],
        json!(["file:verified-fixture"])
    );
    daemon.stop().await;
}

#[tokio::test]
async fn person_handoff_is_visible_acknowledged_and_closed_with_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(root.path());
    daemon.declare(None);
    daemon.start().await;
    let opened = daemon
        .ok(&["work", "start", "Review fixture", "--as", ALDER])
        .await;
    let step = opened["subject"].as_str().unwrap();
    daemon
        .ok(&[
            "work",
            "claim",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
        ])
        .await;
    daemon
        .ok(&[
            "work",
            "handoff",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
            "--to",
            "person/avery",
            "--note",
            "Next: check the fixture",
        ])
        .await;
    let note = daemon.note("person/avery");
    let attention = daemon.store.attention_items(Some("person/avery")).unwrap();
    assert!(attention.iter().any(|item| item.subject == step));
    daemon
        .fails(
            &[
                "work",
                "done",
                step,
                "--as",
                "person/avery",
                "--summary",
                "Reviewed",
                "--evidence",
                "file:review",
            ],
            "acknowledge",
        )
        .await;
    daemon.restart().await;
    daemon
        .ok(&[
            "work",
            "acknowledge",
            step,
            "--message",
            &note,
            "--as",
            "person/avery",
        ])
        .await;
    daemon
        .fails(
            &[
                "work",
                "done",
                step,
                "--as",
                "person/avery",
                "--summary",
                "Reviewed",
            ],
            "evidence",
        )
        .await;
    daemon
        .ok(&[
            "work",
            "done",
            step,
            "--as",
            "person/avery",
            "--summary",
            "Reviewed",
            "--evidence",
            "file:review",
        ])
        .await;
    finish_run(&daemon.store, opened["run"].as_str().unwrap());
    daemon.restart().await;
    assert_eq!(
        daemon
            .store
            .mission_run(opened["run"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .status,
        "completed"
    );
    assert!(
        daemon
            .store
            .attention_items(Some("person/avery"))
            .unwrap()
            .is_empty()
    );
    daemon.stop().await;
}

/// A person answering an ask an agent made from spontaneous work closes the ask, not the work, so
/// it needs no evidence (Nathan, 2026-10-06: approving an attention item failed with "close
/// spontaneous work with --evidence"). Closing a handed-off step still does.
#[tokio::test]
async fn a_person_answers_an_ask_from_spontaneous_work_without_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(root.path());
    daemon.declare(None);
    daemon.start().await;
    let opened = daemon
        .ok(&["work", "start", "Rollout scope", "--as", ALDER])
        .await;
    let step = opened["subject"].as_str().unwrap();
    daemon
        .ok(&[
            "work",
            "claim",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
        ])
        .await;
    let asked = daemon
        .ok(&[
            "work",
            "ask",
            "--for",
            "person/avery",
            "--title",
            "Integrate or keep the pilot?",
            "--reason",
            "The rollout needs a scope decision",
            "--step",
            step,
            "--as",
            ALDER,
            "--incarnation",
            "alder-one",
            "--idempotency-key",
            "scope-ask",
        ])
        .await;
    let ask = asked["subject"].as_str().unwrap();
    daemon
        .ok(&[
            "work",
            "done",
            ask,
            "--as",
            "person/avery",
            "--summary",
            "Integrate it",
        ])
        .await;
    daemon.stop().await;
}
