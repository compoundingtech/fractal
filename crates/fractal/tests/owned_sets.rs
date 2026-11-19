//! Each fixture serves an isolated daemon over its own Unix socket and durable graph.
use fractal_daemon::SeatsStoreCore;
use fractal_daemon::OwnedSetsStore;
use fractal_daemon::RetirementsStore;
use std::{path::Path, sync::Arc, time::Duration};

use serde_json::{Value, json};
use fractal_daemon::{
    api::AppState,
    client::Client,
    fleet::MemberKey,
    model::{ClaimInput, IntentInput},
    store::{
        Store,
        owned_sets::{Enforce, Options, PolicyRequest, Preview, Request, Source},
    },
};
use tokio::sync::{Notify, watch};

const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

struct Daemon {
    store: Arc<Store>,
    client: Client,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn daemon(root: &Path, name: &str, key: Arc<MemberKey>, anchor: &MemberKey) -> Daemon {
    let root = root.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(Store::open(&root.join("graph.sqlite3"), name).unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(anchor.public()).unwrap();
    store.set_member_key(Some(key)).unwrap();
    append(
        &store,
        &format!("daemon/{name}"),
        "daemon.started",
        json!({"status":"running","features":{"owned_sets":1}}),
    );
    let socket = root.join("st3.sock");
    let state = AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        owning_person: None,
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0).0,
        node: name.into(),
        api_socket: root.join("api.sock"),
        state_dir: root.clone(),
        pty_root: root.join("pty"),
        pty_binary: "pty".into(),
        fleet_id: Some(FLEET.into()),
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    };
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix(&server_socket, fractal_daemon::api::router(state))
            .await
            .unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !socket.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    Daemon {
        store,
        client: Client::unix_as(socket, "person/operator").unwrap(),
        server,
    }
}

fn append(store: &Store, subject: &str, kind: &str, fields: Value) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn share(from: &Store, to: &Store, peer: &str) {
    let exchange = from
        .export_replication_exchange_answering(
            FLEET,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        )
        .unwrap();
    to.receive_replication_exchange(peer, FLEET, &exchange)
        .unwrap();
    let admission = to.validate_replication_backlog().unwrap();
    assert_eq!((admission.invalid, admission.unknown), (0, 0));
    to.project_replication_backlog().unwrap();
}

fn bundle(note: &str, second: bool) -> String {
    let mut kdl = format!(
        "version 2\nagent \"garden/orchard\" {{\n workspace \".\"\n command \"true\"\n description \"{note}\"\n}}\n"
    );
    if second {
        kdl.push_str("agent \"garden/meadow\" {\n workspace \".\"\n command \"true\"\n}\n");
    }
    kdl
}

async fn request(daemon: &Daemon, sequence: u64, kdl: String) -> Request {
    let selected = daemon.store.owned_sets().unwrap().into_iter().next();
    Request {
        intent: IntentInput {
            kdl,
            source_name: None,
        },
        options: Options {
            rollout: None,
            set: "garden".into(),
            source: Source {
                repository: "acme/garden".into(),
                r#ref: "refs/heads/main".into(),
                sha: format!("{sequence:040x}"),
                sequence,
            },
            expected_set: selected.map_or("absent".into(), |v| v.revision),
            adopt: Default::default(),
            allow_empty: false,
            confirm_retire: None,
            policy: None,
            expected_subjects: Default::default(),
        },
        actor: "person/operator".into(),
        idempotency_key: format!("publish-{sequence}"),
        program_bundles: Default::default(),
        source_map: None,
    }
}

async fn preview(daemon: &Daemon, request: &mut Request) -> Preview {
    let preview: Preview = daemon
        .client
        .post("/v1/sets/preview", request)
        .await
        .unwrap();
    assert!(preview.blockers.is_empty(), "{preview:?}");
    request.options.expected_subjects = preview.expected_subjects.clone();
    preview
}

async fn publish(daemon: &Daemon, sequence: u64, note: &str) {
    let mut request = request(daemon, sequence, bundle(note, true)).await;
    preview(daemon, &mut request).await;
    let result: Value = daemon
        .client
        .post("/v1/sets/apply", &request)
        .await
        .unwrap();
    assert_eq!(result["publication"]["changed"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_daemons_heal_to_newest_source_and_pruning_requires_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let keys = (0..3)
        .map(|_| Arc::new(MemberKey::generate().unwrap().0))
        .collect::<Vec<_>>();
    let amber = daemon(root.path(), "amber", keys[0].clone(), &keys[0]).await;
    let cobalt = daemon(root.path(), "cobalt", keys[1].clone(), &keys[0]).await;
    let ivory = daemon(root.path(), "ivory", keys[2].clone(), &keys[0]).await;
    for (i, name) in ["amber", "cobalt", "ivory"].into_iter().enumerate() {
        let mut fields = json!({"fleet_id":FLEET,"member_key":keys[i].public(),
            "via":if i==0{"anchor"}else{"invite"},"mode":"listening"});
        if i != 0 {
            fields["sponsor"] = json!("host/amber");
        }
        append(
            &amber.store,
            &format!("host/{name}"),
            "fleet.member-admitted",
            fields,
        );
    }
    for (from, name) in [(&amber, "amber"), (&cobalt, "cobalt"), (&ivory, "ivory")] {
        for to in [&amber, &cobalt, &ivory] {
            share(&from.store, &to.store, name);
        }
    }
    publish(&amber, 10, "initial").await;
    share(&amber.store, &cobalt.store, "amber");
    share(&amber.store, &ivory.store, "amber");
    publish(&amber, 30, "newer").await;
    publish(&cobalt, 20, "older").await;
    share(&amber.store, &ivory.store, "amber");
    share(&cobalt.store, &ivory.store, "cobalt"); // Late older render.
    share(&cobalt.store, &amber.store, "cobalt");
    share(&amber.store, &cobalt.store, "amber");
    let token = amber
        .store
        .selected_desired_token("agent/garden/orchard")
        .unwrap()
        .unwrap();
    for d in [&amber, &cobalt, &ivory] {
        let response: Value = d.client.get("/v1/client/sets/garden").await.unwrap();
        assert_eq!(response["receipt"]["source"]["sequence"], 30);
        assert_eq!(
            d.store
                .selected_desired_token("agent/garden/orchard")
                .unwrap()
                .as_deref(),
            Some(token.as_str())
        );
        assert!(
            d.store
                .status(Some("agent/garden/orchard"))
                .unwrap()
                .subjects[0]
                .conflicts
                .is_empty()
        );
        // Typed generated clients understand the additive owned-set resource.
        serde_json::from_value::<fractal_client::Resource>(response).unwrap();
    }
    let selected = amber.store.owned_sets().unwrap().remove(0);
    for (subject, member) in &selected.receipt.members {
        let incarnation = format!("launch-{subject}");
        append(
            &amber.store,
            subject,
            "runtime.action.succeeded",
            json!({
                "action":"start", "incarnation_id":incarnation, "desired_token":member.claim,
            }),
        );
        append(
            &amber.store,
            subject,
            "runtime.observed",
            json!({
                "status":"running", "incarnation_id":incarnation, "runtime_id":subject,
            }),
        );
    }
    let running: Value = amber
        .client
        .get(&format!("/v1/client/sets/garden?sha={:040x}", 30))
        .await
        .unwrap();
    assert_eq!(running["commit_status"]["running"], true);
    for member in running["members_status"].as_array().unwrap() {
        assert_eq!(member["desired_token"], member["launched_token"]);
        assert_eq!(member["rollout"], "running");
    }
    let mut stale = request(&ivory, 25, bundle("late", true)).await;
    let stale_preview: Preview = ivory.client.post("/v1/sets/preview", &stale).await.unwrap();
    assert!(!stale_preview.blockers.is_empty());
    stale.options.expected_subjects = stale_preview.expected_subjects;
    assert!(
        ivory
            .client
            .post::<_, Value>("/v1/sets/apply", &stale)
            .await
            .is_err()
    );

    let mut prune = request(&amber, 40, bundle("newer", false)).await;
    let p = preview(&amber, &mut prune).await;
    assert!(p.mass_retirement);
    let before = amber.store.owned_sets().unwrap()[0].revision.clone();
    let refusal = amber
        .client
        .post::<_, Value>("/v1/sets/apply", &prune)
        .await
        .unwrap_err();
    assert!(
        refusal.to_string().contains("mass-retirement-refused"),
        "{refusal}"
    );
    assert_eq!(amber.store.owned_sets().unwrap()[0].revision, before);
    prune.options.confirm_retire = Some(p.digest);
    amber
        .client
        .post::<_, Value>("/v1/sets/apply", &prune)
        .await
        .unwrap();
    share(&amber.store, &cobalt.store, "amber");
    share(&amber.store, &ivory.store, "amber");
    for d in [&amber, &cobalt, &ivory] {
        let response: Value = d.client.get("/v1/client/sets/garden").await.unwrap();
        assert_eq!(response["receipt"]["source"]["sequence"], 40);
        assert!(response["receipt"]["retired"]["agent/garden/meadow"].is_object());
        assert_eq!(
            d.store
                .desired_subject_with_writer("agent/garden/meadow")
                .unwrap()
                .unwrap()
                .0
                .kind,
            "stop"
        );
        let old: Value = d
            .client
            .get(&format!("/v1/client/sets/garden?sha={:040x}", 30))
            .await
            .unwrap();
        assert_eq!(old["commit_status"]["superseded"], true);
        assert_eq!(old["commit_status"]["running"], false);
    }

    // The executable reads every specified file before publication and supports multi-file bundles.
    let previous = amber.store.owned_sets().unwrap()[0].revision.clone();
    let orchard = root.path().join("orchard.kdl");
    let blossom = root.path().join("blossom.kdl");
    std::fs::write(&orchard, bundle("newer", false)).unwrap();
    std::fs::write(
        &blossom,
        "version 2\nagent \"garden/blossom\" { command \"true\" }\n",
    )
    .unwrap();
    let run = |files: Vec<std::path::PathBuf>, dry: bool| {
        let socket = amber.client.socket_path().unwrap().to_path_buf();
        let previous = previous.clone();
        tokio::task::spawn_blocking(move || {
            let mut command =
                fractal_daemon::test_support::command(test_bin!("fractal-fixture"));
            command.env_remove("ST_AGENT").env_remove("FRACTAL_AGENT")
                .env_remove("ST_MISSION_RUN")
                .args(["--json", "--endpoint"])
                .arg(socket)
                .args(["apply", "--set", "garden"])
                .args(files)
                .args([
                    "--repository",
                    "acme/garden",
                    "--ref",
                    "refs/heads/main",
                    "--sha",
                ])
                .arg(format!("{:040x}", 50))
                .args(["--source-sequence", "50", "--expect-set"])
                .arg(previous)
                .args(["--as", "person/operator"]);
            if dry {
                command.arg("--dry-run");
            }
            command.output().unwrap()
        })
    };
    let missing = run(
        vec![orchard.clone(), root.path().join("missing.kdl")],
        false,
    )
    .await
    .unwrap();
    assert!(!missing.status.success());
    assert_eq!(amber.store.owned_sets().unwrap()[0].revision, previous);
    let dry = run(vec![orchard.clone(), blossom.clone()], true)
        .await
        .unwrap();
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let dry: Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(dry["changes"]["agent/garden/blossom"], "added");
    let applied = run(vec![orchard, blossom], false).await.unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        amber.store.owned_sets().unwrap()[0].receipt.source.sequence,
        50
    );
    let mut mixed = bundle("mixed", false);
    mixed.push_str("agent \"garden/blossom\" { command \"true\" }\n\
        mission \"harvest\" state=\"ready\" { goal \"Harvest\"; step \"work\" { assigned-to \"agent/garden/orchard\" } }\n\
        schedule \"garden/daily\" { every \"6h\"; anchor \"2026-01-01T00:00:00Z\"; work { mission \"harvest\"; workspace \"/tmp\"; } }\n");
    let mut mixed = request(&amber, 60, mixed).await;
    preview(&amber, &mut mixed).await;
    let correct_heads = mixed.options.expected_subjects.clone();
    mixed
        .options
        .expected_subjects
        .insert("mission/harvest".into(), vec!["stale".into()]);
    let before = amber.store.owned_sets().unwrap()[0].revision.clone();
    assert!(
        amber
            .client
            .post::<_, Value>("/v1/sets/apply", &mixed)
            .await
            .is_err()
    );
    assert_eq!(amber.store.owned_sets().unwrap()[0].revision, before);
    assert!(amber.store.mission_spec("harvest", None).unwrap().is_none());
    assert!(
        amber
            .store
            .selected_desired_token("schedule/garden/daily")
            .unwrap()
            .is_none()
    );
    mixed.options.expected_subjects = correct_heads;
    amber
        .client
        .post::<_, Value>("/v1/sets/apply", &mixed)
        .await
        .unwrap();
    share(&amber.store, &ivory.store, "amber");
    let selected = ivory.store.owned_sets().unwrap().remove(0);
    assert!(selected.blockers.is_empty());
    assert_eq!(selected.receipt.members["mission/harvest"].kind, "mission");
    assert_eq!(
        selected.receipt.members["schedule/garden/daily"].kind,
        "schedule"
    );
    let mut omit = request(&amber, 70, bundle("mixed", false)).await;
    let p = preview(&amber, &mut omit).await;
    omit.options.confirm_retire = Some(p.digest);
    amber
        .client
        .post::<_, Value>("/v1/sets/apply", &omit)
        .await
        .unwrap();
    share(&amber.store, &ivory.store, "amber");
    let selected = ivory.store.owned_sets().unwrap().remove(0);
    assert!(selected.blockers.is_empty(), "{:?}", selected.blockers);
    let schedule = ivory
        .store
        .desired_subject_with_writer("schedule/garden/daily")
        .unwrap()
        .unwrap()
        .0;
    assert!(
        fractal_daemon::graph::schedule_spec(&schedule.desired, "ivory")
            .unwrap()
            .stopped
    );
    assert_eq!(
        ivory
            .store
            .mission_spec("harvest", None)
            .unwrap()
            .unwrap()
            .state,
        fractal_daemon::model::MissionState::Retired
    );
    let launched_token = amber
        .store
        .selected_desired_token("agent/garden/orchard")
        .unwrap()
        .unwrap();
    append(
        &amber.store,
        "agent/garden/orchard",
        "runtime.action.succeeded",
        json!({
            "action":"start", "incarnation_id":"launch-agent/garden/orchard",
            "desired_token":launched_token,
        }),
    );
    let labeled = bundle("mixed", false).replace(
        "description \"mixed\"",
        "description \"mixed\"\n name \"Orchard\"",
    );
    let mut labeled = request(&amber, 80, labeled).await;
    preview(&amber, &mut labeled).await;
    amber
        .client
        .post::<_, Value>("/v1/sets/apply", &labeled)
        .await
        .unwrap();
    let status: Value = amber.client.get("/v1/client/sets/garden").await.unwrap();
    let orchard = status["members_status"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == "agent/garden/orchard")
        .unwrap();
    assert_eq!(orchard["rollout"], "running");
    assert_eq!(orchard["launch_current"], true);
    assert_eq!(orchard["launched_token"], launched_token);
    assert_ne!(orchard["desired_token"], orchard["launched_token"]);
    for args in [
        vec!["ls".to_owned()],
        vec!["show".to_owned(), "garden".to_owned()],
        vec![
            "status".to_owned(),
            "garden".to_owned(),
            "--sha".to_owned(),
            format!("{:040x}", 80),
        ],
    ] {
        let socket = root.path().join("amber/st3.sock");
        let output = tokio::task::spawn_blocking(move || {
            fractal_daemon::test_support::command(test_bin!("fractal-fixture")).env_remove("ST_AGENT").env_remove("FRACTAL_AGENT")
                .env_remove("ST_MISSION_RUN")
                .args(["--json", "--endpoint"])
                .arg(socket)
                .arg("sets")
                .args(args)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let selected = value["value"]["items"]
            .as_array()
            .map_or(&value["value"], |items| &items[0]);
        assert_eq!(selected["receipt"]["source"]["sequence"], 80);
    }
}

/// PTY process identities and exits are controlled independently of the isolated HTTP daemon.
#[derive(Default)]
struct RolloutRuntime {
    observations:
        std::sync::Mutex<std::collections::BTreeMap<String, fractal_daemon::reconcile::RuntimeObservation>>,
    starts: std::sync::Mutex<Vec<fractal_daemon::model::MemberSpec>>,
    stops: std::sync::Mutex<Vec<String>>,
}
impl fractal_daemon::reconcile::RuntimeControl for RolloutRuntime {
    fn snapshot_ptys(&self) -> anyhow::Result<Vec<fractal_daemon::reconcile::RuntimeObservation>> {
        Ok(self
            .observations
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect())
    }
    fn observe_exec(&self, id: &str) -> anyhow::Result<Option<fractal_daemon::reconcile::RuntimeObservation>> {
        Ok(self.observations.lock().unwrap().get(id).cloned())
    }
    fn start(&self, member: &fractal_daemon::model::MemberSpec) -> anyhow::Result<()> {
        let mut starts = self.starts.lock().unwrap();
        starts.push(member.clone());
        self.observations.lock().unwrap().insert(
            member.runtime_id.clone(),
            fractal_daemon::reconcile::RuntimeObservation {
                runtime_id: member.runtime_id.clone(),
                terminal: member.terminal,
                status: "running".into(),
                incarnation_id: Some(format!("replacement-{}", starts.len())),
                exit_code: None,
            },
        );
        Ok(())
    }
    fn start_operation(
        &self,
        id: &str,
        terminal: bool,
        incarnation: &str,
    ) -> anyhow::Result<Option<String>> {
        let matches = self
            .observations
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|observed| {
                observed.terminal == terminal
                    && observed.incarnation_id.as_deref() == Some(incarnation)
            });
        if !matches {
            return Ok(None);
        }
        Ok(self
            .starts
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|member| member.runtime_id == id && member.terminal == terminal)
            .and_then(|member| member.tags.get("st3.start-operation"))
            .cloned())
    }
    fn stop(&self, id: &str, _: bool, expected: Option<&str>) -> anyhow::Result<()> {
        let mut observations = self.observations.lock().unwrap();
        let observed = observations.get_mut(id).unwrap();
        anyhow::ensure!(
            observed.incarnation_id.as_deref() == expected,
            "stale incarnation"
        );
        self.stops.lock().unwrap().push(expected.unwrap().into());
        observed.status = "exited".into();
        Ok(())
    }
    fn kill(&self, id: &str, terminal: bool, expected: Option<&str>) -> anyhow::Result<()> {
        self.stop(id, terminal, expected)
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

fn native_observation(store: &Store, subject: &str, incarnation: &str) {
    // The fixture producer retries on each reconciliation pass, just as the native
    // report waits for the graph to acknowledge the newly started runtime.
    if !store.latest_claim(subject, Some("runtime.observed")).unwrap()
        .is_some_and(|runtime| runtime.body["fields"]["status"] == "running"
            && runtime.body["fields"]["incarnation_id"].as_str() == Some(incarnation))
    {
        return;
    }
    append(
        store,
        subject,
        "harness.session-file",
        json!({"harness":"claude", "session_id":format!("native-{}", subject.replace('/', "-")), "incarnation_id":incarnation}),
    );
    append(
        store,
        subject,
        "harness.observed",
        json!({"state":"idle", "driver":"claude", "incarnation_id":incarnation, "quiescent":true}),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_rollout_publishes_with_automatic_member_and_only_moves_on_explicit_cli_request() {
    use fractal_daemon::reconcile::{Reconciler, RuntimeControl, RuntimeObservation};
    let root = tempfile::tempdir().unwrap();
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", anchor.clone(), &anchor).await;
    append(
        &d.store,
        "daemon/amber",
        "daemon.started",
        json!({"status":"running", "features":{"owned_sets":1,"seat_rollout":1,"seat_rollout_manual":1}}),
    );
    let runtime = Arc::new(RolloutRuntime::default());
    let reconciler = Reconciler::new(
        d.store.clone(),
        runtime.clone(),
        "amber".into(),
        Arc::new(Notify::new()),
    );
    let manual = "agent/garden/orchard";
    let automatic = "agent/garden/meadow";
    let bundle = |model: &str, manual_policy: bool| {
        let mut source = "version 2\n".to_owned();
        for (name, held) in [("orchard", manual_policy), ("meadow", false)] {
            let workspace = root.path().join(name);
            std::fs::create_dir_all(&workspace).unwrap();
            source.push_str(&format!("agent \"garden/{name}\" {{ host \"amber\"; workspace {:?}; {} harness \"claude\" {{ model {model:?}; }}; render {{ file \"active-model\" {model:?}; }} }}\n",
                workspace.display().to_string(), if held {"rollout \"manual\";"} else {""}));
        }
        source
    };
    let mut initial = request(&d, 1, bundle("first", true)).await;
    initial.options.rollout = Some(fractal_daemon::rollout::Policy::when_idle(1_800_000, false));
    preview(&d, &mut initial).await;
    d.client
        .post::<_, Value>("/v1/sets/apply", &initial)
        .await
        .unwrap();
    // Seed the two running incarnations as independent runtime evidence, not status synthesis.
    for subject in [manual, automatic] {
        let desired = d
            .store
            .desired_subject_with_writer(subject)
            .unwrap()
            .unwrap()
            .0;
        let member = desired.member.unwrap();
        let incarnation = format!("original-{subject}");
        runtime.observations.lock().unwrap().insert(
            member.runtime_id.clone(),
            RuntimeObservation {
                runtime_id: member.runtime_id.clone(),
                terminal: true,
                status: "running".into(),
                incarnation_id: Some(incarnation.clone()),
                exit_code: None,
            },
        );
        append(
            &d.store,
            subject,
            "runtime.action.succeeded",
            json!({"action":"start", "desired_token":d.store.selected_desired_token(subject).unwrap().unwrap(), "incarnation_id":incarnation}),
        );
        append(
            &d.store,
            subject,
            "runtime.observed",
            json!({"status":"running", "host":"amber", "runtime_id":member.runtime_id, "terminal":true,"incarnation_id":incarnation}),
        );
        native_observation(&d.store, subject, &incarnation);
    }
    reconciler.reconcile_once().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("orchard/active-model")).unwrap(),
        "first"
    );
    let unpublished = d
        .client
        .get::<Value>(&format!("/v1/client/sets/garden?sha={:040x}", 2))
        .await
        .unwrap_err();
    assert!(
        unpublished.to_string().contains("no receipt"),
        "{unpublished}"
    );
    assert!(
        d.client
            .get::<Value>("/v1/client/sets/garden")
            .await
            .is_ok()
    );
    let mut change = request(&d, 2, bundle("second", true)).await;
    change.options.rollout = initial.options.rollout.clone();
    let p = preview(&d, &mut change).await;
    assert!(p.effects[manual].contains("pending (manual)"));
    let change_digest = p.digest.clone();
    d.client
        .post::<_, Value>("/v1/sets/apply", &change)
        .await
        .unwrap();
    for _ in 0..8 {
        reconciler.reconcile_once().unwrap();
        if let Some(operation) = d.store.rollout(automatic).unwrap()
            && operation.drain_ack.is_none()
        {
            fractal_daemon::rollout::phase(&d.store, automatic, &operation, "drain-ack", None, &[]).unwrap();
        }
        for observation in runtime.snapshot_ptys().unwrap() {
            if observation.status == "running"
                && observation
                    .incarnation_id
                    .as_deref()
                    .unwrap()
                    .starts_with("replacement-")
            {
                native_observation(
                    &d.store,
                    automatic,
                    observation.incarnation_id.as_deref().unwrap(),
                );
            }
        }
    }
    assert_eq!(
        d.store.rollout(automatic).unwrap().unwrap().phase,
        "running"
    );
    assert!(d.store.rollout(manual).unwrap().is_none());
    assert_eq!(runtime.starts.lock().unwrap().len(), 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("orchard/active-model")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("meadow/active-model")).unwrap(),
        "second"
    );
    assert_eq!(
        *runtime.stops.lock().unwrap(),
        vec![format!("original-{automatic}")]
    );
    assert!(
        fractal_daemon::rollout::hold_render(
            &d.store,
            &d.store
                .desired_subject_with_writer(manual)
                .unwrap()
                .unwrap()
                .0
        )
        .unwrap()
    );
    let status: Value = d
        .client
        .get(&format!("/v1/client/sets/garden?sha={:040x}", 2))
        .await
        .unwrap();
    assert_eq!(status["commit_status"]["published"], true);
    assert_eq!(status["commit_status"]["satisfied"], true);
    assert_eq!(status["commit_status"]["running"], false);
    let member = status["members_status"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["subject"] == manual)
        .unwrap();
    assert_eq!(member["rollout"], "pending");
    assert_eq!(member["rollout_mode"], "manual");
    assert_eq!(
        member["publication_status"],
        "published, rollout pending (manual)"
    );
    assert_eq!(member["incarnation"], format!("original-{manual}"));
    serde_json::from_value::<fractal_client::Resource>(status).unwrap();
    let agent: Value = d
        .client
        .get(&format!("/v1/client/agents/{manual}"))
        .await
        .unwrap();
    assert_eq!(agent["state"], "running");
    assert_eq!(agent["rollout"]["mode"], "manual");
    assert_eq!(
        agent["rollout"]["status"],
        "published, rollout pending (manual)"
    );
    serde_json::from_value::<fractal_client::Resource>(agent).unwrap();
    // Neither restarting nor suspension may sneak around the pending publication.
    for path in ["/v1/agents/restart", "/v1/agents/suspend"] {
        let error = d
            .client
            .post::<_, Value>(
                path,
                &json!({"subject":manual,"actor":"person/operator","idempotency_key":path}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rollout-in-progress"), "{error}");
    }
    let socket = d.client.socket_path().unwrap().to_path_buf();
    let (shown, output) = tokio::task::spawn_blocking(move || {
        let command = || {
            let mut command =
                fractal_daemon::test_support::command(test_bin!("fractal-fixture"));
            command.env_remove("ST_AGENT").env_remove("FRACTAL_AGENT")
                .env_remove("ST_MISSION_RUN")
                .args(["--endpoint"])
                .arg(&socket);
            command
        };
        let shown = command().args(["agents", "show", manual]).output().unwrap();
        let rolled = command()
            .args([
                "--json",
                "agents",
                "rollout",
                manual,
                "--as",
                "person/operator",
            ])
            .output()
            .unwrap();
        (shown, rolled)
    })
    .await
    .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert!(String::from_utf8_lossy(&shown.stdout).contains("published, rollout pending (manual)"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..8 {
        reconciler.reconcile_once().unwrap();
        let operation = d.store.rollout(manual).unwrap().unwrap();
        if operation.drain_ack.is_none() {
            fractal_daemon::rollout::phase(&d.store, manual, &operation, "drain-ack", None, &[]).unwrap();
        }
        if let Some(incarnation) = operation.replacement_incarnation {
            native_observation(&d.store, manual, &incarnation);
        }
    }
    assert_eq!(d.store.rollout(manual).unwrap().unwrap().phase, "running");
    assert_eq!(runtime.starts.lock().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(root.path().join("orchard/active-model")).unwrap(),
        "second"
    );
    let final_status: Value = d
        .client
        .get(&format!("/v1/client/sets/garden?sha={:040x}", 2))
        .await
        .unwrap();
    assert_eq!(final_status["commit_status"]["running"], true);
    assert_eq!(final_status["commit_status"]["satisfied"], true);
    // Declaration-only policy removal is a normal reviewed change; it does not relaunch.
    let mut remove_policy = request(&d, 3, bundle("second", false)).await;
    remove_policy.options.rollout = initial.options.rollout.clone();
    let p = preview(&d, &mut remove_policy).await;
    assert_eq!(p.changes[manual], "changed");
    assert!(p.effects[manual].contains("launch unchanged"));
    assert_ne!(p.digest, change_digest);
    d.client
        .post::<_, Value>("/v1/sets/apply", &remove_policy)
        .await
        .unwrap();
    reconciler.reconcile_once().unwrap();
    assert_eq!(runtime.starts.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_authored_resume_initial_adoption_preserves_the_incumbent() {
    manual_authored_selection_publication(
        "--resume",
        "\"authored-session\"",
        ManualPublicationCase::Adoption,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_authored_continue_initial_adoption_preserves_the_incumbent() {
    manual_authored_selection_publication("--continue", "", ManualPublicationCase::Adoption).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_authored_selection_metadata_only_publish_preserves_the_incumbent() {
    for (selector, argument) in [("--resume", "\"authored-session\""), ("--continue", "")] {
        manual_authored_selection_publication(selector, argument, ManualPublicationCase::Metadata)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_authored_selection_launch_change_is_pending_and_cli_rollout_refuses_before_cutover() {
    for (selector, argument) in [("--resume", "\"authored-session\""), ("--continue", "")] {
        manual_authored_selection_publication(selector, argument, ManualPublicationCase::LaunchChange)
            .await;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManualPublicationCase {
    Adoption,
    Metadata,
    LaunchChange,
}

async fn manual_authored_selection_publication(
    selector: &'static str,
    argument: &str,
    case: ManualPublicationCase,
) {
    use fractal_daemon::reconcile::{Reconciler, RuntimeControl, RuntimeObservation};

    let root = tempfile::tempdir().unwrap();
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", anchor.clone(), &anchor).await;
    append(
        &d.store,
        "daemon/amber",
        "daemon.started",
        json!({"status":"running", "features":{"owned_sets":1,"seat_rollout":1,"seat_rollout_manual":1}}),
    );
    let subject = "agent/garden/orchard";
    let workspace = root.path().join("orchard");
    std::fs::create_dir_all(&workspace).unwrap();
    let bundle = |model: &str, name: &str, manual: bool, authored: bool| {
        format!(
            "version 2\nagent \"garden/orchard\" {{ host \"amber\"; workspace {:?}; name {name:?}; {} harness \"claude\" {{ model {model:?}; {} }}; render {{ file \"active-model\" {model:?}; }} }}\n",
            workspace.display().to_string(),
            if manual { "rollout \"manual\";" } else { "" },
            if authored {
                format!("args {selector:?} {argument};")
            } else {
                String::new()
            },
        )
    };
    let authored_adoption = case == ManualPublicationCase::Adoption;
    let unmanaged = fractal_daemon::parse_intent(
        &bundle("first", "unmanaged", false, authored_adoption),
        "amber",
    )
    .unwrap();
    d.store
        .apply_internal(&unmanaged, "unmanaged-fixture")
        .unwrap();
    let member = unmanaged.subjects[subject].member.as_ref().unwrap();
    let mut incarnation = "original-orchard";
    let mut launched_token = d.store.selected_desired_token(subject).unwrap().unwrap();
    let runtime = Arc::new(RolloutRuntime::default());
    runtime.observations.lock().unwrap().insert(
        member.runtime_id.clone(),
        RuntimeObservation {
            runtime_id: member.runtime_id.clone(),
            terminal: true,
            status: "running".into(),
            incarnation_id: Some(incarnation.into()),
            exit_code: None,
        },
    );
    append(
        &d.store,
        subject,
        "runtime.action.succeeded",
        json!({"action":"start","desired_token":launched_token,"incarnation_id":incarnation}),
    );
    append(
        &d.store,
        subject,
        "runtime.observed",
        json!({"status":"running","host":"amber","runtime_id":member.runtime_id,"terminal":true,"incarnation_id":incarnation}),
    );
    native_observation(&d.store, subject, incarnation);
    let reconciler = Reconciler::new(
        d.store.clone(),
        runtime.clone(),
        "amber".into(),
        Arc::new(Notify::new()),
    );
    reconciler.reconcile_once().unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.join("active-model")).unwrap(),
        "first"
    );

    // Adoption adds ownership and manual policy without replacing the authored launch.
    let mut adoption = request(&d, 1, bundle("first", "managed", true, authored_adoption)).await;
    adoption.options.adopt.insert(subject.into());
    let adoption_preview = preview(&d, &mut adoption).await;
    assert_eq!(adoption_preview.changes[subject], "added");
    let applied: Value = d
        .client
        .post("/v1/sets/apply", &adoption)
        .await
        .unwrap();
    assert_eq!(applied["publication"]["changed"], true);
    assert!(d.store.owned_sets().unwrap()[0].receipt.adoptions.contains_key(subject));
    reconciler.reconcile_once().unwrap();
    let adopted = d
        .store
        .desired_subject_with_writer(subject)
        .unwrap()
        .unwrap()
        .0;
    assert!(fractal_daemon::rollout::manual(&adopted));
    assert_eq!(adopted.member.as_ref().unwrap().launch, member.launch);
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());
    assert!(d.store.rollout(subject).unwrap().is_none());
    if case == ManualPublicationCase::Adoption {
        let observations = runtime.snapshot_ptys().unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].status, "running");
        assert_eq!(observations[0].incarnation_id.as_deref(), Some(incarnation));
        return;
    }

    if case == ManualPublicationCase::Metadata {
        // Begin with a resumable owned set so this publication has its own negative control.
        let mut authored = request(&d, 2, bundle("first", "managed", true, true)).await;
        preview(&d, &mut authored).await;
        d.client
            .post::<_, Value>("/v1/sets/apply", &authored)
            .await
            .unwrap();
        // Independent runtime evidence supplies the authored-selector incumbent for the
        // metadata-only assertion, without pretending the reconciler rolled it out.
        incarnation = "authored-orchard";
        launched_token = d.store.selected_desired_token(subject).unwrap().unwrap();
        let authored_member = d
            .store
            .desired_subject_with_writer(subject)
            .unwrap()
            .unwrap()
            .0
            .member
            .unwrap();
        runtime
            .observations
            .lock()
            .expect("fixture mutex")
            .remove(&member.runtime_id);
        runtime.observations.lock().expect("fixture mutex").insert(
            authored_member.runtime_id.clone(),
            RuntimeObservation {
                runtime_id: authored_member.runtime_id.clone(),
                terminal: authored_member.terminal,
                status: "running".into(),
                incarnation_id: Some(incarnation.into()),
                exit_code: None,
            },
        );
        append(
            &d.store,
            subject,
            "runtime.action.succeeded",
            json!({"action":"start","desired_token":launched_token,"incarnation_id":incarnation}),
        );
        append(
            &d.store,
            subject,
            "runtime.observed",
            json!({"status":"running","host":authored_member.host,"runtime_id":authored_member.runtime_id,"terminal":authored_member.terminal,"incarnation_id":incarnation}),
        );
        native_observation(&d.store, subject, incarnation);
        reconciler.reconcile_once().unwrap();

        // Only the presentation name changes, which shares launch lineage with its
        // predecessor; the authored selector and launch remain unchanged.
        let mut metadata = request(&d, 3, bundle("first", "metadata-only", true, true)).await;
        let metadata_preview = preview(&d, &mut metadata).await;
        assert_eq!(metadata_preview.changes[subject], "changed");
        assert!(metadata_preview.effects[subject].contains("launch unchanged"));
        d.client
            .post::<_, Value>("/v1/sets/apply", &metadata)
            .await
            .unwrap();
        reconciler.reconcile_once().unwrap();
        let metadata_status: Value = d
            .client
            .get(&format!("/v1/client/sets/garden?sha={:040x}", 3))
            .await
            .unwrap();
        assert_eq!(metadata_status["commit_status"]["published"], true);
        assert_eq!(metadata_status["commit_status"]["running"], true);
        let metadata_member = &metadata_status["members_status"][0];
        assert_eq!(metadata_member["launch_current"], true);
        assert_eq!(metadata_member["incarnation"], incarnation);
        assert_eq!(metadata_member["launched_token"], launched_token);
        assert_ne!(
            metadata_member["desired_token"],
            metadata_member["launched_token"]
        );
        assert!(d.store.rollout(subject).unwrap().is_none());
        assert!(runtime.starts.lock().expect("fixture mutex").is_empty());
        assert!(runtime.stops.lock().expect("fixture mutex").is_empty());
        return;
    }

    let mut launch_change = request(&d, 2, bundle("second", "managed", true, true)).await;
    let change_preview = preview(&d, &mut launch_change).await;
    assert!(change_preview.effects[subject].contains("pending (manual)"));
    d.client
        .post::<_, Value>("/v1/sets/apply", &launch_change)
        .await
        .unwrap();
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    let pending: Value = d
        .client
        .get(&format!("/v1/client/sets/garden?sha={:040x}", 2))
        .await
        .unwrap();
    assert_eq!(pending["commit_status"]["published"], true);
    assert_eq!(pending["commit_status"]["satisfied"], true);
    assert_eq!(pending["commit_status"]["running"], false);
    assert_eq!(pending["members_status"][0]["rollout"], "pending");
    assert_eq!(
        pending["members_status"][0]["publication_status"],
        "published, rollout pending (manual)"
    );
    assert_eq!(pending["members_status"][0]["incarnation"], incarnation);
    assert_eq!(
        std::fs::read_to_string(workspace.join("active-model")).unwrap(),
        "first"
    );
    assert!(d.store.rollout(subject).unwrap().is_none());
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());

    let socket = d.client.socket_path().unwrap().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        fractal_daemon::test_support::command(test_bin!("fractal-fixture")).env_remove("ST_AGENT").env_remove("FRACTAL_AGENT")
            .env_remove("ST_MISSION_RUN")
            .args(["--endpoint"])
            .arg(socket)
            .args(["agents", "rollout", subject, "--as", "person/operator"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("authored-session-selection"), "{error}");
    assert!(error.contains("cannot resume"), "{error}");
    assert!(error.contains(selector), "{error}");
    let api_error = d
        .client
        .post::<_, Value>(
            "/v1/agents/rollout",
            &json!({
                "subject":subject,
                "actor":"person/operator",
                "expected_desired":d.store.selected_desired_token(subject).unwrap().unwrap(),
                "expected_incarnation":incarnation,
                "policy":fractal_daemon::rollout::Policy::when_idle(1_800_000, false),
                "idempotency_key":"unsupported-manual-rollout",
            }),
        )
        .await
        .unwrap_err();
    assert!(api_error.to_string().contains("authored-session-selection"), "{api_error}");
    assert!(api_error.to_string().contains("cannot resume"), "{api_error}");
    assert!(api_error.to_string().contains(selector), "{api_error}");

    // A refused request must not leave a durable drain or action for reconciliation.
    assert!(
        d.store
            .latest_claim(subject, Some("runtime.action.requested"))
            .unwrap()
            .is_none()
    );
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert!(d.store.rollout(subject).unwrap().is_none());
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());
    let observations = runtime.snapshot_ptys().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].status, "running");
    assert_eq!(observations[0].incarnation_id.as_deref(), Some(incarnation));
    assert_eq!(
        std::fs::read_to_string(workspace.join("active-model")).unwrap(),
        "first"
    );
    let agent: Value = d
        .client
        .get(&format!("/v1/client/agents/{subject}"))
        .await
        .unwrap();
    assert_eq!(agent["state"], "running");
    assert_eq!(agent["rollout"]["status"], "published, rollout pending (manual)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_publication_preserves_suspension_and_requires_resume_before_rollout() {
    let root = tempfile::tempdir().unwrap();
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", anchor.clone(), &anchor).await;
    append(
        &d.store,
        "daemon/amber",
        "daemon.started",
        json!({"status":"running", "features":{"owned_sets":1,"seat_rollout":1,"seat_rollout_manual":1}}),
    );
    let subject = "agent/garden/orchard";
    let workspace = root.path().join("orchard");
    std::fs::create_dir_all(&workspace).unwrap();
    let bundle = |model: &str| {
        format!("version 2\nagent \"garden/orchard\" {{ host \"amber\"; workspace {:?}; rollout \"manual\"; harness \"claude\" {{ model {model:?}; }} }}\n",
            workspace.display().to_string())
    };
    let mut initial = request(&d, 1, bundle("first")).await;
    preview(&d, &mut initial).await;
    d.client.post::<_, Value>("/v1/sets/apply", &initial).await.unwrap();
    let incumbent = d.store.selected_desired_token(subject).unwrap().unwrap();
    let member = d.store.desired_subject_with_writer(subject).unwrap().unwrap().0.member.unwrap();
    append(&d.store, subject, "runtime.action.succeeded",
        json!({"action":"start","desired_token":incumbent,"incarnation_id":"original"}));
    append(&d.store, subject, "runtime.observed",
        json!({"status":"running","host":"amber","runtime_id":member.runtime_id,"terminal":true,"incarnation_id":"original"}));
    native_observation(&d.store, subject, "original");
    d.client.post::<_, Value>("/v1/agents/suspend",
        &json!({"subject":subject,"actor":"person/operator","idempotency_key":"suspend-before-publication"}))
        .await.unwrap();
    let operation = fractal_daemon::suspension::current(&d.store, subject).unwrap().unwrap();
    assert_eq!(operation.phase, "quiescing");

    // Publish while suspend is in flight. Owner acknowledgements model the stopped
    // runtime boundary; native process continuity is covered by the lifecycle tests.
    let mut changed = request(&d, 2, bundle("second")).await;
    preview(&d, &mut changed).await;
    d.client.post::<_, Value>("/v1/sets/apply", &changed).await.unwrap();
    assert_eq!(fractal_daemon::suspension::current(&d.store, subject).unwrap().unwrap().operation_id, operation.operation_id);
    for (key, phase) in [
        (fractal_daemon::suspension::suspend_snapshot_key(&operation.operation_id), "snapshotting"),
        (fractal_daemon::suspension::suspend_completed_key(&operation.operation_id), "suspended"),
    ] {
        d.store.append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "runtime.action.succeeded".into(),
            actor: Some("person/operator".into()),
            fields: serde_json::from_value(json!({"action":"suspend","operation_status":phase,"harness":"claude","native_session_id":"native-agent-garden-orchard"})).unwrap(),
            evidence: vec![operation.operation_id.clone()],
            expected_subject: None,
            idempotency_key: Some(key),
        }).unwrap();
    }
    append(&d.store, subject, "runtime.observed",
        json!({"status":"exited","host":"amber","runtime_id":member.runtime_id,"terminal":true,"incarnation_id":"original","exit_code":0}));
    let runtime = Arc::new(RolloutRuntime::default());
    let reconciler = fractal_daemon::reconcile::Reconciler::new(
        d.store.clone(), runtime.clone(), "amber".into(), Arc::new(Notify::new()));
    reconciler.reconcile_once().unwrap();
    assert!(runtime.starts.lock().unwrap().is_empty());
    let shown: Value = d.client.get(&format!("/v1/client/agents/{subject}")).await.unwrap();
    assert_eq!(shown["state"], "suspended");
    assert_eq!(shown["rollout"]["mode"], "manual");
    assert_eq!(shown["rollout"]["status"], "published, rollout pending (manual)");
    let refused = d.client.post::<_, Value>("/v1/agents/rollout",
        &json!({"subject":subject,"actor":"person/operator","expected_desired":d.store.selected_desired_token(subject).unwrap().unwrap(),"expected_incarnation":"original","policy":fractal_daemon::rollout::Policy::when_idle(1_800_000, false),"idempotency_key":"rollout-while-suspended"}))
        .await.unwrap_err();
    assert!(refused.to_string().contains("rollout-suspended"), "{refused}");
    assert!(d.store.rollout(subject).unwrap().is_none());
    d.client.post::<_, Value>("/v1/agents/resume",
        &json!({"subject":subject,"actor":"person/operator","idempotency_key":"resume-incumbent"}))
        .await.unwrap();
    let resumed = fractal_daemon::suspension::current(&d.store, subject).unwrap().unwrap();
    assert_eq!(resumed.phase, "restoring");
    assert_eq!(resumed.native_session_id.as_deref(), Some("native-agent-garden-orchard"));
    assert_ne!(d.store.selected_desired_token(subject).unwrap().unwrap(), incumbent);
    assert!(d.store.rollout(subject).unwrap().is_none());
    d.store.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: "runtime.action.succeeded".into(),
        actor: Some("person/operator".into()),
        fields: serde_json::from_value(json!({"action":"resume","operation_status":"resumed","harness":"claude","native_session_id":"native-agent-garden-orchard","incarnation_id":"resumed-incumbent"})).unwrap(),
        evidence: vec![resumed.operation_id.clone()],
        expected_subject: None,
        idempotency_key: Some(fractal_daemon::suspension::resume_completed_key(&resumed.operation_id)),
    }).unwrap();
    append(&d.store, subject, "runtime.action.succeeded",
        json!({"action":"start","desired_token":incumbent,"incarnation_id":"resumed-incumbent"}));
    append(&d.store, subject, "runtime.observed",
        json!({"status":"running","host":"amber","runtime_id":member.runtime_id,"terminal":true,"incarnation_id":"resumed-incumbent"}));
    native_observation(&d.store, subject, "resumed-incumbent");
    let shown: Value = d.client.get(&format!("/v1/client/agents/{subject}")).await.unwrap();
    assert_eq!(shown["state"], "running");
    assert_eq!(shown["rollout"]["status"], "published, rollout pending (manual)");
    d.client.post::<_, Value>("/v1/agents/rollout",
        &json!({"subject":subject,"actor":"person/operator","expected_desired":d.store.selected_desired_token(subject).unwrap().unwrap(),"expected_incarnation":"resumed-incumbent","policy":fractal_daemon::rollout::Policy::when_idle(1_800_000, false),"idempotency_key":"rollout-after-resume"}))
        .await.unwrap();
    assert_eq!(d.store.rollout(subject).unwrap().unwrap().phase, "draining");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_declaration_diffs_match_publication_reads_and_mission_previews() {
    use fractal_daemon::model::{MissionRequest, MissionResponse};
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let source = |note: &str, interval: &str| {
        format!(
            r#"version 2
agent "garden/orchard" {{ command "true"; description "{note}"; env {{ TOKEN "fixture-value"; }} }}
mission "garden/harvest" state="ready" timeout="1m" {{
    goal "{note}"
    step "inspect" {{ agentless }}
}}
schedule "garden/daily" {{
    host "amber"
    every "{interval}"
    anchor "2026-01-01T00:00:00Z"
    work {{ mission "garden/harvest"; workspace "/tmp"; }}
}}
"#
        )
    };
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../../docs/st3/client-v0/schemas/client-v0.schema.json"
    ))
    .unwrap();
    schema["oneOf"] = json!([{ "$ref": "#/$defs/PublicationDefinition" }]);
    let validator = jsonschema::options().build(&schema).unwrap();
    let mut first = request(&d, 1, source("first", "6h")).await;
    let p = preview(&d, &mut first).await;
    assert_eq!(p.declaration_diffs.len(), 3);
    assert!(
        p.declaration_diffs
            .values()
            .all(|diff| diff.before.is_none())
    );
    assert_eq!(
        p.declaration_diffs["mission/garden/harvest"].after["max_active_runs"],
        1
    );
    assert_eq!(
        p.declaration_diffs["mission/garden/harvest"].after["revision_cutover"],
        "restart-active"
    );
    let mission = &p.declaration_diffs["mission/garden/harvest"].after;
    assert_eq!(mission["timeout_ms"], 60_000);
    assert_eq!(
        mission["steps"]["inspect"]["retry"],
        json!({"attempts": 1, "backoff_ms": 0})
    );
    d.client
        .post::<_, Value>("/v1/sets/apply", &first)
        .await
        .unwrap();
    let read = |subject: &str| {
        format!(
            "/v1/client/publication-definition?subject={}",
            urlencoding::encode(subject)
        )
    };
    for (subject, diff) in &p.declaration_diffs {
        let response: Value = d.client.get(&read(subject)).await.unwrap();
        assert!(
            validator.is_valid(&response),
            "{:?}",
            validator
                .iter_errors(&response)
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(response["declaration"], diff.after);
        assert_eq!(response["subject"], *subject);
    }
    let mut updated = request(&d, 2, source("second", "12h")).await;
    let changed = preview(&d, &mut updated).await;
    let ordinary: MissionResponse = d
        .client
        .post(
            "/v1/intent/mission",
            &MissionRequest {
                intent: IntentInput {
                    kdl: updated
                        .intent
                        .kdl
                        .split("\nschedule ")
                        .next()
                        .unwrap()
                        .into(),
                    source_name: None,
                },
                at_index: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(ordinary.declaration_diffs.len(), 2);
    for (subject, diff) in &ordinary.declaration_diffs {
        assert_eq!(
            serde_json::to_value(diff).unwrap(),
            serde_json::to_value(&changed.declaration_diffs[subject]).unwrap()
        );
    }
    assert!(
        changed.declaration_diffs["mission/garden/harvest"]
            .fields
            .contains("/goals")
    );
    for (subject, diff) in &changed.declaration_diffs {
        assert_eq!(
            diff.before.as_ref(),
            Some(&p.declaration_diffs[subject].after)
        );
        assert!(!diff.fields.is_empty());
    }
    d.client
        .post::<_, Value>("/v1/sets/apply", &updated)
        .await
        .unwrap();
    for (subject, diff) in &changed.declaration_diffs {
        let response: Value = d.client.get(&read(subject)).await.unwrap();
        assert!(
            validator.is_valid(&response),
            "{:?}",
            validator
                .iter_errors(&response)
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(response["declaration"], diff.after);
    }
    let mut same = request(&d, 3, source("second", "12h")).await;
    assert!(preview(&d, &mut same).await.declaration_diffs.is_empty());
    let mut empty = request(&d, 3, "version 2\n".into()).await;
    empty.options.allow_empty = true;
    let retire = preview(&d, &mut empty).await;
    assert_eq!(retire.declaration_diffs.len(), 3);
    assert_eq!(
        retire.declaration_diffs["mission/garden/harvest"].after["state"],
        "retired"
    );
    empty.options.confirm_retire = Some(retire.digest.clone());
    d.client
        .post::<_, Value>("/v1/sets/apply", &empty)
        .await
        .unwrap();
    for (subject, diff) in &retire.declaration_diffs {
        let response: Value = d.client.get(&read(subject)).await.unwrap();
        assert!(
            validator.is_valid(&response),
            "{:?}",
            validator
                .iter_errors(&response)
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(response["declaration"], diff.after);
        assert_eq!(
            diff.before.as_ref(),
            Some(&changed.declaration_diffs[subject].after)
        );
    }
    let mut restored = request(&d, 4, source("second", "12h")).await;
    let restore = preview(&d, &mut restored).await;
    for (subject, diff) in &restore.declaration_diffs {
        assert_eq!(restore.changes[subject], "added");
        assert_eq!(
            diff.before.as_ref(),
            Some(&retire.declaration_diffs[subject].after)
        );
        assert_eq!(diff.after, changed.declaration_diffs[subject].after);
    }
    // Old serialized previews still deserialize with an empty additive diff map.
    let mut legacy = serde_json::to_value(retire).unwrap();
    legacy.as_object_mut().unwrap().remove("declaration_diffs");
    assert!(
        serde_json::from_value::<Preview>(legacy)
            .unwrap()
            .declaration_diffs
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adoption_diff_reads_existing_unmanaged_definition_and_invalid_actors_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let initial = fractal_daemon::graph::parse_intent(&bundle("unmanaged", false), "amber").unwrap();
    d.store
        .apply_internal(&initial, "unmanaged-fixture")
        .unwrap();
    let mut adopt = request(&d, 1, bundle("managed", false)).await;
    adopt.options.adopt.insert("agent/garden/orchard".into());
    let p = preview(&d, &mut adopt).await;
    let before = &p.declaration_diffs["agent/garden/orchard"].before;
    assert_eq!(
        before.as_ref().unwrap(),
        &serde_json::to_value(&initial.subjects["agent/garden/orchard"]).unwrap()
    );
    adopt.actor = "daemon/not-an-actor".into();
    assert!(
        d.client
            .post::<_, Value>("/v1/sets/preview", &adopt)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspended_authored_selection_refuses_start_and_resumes_the_recorded_session() {
    use fractal_daemon::reconcile::{Reconciler, RuntimeControl};
    use fractal_daemon::suspension;

    let root = tempfile::tempdir().unwrap();
    let anchor = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", anchor.clone(), &anchor).await;
    let subject = "agent/garden/orchard";
    let native = "00000000-0000-4000-8000-000000000024";
    let transcript = root.path().join(format!("2026-10-24_{native}.jsonl"));
    std::fs::write(&transcript, format!("{{\"type\":\"session\",\"id\":\"{native}\"}}\n")).unwrap();
    let source = format!(
        "version 2\nagent \"garden/orchard\" {{ host \"amber\"; workspace {:?}; harness \"omp\" {{ args \"--resume\" {:?}; }} }}",
        root.path().display().to_string(), transcript.display().to_string(),
    );
    let intent = fractal_daemon::parse_intent(&source, "amber").unwrap();
    d.store.apply_internal(&intent, "suspended-authored-fixture").unwrap();
    let token = d.store.selected_desired_token(subject).unwrap().unwrap();
    // Reproduce a historical suspension: current suspend preflight prevents creating a new one
    // with an authored selector, but old durable suspensions still have to be resumable.
    let suspend = d.store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "runtime.action.requested".into(),
        actor: Some("person/operator".into()),
        fields: serde_json::from_value(json!({"action":"suspend","incarnation_id":"original"})).unwrap(),
        evidence: vec![token.clone()], expected_subject: None, idempotency_key: None,
    }).unwrap();
    for (key, phase) in [
        (suspension::suspend_snapshot_key(&suspend.id), "snapshotting"),
        (suspension::suspend_completed_key(&suspend.id), "suspended"),
    ] {
        d.store.append_claim(&ClaimInput {
            subject: subject.into(), kind: "runtime.action.succeeded".into(),
            actor: Some("person/operator".into()),
            fields: serde_json::from_value(json!({"action":"suspend","operation_status":phase,
                "harness":"omp","native_session_id":native})).unwrap(),
            evidence: vec![suspend.id.clone()], expected_subject: None, idempotency_key: Some(key),
        }).unwrap();
    }
    assert_eq!(suspension::current(&d.store, subject).unwrap().unwrap().phase, "suspended");
    let refused: fractal_daemon::model::ClaimRecord = d.client.post("/v1/agents/resume", &json!({
        "subject":subject,"actor":"person/operator","idempotency_key":"historical-refused-resume",
    })).await.unwrap();
    d.store.append_claim(&ClaimInput {
        subject: subject.into(), kind: "runtime.action.failed".into(),
        actor: Some("person/operator".into()),
        fields: serde_json::from_value(json!({"action":"resume","code":"authored-session-selection",
            "reason":"historical blanket authored selector refusal"})).unwrap(),
        evidence: vec![refused.id.clone()], expected_subject: None,
        idempotency_key: Some(suspension::resume_failed_key(&refused.id)),
    }).unwrap();
    let held = suspension::current(&d.store, subject).unwrap().unwrap();
    assert_eq!(held.phase, "suspended");
    assert_eq!(held.code.as_deref(), Some("authored-session-selection"));
    let socket = d.client.socket_path().unwrap().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        fractal_daemon::test_support::command(test_bin!("fractal-fixture"))
            .env_remove("ST_AGENT").env_remove("FRACTAL_AGENT").env_remove("ST_MISSION_RUN")
            .args(["--endpoint"]).arg(socket)
            .args(["agents", "start", "garden/orchard", "--as", "person/operator"])
            .output().unwrap()
    }).await.unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("start-suspended"), "{error}");
    assert!(error.contains("st agents resume"), "{error}");
    assert_eq!(d.store.selected_desired_token(subject).unwrap().unwrap(), token);

    let runtime = Arc::new(RolloutRuntime::default());
    let reconciler = Reconciler::new(d.store.clone(), runtime.clone(), "amber".into(), Arc::new(Notify::new()));
    d.client.post::<_, Value>("/v1/agents/resume", &json!({
        "subject":subject,"actor":"person/operator","idempotency_key":"resume-authored",
    })).await.unwrap();
    for _ in 0..4 { reconciler.reconcile_once().unwrap(); }
    let launch = runtime.starts.lock().unwrap().last().unwrap().clone();
    assert_eq!(launch.environment[suspension::RESUME_ENV], native);
    // Exercise the same argv check used by the driver, including the pinned-path branch.
    // The native_resume unit fixture separately runs the resulting argv through a stub harness.
    let argv = vec!["omp".into(), "--resume".into(), transcript.display().to_string()];
    let selected = fractal_daemon::native_resume::pi_family_argv(
        "omp", argv.clone(), &root.path().join("sessions"), native, Some(&transcript),
    ).unwrap();
    assert_eq!(selected, argv);
    let observation = runtime.snapshot_ptys().unwrap().pop().unwrap();
    append(&d.store, subject, "harness.session-file", json!({
        "harness":"omp","session_id":native,"incarnation_id":observation.incarnation_id,
        "path":transcript,"status":"active",
    }));
    for _ in 0..3 { reconciler.reconcile_once().unwrap(); }
    let resumed = suspension::current(&d.store, subject).unwrap().unwrap();
    assert_eq!(resumed.phase, "resumed");
    assert_eq!(resumed.native_session_id.as_deref(), Some(native));

    std::fs::write(&transcript, "{\"type\":\"session\",\"id\":\"other-native\"}\n").unwrap();
    assert_eq!(
        fractal_daemon::native_resume::pi_family_argv(
            "omp", argv, &root.path().join("sessions"), native, Some(&transcript),
        ).unwrap_err().code,
        "authored-session-mismatch",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_tasks_publish_with_their_agent_and_revision_tokens_pin_the_same_bundle() {
    let root = tempfile::tempdir().unwrap();
    let keys = (0..2)
        .map(|_| Arc::new(MemberKey::generate().unwrap().0))
        .collect::<Vec<_>>();
    let amber = daemon(root.path(), "amber", keys[0].clone(), &keys[0]).await;
    let ivory = daemon(root.path(), "ivory", keys[1].clone(), &keys[0]).await;
    for (i, name) in ["amber", "ivory"].into_iter().enumerate() {
        let mut fields = json!({"fleet_id":FLEET,"member_key":keys[i].public(),
            "via":if i==0{"anchor"}else{"invite"},"mode":"listening"});
        if i != 0 {
            fields["sponsor"] = json!("host/amber");
        }
        append(&amber.store, &format!("host/{name}"), "fleet.member-admitted", fields);
    }
    let exchange = |amber: &Daemon, ivory: &Daemon| {
        share(&amber.store, &ivory.store, "amber");
        share(&ivory.store, &amber.store, "ivory");
    };
    exchange(&amber, &ivory);
    // The full tree: an agent whose exec task compiles to its own subject, grouped `under` a
    // sibling agent, and a schedule pinning a mission of the same publication.
    let tree = |pump: bool| {
        let mut kdl = bundle("tasks", false);
        if pump {
            kdl.push_str(
                "agent \"garden/pump\" {\n workspace \".\"\n under \"agent/garden/orchard\" reason=\"The orchard runs its pump.\"\n exec \"flow\" { argv \"true\"; }\n}\n",
            );
        }
        kdl.push_str("mission \"garden/harvest\" state=\"ready\" { goal \"Harvest\"; step \"work\" { assigned-to \"agent/garden/orchard\" } }\n\
            schedule \"garden/daily\" { every \"6h\"; anchor \"2026-01-01T00:00:00Z\"; work { mission \"garden/harvest@ST_REVISION[garden/harvest]\"; workspace \"/tmp\"; } }\n");
        kdl
    };

    // Older daemons refuse replicated revisions carrying tasks, so activation waits for all.
    let mut publication = request(&amber, 10, tree(true)).await;
    let refused: Preview = amber
        .client
        .post("/v1/sets/preview", &publication)
        .await
        .unwrap();
    assert!(
        refused
            .blockers
            .iter()
            .any(|b| b == "host/ivory has not advertised owned-set agent task support; upgrade before activation"),
        "{:?}",
        refused.blockers
    );
    for (daemon, name) in [(&amber, "amber"), (&ivory, "ivory")] {
        append(
            &daemon.store,
            &format!("daemon/{name}"),
            "daemon.started",
            json!({"status":"running","features":{"owned_sets":1,"owned_set_tasks":1}}),
        );
    }
    exchange(&amber, &ivory);

    let planned = preview(&amber, &mut publication).await;
    for subject in [
        "agent/garden/orchard",
        "agent/garden/pump",
        "exec/garden/pump/flow",
        "mission/garden/harvest",
        "schedule/garden/daily",
    ] {
        assert_eq!(planned.changes[subject], "added", "{subject}: {planned:?}");
    }
    let _: Value = amber
        .client
        .post("/v1/sets/apply", &publication)
        .await
        .unwrap();
    exchange(&amber, &ivory);
    let revision = amber
        .store
        .mission_spec("garden/harvest", None)
        .unwrap()
        .unwrap()
        .revision;
    for daemon in [&amber, &ivory] {
        let selected = daemon.store.owned_sets().unwrap().remove(0);
        assert!(selected.blockers.is_empty(), "{:?}", selected.blockers);
        assert_eq!(selected.receipt.members["exec/garden/pump/flow"].kind, "exec");
        assert!(
            daemon
                .store
                .selected_desired_token("exec/garden/pump/flow")
                .unwrap()
                .is_some()
        );
        let schedule = daemon
            .store
            .desired_subject_with_writer("schedule/garden/daily")
            .unwrap()
            .unwrap()
            .0;
        let pinned = serde_json::to_string(&schedule.desired).unwrap();
        assert!(pinned.contains(&format!("garden/harvest@{revision}")), "{pinned}");
    }

    // Omitting the agent retires its task with it; the unchanged mission keeps its revision.
    let mut omit = request(&amber, 20, tree(false)).await;
    let planned = preview(&amber, &mut omit).await;
    assert_eq!(planned.changes["agent/garden/pump"], "retiring");
    assert_eq!(planned.changes["exec/garden/pump/flow"], "retiring");
    assert_eq!(planned.changes["mission/garden/harvest"], "unchanged");
    assert_eq!(planned.changes["schedule/garden/daily"], "unchanged");
    let _: Value = amber.client.post("/v1/sets/apply", &omit).await.unwrap();
    exchange(&amber, &ivory);
    for daemon in [&amber, &ivory] {
        let selected = daemon.store.owned_sets().unwrap().remove(0);
        assert!(selected.blockers.is_empty(), "{:?}", selected.blockers);
        assert!(selected.receipt.retired.contains_key("exec/garden/pump/flow"));
        let flow = daemon
            .store
            .desired_subject_with_writer("exec/garden/pump/flow")
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(flow.kind, "stop");
    }

    // A token naming a mission outside the publication refuses it outright.
    let foreign = request(
        &amber,
        30,
        tree(false).replace("ST_REVISION[garden/harvest]", "ST_REVISION[garden/elsewhere]"),
    )
    .await;
    let error = amber
        .client
        .post::<_, Preview>("/v1/sets/preview", &foreign)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unresolved-revision-token"), "{error}");
}

const HARVEST: &str = "mission \"garden/harvest\" state=\"ready\" { goal \"Harvest\"; step \"work\" { assigned-to \"agent/garden/orchard\" } }\n";
const ROSE: &str = "agent \"garden/rose\" {\n workspace \".\"\n harness \"codex\" {}\n}\n";
const LILY: &str = "agent \"garden/lily\" {\n workspace \".\"\n command \"true\"\n}\n";

/// The orchard and rose agents and the harvest mission, plus `extra` declarations.
fn enforced_bundle(extra: &str) -> String {
    format!("{}{ROSE}{HARVEST}{extra}", bundle("source", false))
}

fn enforce(mode: Enforce) -> PolicyRequest {
    PolicyRequest { enforce: Some(mode), ..Default::default() }
}

async fn publish_set(d: &Daemon, sequence: u64, kdl: String, policy: Option<PolicyRequest>) -> (Preview, Value) {
    let mut publication = request(d, sequence, kdl).await;
    publication.options.policy = policy;
    let preview = preview(d, &mut publication).await;
    let result: Value = d.client.post("/v1/sets/apply", &publication).await.unwrap();
    assert_eq!(result["publication"]["changed"], true, "{result}");
    (preview, result)
}

/// A plain `fractal apply`, optionally `--force --reason`.
async fn plain(d: &Daemon, kdl: &str, key: &str, force: Option<&str>) -> anyhow::Result<Value> {
    let planned: Value = d
        .client
        .post("/v1/intent/mission", &json!({"intent": {"kdl": kdl}, "at_index": null}))
        .await?;
    d.client
        .post("/v1/intent/apply", &json!({"intent": {"kdl": kdl}, "expected_subjects": planned["subject_tokens"],
            "idempotency_key": key, "actor": "person/operator", "force_reason": force}))
        .await
}

fn until() -> u64 {
    u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()).unwrap()
        + 900_000
}

async fn hold(d: &Daemon, key: &str, force: bool) -> anyhow::Result<Value> {
    d.client
        .post("/v1/delivery/hold", &json!({"subject": "agent/garden/rose", "actor": "person/operator",
            "held": true, "until_unix_ms": until(), "reason": "quiet during the demo",
            "idempotency_key": key, "force": force}))
        .await
}

async fn hold_active(d: &Daemon) -> bool {
    let held: Value = d.client.get("/v1/delivery/hold?subject=agent%2Fgarden%2Frose").await.unwrap();
    held["active"] == true
}

async fn source_status(d: &Daemon, audit: bool) -> Value {
    d.client
        .get(if audit { "/v1/source/status?audit=true" } else { "/v1/source/status" })
        .await
        .unwrap()
}

fn assert_contains(text: &str, expected: &[&str]) {
    for expected in expected {
        assert!(text.contains(expected), "{expected:?} missing from: {text}");
    }
}

/// An `enforce` set refuses each ad hoc desired-state write to a member, naming the verb, the
/// set, its source and the escape. A set without a policy keeps today's refusal, with no force.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enforced_set_refuses_ad_hoc_writes_naming_each_verb() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    publish_set(&d, 10, enforced_bundle(""), Some(enforce(Enforce::Enforce))).await;
    let source = [
        "set-managed-subject", "owned-set/garden from acme/garden refs/heads/main",
        "enforce enforce", "--force --reason TEXT",
    ];
    let refused = plain(&d, &bundle("ad hoc", false), "apply", None).await.unwrap_err().to_string();
    assert_contains(&refused, &source);
    assert_contains(&refused, &["ad hoc apply is refused"]);
    let refused = plain(&d, "version 2\nstop \"agent/garden/orchard\"\n", "stop", None).await.unwrap_err().to_string();
    assert_contains(&refused, &source);
    assert_contains(&refused, &["ad hoc stop is refused"]);
    let refused = hold(&d, "hold", false).await.unwrap_err().to_string();
    assert_contains(&refused, &source);
    assert_contains(&refused, &["ad hoc hold is refused"]);
    assert!(!hold_active(&d).await);
    let refused = d
        .client
        .post::<_, Value>("/v1/missions/garden%2Fharvest/retire",
            &json!({"actor": "person/operator", "idempotency_key": "retire-harvest"}))
        .await
        .unwrap_err()
        .to_string();
    assert_contains(&refused, &source);
    assert_contains(&refused, &["ad hoc mission retire is refused"]);
    let refused = plain(&d, &bundle("ad hoc", false), "blank", Some("  ")).await.unwrap_err().to_string();
    assert_contains(&refused, &["source-force-refused", "nonempty --reason"]);
    assert!(source_status(&d, false).await["open"].as_array().unwrap().is_empty());

    // `off` is today's behaviour: the refusal stands and there is no force path.
    publish_set(&d, 11, enforced_bundle(""), Some(enforce(Enforce::Off))).await;
    assert!(d.store.owned_sets().unwrap()[0].receipt.policy.is_none());
    let refused = plain(&d, &bundle("ad hoc", false), "off", None).await.unwrap_err().to_string();
    assert_contains(&refused, &["set-managed-subject", "publish its declaration through that set"]);
    let refused = plain(&d, &bundle("ad hoc", false), "off-force", Some("hotfix")).await.unwrap_err().to_string();
    assert_contains(&refused, &["source-force-refused", "no force path"]);
    hold(&d, "off-hold", false).await.unwrap();
    assert!(hold_active(&d).await);
}

/// Forced writes take effect and record debt; the next publication converges every member,
/// reports each debt as reverted, adopted or closed, writes `source.debt.closed` for it in the
/// same transaction, and releases a forced hold there too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_writes_record_debt_until_the_next_publication_closes_it() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let policy = PolicyRequest {
        enforce: Some(Enforce::Enforce),
        namespaces: Some(vec!["agent/garden/*".into(), "mission/garden/*".into()]),
        unmanaged_allow: Some(vec!["agent/garden/scratch-*".into()]),
        ..Default::default()
    };
    publish_set(&d, 10, enforced_bundle(""), Some(policy)).await;
    let set = d.store.owned_sets().unwrap().remove(0);
    let orchard = "agent/garden/orchard";
    let source_revision = d.store.mission_spec("garden/harvest", None).unwrap().unwrap().revision;

    let forced = plain(&d, &bundle("ad hoc", false), "orchard", Some("hotfix while CI is down")).await.unwrap();
    let recorded = &forced["source_debts_recorded"][0];
    assert_eq!((recorded["subject"].as_str(), recorded["verb"].as_str()), (Some(orchard), Some("apply")));
    assert_eq!(recorded["actor"], "person/operator");
    assert_eq!(recorded["reason"], "hotfix while CI is down");
    assert_eq!(recorded["sha"], format!("{:040x}", 10));
    let forced_token = d.store.selected_desired_token(orchard).unwrap().unwrap();
    assert_ne!(forced_token, set.receipt.members[orchard].claim);
    assert_eq!(recorded["claim"], forced_token);

    let mission = format!("version 2\n{}", HARVEST.replace("goal \"Harvest\"", "goal \"Harvest early\""));
    let forced = plain(&d, &mission, "harvest", Some("frost tonight")).await.unwrap();
    assert_eq!(forced["source_debts_recorded"][0]["verb"], "mission publish");
    let forced_revision = d.store.mission_spec("garden/harvest", None).unwrap().unwrap().revision;
    assert_ne!(forced_revision, source_revision);

    hold(&d, "rose", true).await.unwrap();
    assert!(hold_active(&d).await);

    let refused = plain(&d, &format!("version 2\n{LILY}"), "lily", None).await.unwrap_err().to_string();
    assert_contains(&refused, &["set-managed-subject", "namespace claimed by owned-set/garden"]);
    plain(&d, &format!("version 2\n{LILY}"), "lily-forced", Some("trying a lily")).await.unwrap();
    plain(&d, &format!("version 2\n{}", LILY.replace("lily", "tulip")), "tulip", Some("trying a tulip")).await.unwrap();
    let plant = format!("version 2\n{}", HARVEST.replace("garden/harvest", "garden/plant"));
    let forced = plain(&d, &plant, "plant", Some("trying a planting")).await.unwrap();
    assert_eq!(forced["source_debts_recorded"][0]["verb"], "create");
    plain(&d, &format!("version 2\n{}", LILY.replace("lily", "scratch-1")), "scratch", None).await.unwrap();
    let status = source_status(&d, false).await;
    assert_eq!(status["open"].as_array().unwrap().len(), 6, "{status}");
    assert_eq!(status["sets"][0]["enforce"], "enforce");

    // The source now declares the lily exactly as forced, and still omits the tulip.
    let (preview, published) = publish_set(&d, 11, enforced_bundle(LILY), None).await;
    assert!(preview.effects[orchard].starts_with("revert ad hoc change by person/operator: hotfix"), "{:?}", preview.effects);
    assert_eq!(preview.changes["agent/garden/tulip"], "retiring");
    assert_eq!(preview.changes["mission/garden/plant"], "retiring");
    let outcomes: std::collections::BTreeMap<String, (String, String)> = published["publication"]["source_debts_reverted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["subject"].as_str().unwrap().to_owned(),
            (d["verb"].as_str().unwrap().to_owned(), d["outcome"].as_str().unwrap().to_owned())))
        .collect();
    let expected = [
        (orchard, "apply", "reverted"),
        ("mission/garden/harvest", "mission publish", "reverted"),
        ("agent/garden/rose", "hold", "reverted"),
        ("agent/garden/lily", "create", "adopted"),
        ("agent/garden/tulip", "create", "reverted"),
        ("mission/garden/plant", "create", "reverted"),
    ];
    assert_eq!(outcomes.len(), expected.len(), "{outcomes:?}");
    for (subject, verb, outcome) in expected {
        assert_eq!(outcomes[subject], (verb.to_owned(), outcome.to_owned()), "{subject}");
        let closed = d.store.latest_claim(subject, Some("source.debt.closed")).unwrap().unwrap();
        assert_eq!(closed.body["fields"]["outcome"], outcome);
        assert_eq!(closed.body["fields"]["publisher"], "person/operator");
    }
    // Converged: the set's declarations hold again and the forced hold is released.
    let set = d.store.owned_sets().unwrap().remove(0);
    assert_eq!(d.store.selected_desired_token(orchard).unwrap().unwrap(), set.receipt.members[orchard].claim);
    assert_eq!(d.store.mission_spec("garden/harvest", None).unwrap().unwrap().revision, source_revision);
    assert!(set.receipt.members.contains_key("agent/garden/lily"));
    assert!(set.receipt.retired.contains_key("agent/garden/tulip"));
    assert!(set.receipt.retired.contains_key("mission/garden/plant"));
    assert_eq!(
        d.store.mission_spec("garden/plant", None).unwrap().unwrap().state,
        fractal_daemon::model::MissionState::Retired
    );
    assert!(!hold_active(&d).await);
    let status = source_status(&d, false).await;
    assert!(status["open"].as_array().unwrap().is_empty(), "{status}");
    assert_eq!(status["closed"].as_array().unwrap().len(), 6, "{status}");
    assert_eq!(status["closed"][0]["closed_by"]["set_revision"], format!("owned-set/garden@{}", set.revision));
}

/// Terminal retirement of a live member cannot be forced; through client-v0 it is refused as
/// `set-managed-subject`, not `internal`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_retire_of_a_live_member_is_refused_through_client_v0() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    publish_set(&d, 10, enforced_bundle(""), Some(enforce(Enforce::Enforce))).await;
    let orchard = "agent/garden/orchard";
    let client = fractal_client::Client::unix_as(root.path().join("amber").join("st3.sock"), "person/operator");
    let snapshot = client.capabilities().await.unwrap().snapshot.id;
    let error = client
        .agent_retire(
            &format!("action/retire-{}", "a".repeat(64)),
            "retire-orchard-terminally",
            fractal_client::Fence {
                snapshot_id: snapshot,
                runtime_desired_revision: Some(d.store.selected_desired_token(orchard).unwrap().unwrap()),
                ..Default::default()
            },
            fractal_client::AgentRetireParameters {
                agent: orchard.into(),
                reason: "done with it".into(),
                successor: None,
                expect_revision: None,
            },
        )
        .await
        .unwrap_err();
    let text = format!("{error:?} {error}");
    assert_contains(&text, &["SetManagedSubject", "retirement is terminal and cannot be forced",
        "omit agent/garden/orchard from the set publication"]);
    assert!(!text.contains("Internal"), "{text}");
    assert!(d.store.agent_retirement(orchard).unwrap().is_none());
}

/// An `audit` set refuses nothing beyond `off`: each write `enforce` would refuse proceeds and
/// records `source.audited` with that refusal. Only the terminal retirement also opens debt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_audit_set_records_what_enforce_would_refuse() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let policy = PolicyRequest {
        enforce: Some(Enforce::Audit),
        namespaces: Some(vec!["agent/garden/*".into()]),
        ..Default::default()
    };
    publish_set(&d, 10, enforced_bundle(""), Some(policy)).await;
    hold(&d, "audited-hold", false).await.unwrap();
    assert!(hold_active(&d).await);
    plain(&d, &format!("version 2\n{LILY}"), "audited-lily", None).await.unwrap();
    let orchard = "agent/garden/orchard";
    // A declaration cannot take effect over the set's member reference, so it is still refused.
    let refused = plain(&d, &bundle("ad hoc", false), "audited-apply", None).await.unwrap_err().to_string();
    assert_contains(&refused, &["set-managed-subject", "ad hoc apply is refused"]);
    let token = d.store.selected_desired_token(orchard).unwrap().unwrap();
    d.store.retire_agent(orchard, "person/operator", "done", &token, None, "audited-retire", &[]).unwrap();
    let status = source_status(&d, true).await;
    let open: Vec<_> = status["open"].as_array().unwrap().iter().map(|debt| (&debt["subject"], &debt["verb"])).collect();
    assert_eq!(open, [(&json!(orchard), &json!("retire"))], "{status}");
    let audited: Vec<(String, String)> = status["audited"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["subject"].as_str().unwrap().to_owned(), a["verb"].as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(audited, [
        (orchard.to_owned(), "retire".to_owned()),
        ("agent/garden/lily".to_owned(), "create".to_owned()),
        ("agent/garden/rose".to_owned(), "hold".to_owned()),
    ], "{status}");
    assert_eq!(status["audited"][0]["actor"], "person/operator");
    assert_contains(status["audited"][0]["refusal"].as_str().unwrap(), &["retirement is terminal"]);
    assert!(source_status(&d, false).await.get("audited").is_none());
}

/// An ad hoc terminal retirement cannot be reverted, so it never wedges the set: a publication
/// still declaring the member keeps it retired and says so. An `audit` set also records the
/// retirement as source debt, open until the source drops the member and then adopted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ad_hoc_retirement_keeps_the_member_retired_until_the_source_drops_it() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let orchard = "agent/garden/orchard";
    publish_set(&d, 10, enforced_bundle(""), Some(enforce(Enforce::Audit))).await;
    let token = d.store.selected_desired_token(orchard).unwrap().unwrap();
    d.store.retire_agent(orchard, "person/operator", "done", &token, None, "audited-retire", &[]).unwrap();
    let status = source_status(&d, false).await;
    assert_eq!(status["open"].as_array().unwrap().len(), 1, "{status}");
    assert_eq!(status["open"][0]["subject"], orchard);
    assert_eq!(status["open"][0]["verb"], "retire");
    assert_eq!(status["open"][0]["reason"], "done");

    // Every publication still declaring the member succeeds, keeps it retired and reports it.
    for sequence in [11, 12] {
        let (planned, published) = publish_set(&d, sequence, enforced_bundle(LILY), None).await;
        assert!(planned.blockers.is_empty(), "{:?}", planned.blockers);
        assert!(!planned.mass_retirement);
        assert_eq!(planned.changes[orchard], "retired");
        assert_eq!(planned.effects[orchard], "keep terminally retired by person/operator: done; drop it from the source");
        assert!(!planned.declaration_diffs.contains_key(orchard));
        assert!(planned.reverted_debts.is_empty(), "{:?}", planned.reverted_debts);
        assert!(published["publication"]["source_debts_reverted"].as_array().is_none_or(Vec::is_empty), "{published}");
        let set = d.store.owned_sets().unwrap().remove(0);
        assert!(set.blockers.is_empty(), "{:?}", set.blockers);
        assert!(set.receipt.retired.contains_key(orchard) && !set.receipt.members.contains_key(orchard));
        assert!(set.receipt.members.contains_key("agent/garden/lily"));
        assert!(d.store.agent_retirement(orchard).unwrap().is_some());
        assert_eq!(source_status(&d, false).await["open"][0]["verb"], "retire");
    }

    // Dropping the member from the source adopts the retirement and closes its debt.
    let (planned, _) = publish_set(&d, 13, format!("version 2\n{ROSE}{LILY}{HARVEST}"), None).await;
    assert!(!planned.changes.contains_key(orchard), "{:?}", planned.changes);
    assert_eq!(planned.reverted_debts.len(), 1, "{:?}", planned.reverted_debts);
    assert_eq!(planned.reverted_debts[0].outcome.as_deref(), Some("adopted"));
    let status = source_status(&d, false).await;
    assert!(status["open"].as_array().unwrap().is_empty(), "{status}");
    assert_eq!(status["closed"][0]["verb"], "retire", "{status}");
    assert_eq!(status["closed"][0]["outcome"], "adopted", "{status}");
}

/// A set without a policy records no debt, but its publication still keeps an ad hoc retired
/// member retired instead of failing with `recipient-retired`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_off_set_publication_keeps_an_ad_hoc_retired_member_retired() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let orchard = "agent/garden/orchard";
    publish_set(&d, 10, enforced_bundle(""), None).await;
    let token = d.store.selected_desired_token(orchard).unwrap().unwrap();
    d.store.retire_agent(orchard, "person/operator", "done", &token, None, "off-retire", &[]).unwrap();
    let (planned, _) = publish_set(&d, 11, enforced_bundle(LILY), None).await;
    assert!(planned.blockers.is_empty(), "{:?}", planned.blockers);
    assert_eq!(planned.changes[orchard], "retired");
    assert!(d.store.owned_sets().unwrap().remove(0).receipt.retired.contains_key(orchard));
    assert!(source_status(&d, false).await["open"].as_array().unwrap().is_empty());
}

/// Dropping an agent retired ad hoc from the source is not a retirement by that publication, so
/// it needs no mass-retirement confirmation, and it adopts the retire debt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_an_ad_hoc_retired_member_needs_no_retirement_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let orchard = "agent/garden/orchard";
    publish_set(&d, 10, format!("{}{ROSE}", bundle("source", false)), Some(enforce(Enforce::Audit))).await;
    let token = d.store.selected_desired_token(orchard).unwrap().unwrap();
    d.store.retire_agent(orchard, "person/operator", "done", &token, None, "dropped-retire", &[]).unwrap();
    // Half of the previous membership is omitted, which would otherwise need confirmation.
    let (planned, _) = publish_set(&d, 11, format!("version 2\n{ROSE}"), None).await;
    assert!(!planned.mass_retirement);
    assert_eq!(planned.changes[orchard], "retired");
    assert_eq!(planned.effects[orchard], "keep terminally retired by person/operator: done");
    assert_eq!(planned.reverted_debts.len(), 1, "{:?}", planned.reverted_debts);
    assert_eq!(planned.reverted_debts[0].outcome.as_deref(), Some("adopted"));
    let set = d.store.owned_sets().unwrap().remove(0);
    assert!(set.blockers.is_empty(), "{:?}", set.blockers);
    assert!(set.receipt.retired.contains_key(orchard) && !set.receipt.members.contains_key(orchard));
    assert!(source_status(&d, false).await["open"].as_array().unwrap().is_empty());
}

/// A policy needs every active member to advertise `source_enforcement`; a set naming its
/// publishers refuses anyone else; debt and its closure replicate and both replicas converge.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_policy_is_gated_fenced_to_its_publisher_and_debt_replicates() {
    let root = tempfile::tempdir().unwrap();
    let keys = (0..2).map(|_| Arc::new(MemberKey::generate().unwrap().0)).collect::<Vec<_>>();
    let amber = daemon(root.path(), "amber", keys[0].clone(), &keys[0]).await;
    let ivory = daemon(root.path(), "ivory", keys[1].clone(), &keys[0]).await;
    for (i, name) in ["amber", "ivory"].into_iter().enumerate() {
        let mut fields = json!({"fleet_id":FLEET,"member_key":keys[i].public(),
            "via":if i==0{"anchor"}else{"invite"},"mode":"listening"});
        if i != 0 {
            fields["sponsor"] = json!("host/amber");
        }
        append(&amber.store, &format!("host/{name}"), "fleet.member-admitted", fields);
    }
    let exchange = |amber: &Daemon, ivory: &Daemon| {
        share(&amber.store, &ivory.store, "amber");
        share(&ivory.store, &amber.store, "ivory");
    };
    exchange(&amber, &ivory);
    let policy = PolicyRequest {
        enforce: Some(Enforce::Enforce),
        publisher: Some(vec!["person/operator".into()]),
        ..Default::default()
    };
    let mut publication = request(&amber, 10, enforced_bundle("")).await;
    publication.options.policy = Some(policy);
    let blocked: Preview = amber.client.post("/v1/sets/preview", &publication).await.unwrap();
    assert!(
        blocked.blockers.iter().any(|b| b == "host/ivory has not advertised source enforcement support; upgrade before activation"),
        "{:?}", blocked.blockers
    );
    for (daemon, name) in [(&amber, "amber"), (&ivory, "ivory")] {
        append(&daemon.store, &format!("daemon/{name}"), "daemon.started",
            json!({"status":"running","features":{"owned_sets":1,"source_enforcement":1}}));
    }
    exchange(&amber, &ivory);
    preview(&amber, &mut publication).await;
    let _: Value = amber.client.post("/v1/sets/apply", &publication).await.unwrap();
    exchange(&amber, &ivory);

    // Only the named publisher may publish the set.
    let mut stranger = request(&amber, 11, enforced_bundle(LILY)).await;
    stranger.actor = "person/intruder".into();
    preview(&amber, &mut stranger).await;
    let refused = amber.client.post::<_, Value>("/v1/sets/apply", &stranger).await.unwrap_err().to_string();
    assert_contains(&refused, &["source-publisher-refused", "person/operator"]);

    let orchard = "agent/garden/orchard";
    let forced = plain(&amber, &bundle("ad hoc", false), "orchard", Some("hotfix")).await.unwrap();
    exchange(&amber, &ivory);
    let forced_token = forced["source_debts_recorded"][0]["claim"].as_str().unwrap().to_owned();
    for daemon in [&amber, &ivory] {
        assert_eq!(daemon.store.selected_desired_token(orchard).unwrap().unwrap(), forced_token);
        assert_eq!(source_status(daemon, false).await["open"].as_array().unwrap().len(), 1);
    }
    publish_set(&amber, 12, enforced_bundle(""), None).await;
    exchange(&amber, &ivory);
    for daemon in [&amber, &ivory] {
        let set = daemon.store.owned_sets().unwrap().remove(0);
        assert!(set.blockers.is_empty(), "{:?}", set.blockers);
        assert_eq!(set.receipt.policy.as_ref().unwrap().publisher, ["person/operator"]);
        assert_eq!(daemon.store.selected_desired_token(orchard).unwrap().unwrap(), set.receipt.members[orchard].claim);
        let status = source_status(daemon, false).await;
        assert!(status["open"].as_array().unwrap().is_empty(), "{status}");
        assert_eq!(status["closed"][0]["outcome"], "reverted", "{status}");
    }
}

/// A top-level subscription is a set member: omitting it retires it through its stopped form.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_omitted_subscription_member_is_retired_by_its_stopped_form() {
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    // A repository's standing observer needs no declaration of its own.
    let subscription = "subscription \"garden/alerts\" { observer \"observer/github/acme/garden\"; to \"agent/garden/orchard\"; on \"issues\"; delivery \"message\" { every \"30m\" } }\n";
    let (planned, _) = publish_set(&d, 10, format!("{}{subscription}", bundle("source", false)), None).await;
    assert_eq!(planned.changes["subscription/garden/alerts"], "added");
    let set = d.store.owned_sets().unwrap().remove(0);
    assert_eq!(set.receipt.members["subscription/garden/alerts"].kind, "subscription");
    // Outside a set, plain apply has no top-level subscription at all: the ad hoc path is refused
    // before ownership is consulted, so the set stays the subscription's only writer.
    let refused = plain(&d, &format!("version 2\n{}", subscription.replace("30m", "1h")), "sub", None)
        .await.unwrap_err().to_string();
    assert_contains(&refused, &["runtime-outside-mission"]);
    // Omitting a member retires it, which needs confirmation of this exact preview.
    let mut omit = request(&d, 11, bundle("source", false)).await;
    let planned = preview(&d, &mut omit).await;
    omit.options.confirm_retire = Some(planned.digest.clone());
    d.client.post::<_, Value>("/v1/sets/apply", &omit).await.unwrap();
    assert_eq!(planned.changes["subscription/garden/alerts"], "retiring");
    assert_eq!(planned.effects["subscription/garden/alerts"], "stop deliveries; retain delivered messages");
    let set = d.store.owned_sets().unwrap().remove(0);
    assert!(set.blockers.is_empty(), "{:?}", set.blockers);
    assert!(set.receipt.retired.contains_key("subscription/garden/alerts"));
    let stopped = d.store.desired_subject_with_writer("subscription/garden/alerts").unwrap().unwrap().0;
    assert_eq!(stopped.kind, "subscription");
    assert!(fractal_daemon::graph::subscription_spec(&stopped.desired).unwrap().stopped);
}

fn map_file(path: &str, text: Option<&str>, sha256: Option<String>) -> Value {
    json!({"repository": "acme/garden", "path": path, "blob": "b".repeat(40), "text": text, "sha256": sha256})
}

/// One node of `kdl` at `line` of `file`, declared there.
fn map_node(kdl: &str, file: usize, line: u32) -> Value {
    json!({"kdl": kdl, "kdlLines": [2, 2],
        "span": {"file": file, "start": {"line": line, "column": 1}, "end": {"line": line, "column": 9}},
        "frames": [{"file": file, "start": {"line": line, "column": 3}, "end": {"line": line, "column": 3},
            "role": "declaration", "precision": "exact"}]})
}

/// A map of both garden agents: orchard reads `agent.ts` and `shared.ts`, meadow `meadow.ts`
/// and `shared.ts`; `unused.ts` is named by no subject. `line` places every node.
fn garden_map(files: [Value; 4], line: u32, extra: Option<&str>) -> fractal_schema::source_map::SourceMap {
    let mut subjects = json!({
        "agent/garden/orchard": {"declaration": 0, "inputs": [0, 1],
            "nodes": [map_node("agent[garden/orchard]/description[0]", 0, line)]},
        "agent/garden/meadow": {"declaration": 2, "inputs": [2, 1],
            "nodes": [map_node("agent[garden/meadow]", 2, line)]},
    });
    if let Some(subject) = extra {
        subjects[subject] = json!({"declaration": 0});
    }
    serde_json::from_value(json!({"schema": "fractal.source-map/v1", "files": files, "subjects": subjects})).unwrap()
}

async fn apply_mapped(
    d: &Daemon,
    sequence: u64,
    note: &str,
    map: fractal_schema::source_map::SourceMap,
) -> anyhow::Result<Value> {
    let mut publication = request(d, sequence, bundle(note, true)).await;
    preview(d, &mut publication).await;
    publication.source_map = Some(map);
    d.client.post("/v1/sets/apply", &publication).await
}

fn member_revision(d: &Daemon, subject: &str) -> String {
    d.store.owned_sets().unwrap().remove(0).receipt.members[subject].revision.clone()
}

/// A set publication's map records one `source.mapped` claim per new member revision, keeps the
/// first map of a revision, stores each named text once, refuses a map naming a non-member or an
/// unknown text without applying anything, and replicates claims together with their texts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_maps_record_once_per_revision_with_deduplicated_texts() {
    use fractal_schema::source_map::sha256_hex;
    let root = tempfile::tempdir().unwrap();
    let keys = (0..2).map(|_| Arc::new(MemberKey::generate().unwrap().0)).collect::<Vec<_>>();
    let amber = daemon(root.path(), "amber", keys[0].clone(), &keys[0]).await;
    let cobalt = daemon(root.path(), "cobalt", keys[1].clone(), &keys[0]).await;
    for (i, name) in ["amber", "cobalt"].into_iter().enumerate() {
        let mut fields = json!({"fleet_id": FLEET, "member_key": keys[i].public(),
            "via": if i == 0 { "anchor" } else { "invite" }, "mode": "listening"});
        if i != 0 {
            fields["sponsor"] = json!("host/amber");
        }
        append(&amber.store, &format!("host/{name}"), "fleet.member-admitted", fields);
    }
    share(&amber.store, &cobalt.store, "amber");
    share(&cobalt.store, &amber.store, "cobalt");

    let (agent, shared, meadow, unused) = ("agent v1\n", "shared\n", "meadow\n", "unused\n");
    let files = |agent: &str| {
        [
            map_file("graph/agent.ts", Some(agent), None),
            map_file("lib/shared.ts", Some(shared), None),
            map_file("graph/meadow.ts", Some(meadow), None),
            map_file("graph/unused.ts", Some(unused), None),
        ]
    };
    let first = apply_mapped(&amber, 10, "initial", garden_map(files(agent), 4, None)).await.unwrap();
    assert_eq!(
        first["publication"]["source_maps"],
        json!({"recorded": 2, "kept": 0, "blobs_new": 3, "blobs_deduped": 0}),
        "{first}"
    );
    let orchard = member_revision(&amber, "agent/garden/orchard");
    let read: Value = amber
        .client
        .get("/v1/source-maps?subject=agent%2Fgarden%2Forchard&texts=true")
        .await
        .unwrap();
    assert_eq!(read["revision"], orchard);
    let map = &read["map"];
    assert_eq!(map["subject"], "agent/garden/orchard");
    assert_eq!(map["revision"], orchard);
    assert_eq!(map["sha"], format!("{:040x}", 10));
    assert_eq!(map["sequence"], 10);
    assert_eq!(map["set_revision"], format!("owned-set/garden@{}", amber.store.owned_sets().unwrap()[0].revision));
    // Only the files this subject names, re-indexed; texts by their sha256.
    assert_eq!(map["files"].as_array().unwrap().iter().map(|f| f["path"].clone()).collect::<Vec<_>>(),
        [json!("graph/agent.ts"), json!("lib/shared.ts")]);
    assert_eq!(map["files"][1]["blob_hash"], sha256_hex(shared));
    assert_eq!(read["texts"][sha256_hex(agent)], agent);
    assert_eq!(read["texts"][sha256_hex(shared)], shared);
    assert_eq!(amber.store.source_blob(&sha256_hex(meadow)).unwrap().as_deref(), Some(meadow.as_bytes()));
    // A file no recorded map names is not stored.
    assert_eq!(amber.store.source_blob(&sha256_hex(unused)).unwrap(), None);
    let missing = amber.client.get::<Value>(&format!("/v1/source-blobs/{}", sha256_hex(unused))).await.unwrap_err();
    assert!(format!("{missing:#}").contains("not-found"), "{missing:#}");

    // The next commit republishes both revisions with drifted spans: the first maps are kept.
    let kept = apply_mapped(&amber, 11, "initial", garden_map(files(agent), 9, None)).await.unwrap();
    assert_eq!(kept["publication"]["source_maps"], json!({"recorded": 0, "kept": 2, "blobs_new": 0, "blobs_deduped": 0}));
    assert_eq!(member_revision(&amber, "agent/garden/orchard"), orchard);
    let read: Value = amber.client.get("/v1/source-maps?subject=agent%2Fgarden%2Forchard").await.unwrap();
    assert_eq!(read["map"]["sha"], format!("{:040x}", 10));
    assert_eq!(read["map"]["nodes"][0]["span"]["start"]["line"], 4);
    assert!(read.get("texts").is_none());

    // A changed orchard gets a new map; meadow sends only the sha256 of its stored text.
    let mut changed = files("agent v2\n");
    changed[2] = map_file("graph/meadow.ts", None, Some(sha256_hex(meadow)));
    let second = apply_mapped(&amber, 12, "changed", garden_map(changed, 5, None)).await.unwrap();
    assert_eq!(second["publication"]["source_maps"], json!({"recorded": 1, "kept": 1, "blobs_new": 1, "blobs_deduped": 1}));
    let orchard_v2 = member_revision(&amber, "agent/garden/orchard");
    assert_ne!(orchard_v2, orchard);
    let read: Value = amber.client.get("/v1/source-maps?subject=agent%2Fgarden%2Forchard&texts=true").await.unwrap();
    assert_eq!((read["map"]["sha"].clone(), read["texts"][sha256_hex("agent v2\n")].clone()),
        (json!(format!("{:040x}", 12)), json!("agent v2\n")));
    let old: Value = amber
        .client
        .get(&format!("/v1/source-maps?subject=agent%2Fgarden%2Forchard&revision={orchard}"))
        .await
        .unwrap();
    assert_eq!(old["map"]["sha"], format!("{:040x}", 10));
    let unmapped = amber.client.get::<Value>("/v1/source-maps?subject=agent%2Fgarden%2Forchard&revision=0000").await.unwrap_err();
    assert!(format!("{unmapped:#}").contains("source-map-missing"), "{unmapped:#}");

    // A map naming a non-member, or a text the daemon does not hold, refuses the whole publication.
    let selected = amber.store.owned_sets().unwrap()[0].revision.clone();
    let refused = apply_mapped(&amber, 13, "refused", garden_map(files("agent v3\n"), 5, Some("agent/garden/elsewhere")))
        .await
        .unwrap_err();
    assert_contains(&format!("{refused:#}"), &["invalid-source-map", "agent/garden/elsewhere", "not a member"]);
    let mut unknown = files("agent v3\n");
    unknown[1] = map_file("lib/shared.ts", None, Some(sha256_hex("never stored\n")));
    let refused = apply_mapped(&amber, 13, "refused", garden_map(unknown, 5, None)).await.unwrap_err();
    assert_contains(&format!("{refused:#}"), &["source-blob-missing"]);
    assert_eq!(amber.store.owned_sets().unwrap()[0].revision, selected);
    assert_eq!(member_revision(&amber, "agent/garden/orchard"), orchard_v2);

    // Claims replicate with the texts they name.
    share(&amber.store, &cobalt.store, "amber");
    for revision in [&orchard, &orchard_v2] {
        let (_, map) = cobalt.store.source_map("agent/garden/orchard", Some(revision)).unwrap();
        let map = map.unwrap();
        for file in map["files"].as_array().unwrap() {
            let hash = file["blob_hash"].as_str().unwrap();
            assert!(cobalt.store.source_blob(hash).unwrap().is_some(), "{hash} replicated");
        }
    }
}

const HARVEST_TS: &str = "import { policy } from '../../../lib/policy'\nexport default mission({\n  id: 'garden/harvest',\n  goal: policy.goal,\n  steps: [step({\n    id: 'work',\n    gate: exists('resource/garden/report'),\n  })],\n})\n";

/// The garden set with the harvest mission, whose goal comes from `lib/policy.ts`.
fn harvest_bundle(goal: &str) -> String {
    format!(
        "{}mission \"garden/harvest\" state=\"ready\" {{\n goal \"{goal}\"\n step \"work\" {{\n  assigned-to \"agent/garden/orchard\"\n  gate \"the report exists\" {{ exists \"resource/garden/report\" }}\n }}\n}}\n",
        bundle("orchard", false)
    )
}

/// The map of that bundle: `policy` is `lib/policy.ts`, whose goal is on `goal_line`.
fn harvest_map(policy: &str, goal_line: u32) -> fractal_schema::source_map::SourceMap {
    let span = |file: usize, start: u32, end: u32| json!({"file": file, "start": {"line": start, "column": 1}, "end": {"line": end, "column": 3}});
    let frame = |file: usize, line: u32, role: &str| json!({"file": file, "start": {"line": line, "column": 3}, "end": {"line": line, "column": 9}, "role": role, "precision": "exact"});
    serde_json::from_value(json!({
        "schema": "fractal.source-map/v1",
        "files": [
            map_file("graph/garden/harvest/mission.ts", Some(HARVEST_TS), None),
            map_file("lib/policy.ts", Some(policy), None),
            map_file("graph/garden/orchard/agent.ts", Some("export default agent({\n  id: 'garden/orchard',\n})\n"), None),
        ],
        "subjects": {
            "mission/garden/harvest": {"declaration": 0, "inputs": [0, 1], "nodes": [
                {"kdl": "mission[garden/harvest]", "kdlLines": [1, 7], "span": span(0, 2, 9), "frames": []},
                {"kdl": "mission[garden/harvest]/goal[0]", "kdlLines": [2, 2], "span": span(1, goal_line, goal_line),
                 "frames": [frame(0, 4, "declaration"), frame(1, goal_line, "value")]},
                {"kdl": "mission[garden/harvest]/step[work]", "kdlLines": [3, 6], "span": span(0, 5, 8), "frames": [frame(0, 5, "declaration")]},
                {"kdl": "mission[garden/harvest]/step[work]/gate[the report exists]", "kdlLines": [5, 5], "span": span(0, 7, 7), "frames": []},
            ]},
            "agent/garden/orchard": {"declaration": 2, "inputs": [2], "nodes": [
                {"kdl": "agent[garden/orchard]", "kdlLines": [2, 6], "span": span(2, 1, 3), "frames": []},
            ]},
        },
    }))
    .unwrap()
}

async fn publish_mapped(d: &Daemon, sequence: u64, kdl: String, map: fractal_schema::source_map::SourceMap) {
    let mut publication = request(d, sequence, kdl).await;
    preview(d, &mut publication).await;
    publication.source_map = Some(map);
    d.client.post::<_, Value>("/v1/sets/apply", &publication).await.unwrap();
}

async fn source_cli(d: &Daemon, args: &[&str]) -> (bool, String) {
    let socket = d.client.socket_path().unwrap().to_path_buf();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let output = tokio::task::spawn_blocking(move || {
        fractal_daemon::test_support::command(test_bin!("fractal-fixture"))
            .env_remove("ST_AGENT").env_remove("FRACTAL_AGENT").env_remove("ST_MISSION_RUN")
            .arg("--endpoint").arg(socket)
            .args(args)
            .output().unwrap()
    }).await.unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// The CLI reads recorded maps: a subject's declared nodes, one node's frames, a step run through
/// the revision its run pinned, the live subjects a file line feeds, and why two revisions differ.
/// Failure text names the declaring line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_cli_shows_finds_and_diffs_mapped_revisions() {
    use fractal_daemon::MissionsStore;
    let root = tempfile::tempdir().unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    let d = daemon(root.path(), "amber", key.clone(), &key).await;
    let policy_v1 = "export const policy = {\n  goal: 'Harvest',\n}\n";
    publish_mapped(&d, 10, harvest_bundle("Harvest"), harvest_map(policy_v1, 2)).await;
    let first = d.store.mission_spec("garden/harvest", None).unwrap().unwrap().revision;
    let run = d
        .store
        .create_mission_run(&fractal_daemon::model::MissionRunRequest {
            mission: "garden/harvest".into(),
            revision: None,
            workspace: root.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: Some("run".into()),
            inputs: Default::default(),
            idempotency_key: "harvest-run".into(),
        })
        .unwrap();
    let step = run.steps.iter().find(|step| step.step == "work").unwrap().subject.clone();
    // The next commit moves the goal into a commented policy file: a new mission revision.
    let policy_v2 = "// the garden's policy\nexport const policy = {\n  goal: 'Harvest early',\n}\n";
    publish_mapped(&d, 11, harvest_bundle("Harvest early"), harvest_map(policy_v2, 3)).await;
    let second = d.store.mission_spec("garden/harvest", None).unwrap().unwrap().revision;
    assert_ne!(first, second);

    let (ok, shown) = source_cli(&d, &["subject", "show", "mission/garden/harvest", "--source"]).await;
    assert!(ok, "{shown}");
    assert_contains(&shown, &[
        "MISSION   garden/harvest",
        &format!("SOURCE    acme/garden@{}  graph/garden/harvest/mission.ts:2-9", &format!("{:040x}", 11)[..7]),
        &format!("https://github.com/acme/garden/blob/{:040x}/graph/garden/harvest/mission.ts#L2-L9", 11),
        "DECLARES",
        "step work",
        "gate the report exists",
        "USES  lib/policy.ts",
    ]);
    let (ok, node) = source_cli(&d, &["subject", "show", "mission/garden/harvest", "--source", "--node", "goal"]).await;
    assert!(ok, "{node}");
    assert_contains(&node, &["NODE      mission[garden/harvest]/goal[0]", "3 |   goal: 'Harvest early',", "declared", "mission.ts:4", "value", "lib/policy.ts:3"]);
    let (ok, json) = source_cli(&d, &["--json", "subject", "show", "mission/garden/harvest", "--source", "--revision", &first]).await;
    assert!(ok, "{json}");
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!((json["revision"].as_str(), json["source"]["sequence"].as_u64()), (Some(first.as_str()), Some(10)));
    assert_eq!(json["uses"], json!([{"repository": "acme/garden", "path": "lib/policy.ts"}]));

    // A step run resolves through the revision its run pinned, not the newest one.
    let (ok, pinned) = source_cli(&d, &["subject", "show", &step, "--source"]).await;
    assert!(ok, "{pinned}");
    assert_contains(&pinned, &[&format!("REVISION  {} · seq 10", &first[..7]), "NODE      mission[garden/harvest]/step[work]"]);
    assert_eq!(
        d.store.source_declared_at(&step, Some("the report exists")).unwrap().as_deref(),
        Some(format!("graph/garden/harvest/mission.ts:7 (acme/garden@{})", &format!("{:040x}", 10)[..7]).as_str())
    );
    assert_eq!(d.store.source_declared_at(&step, None).unwrap().unwrap(), format!("graph/garden/harvest/mission.ts:5-8 (acme/garden@{})", &format!("{:040x}", 10)[..7]));

    let (ok, line) = source_cli(&d, &["source", "where", "lib/policy.ts:3", "--repository", "acme/garden"]).await;
    assert!(ok, "{line}");
    assert_contains(&line, &["lib/policy.ts:3    goal: 'Harvest early',", "FEEDS(1 subjects, 2 nodes)", "mission/garden/harvest", "Editing this line can change 1 subject on the next publish."]);
    let (ok, none) = source_cli(&d, &["source", "where", "lib/policy.ts:1"]).await;
    assert!(ok, "{none}");
    assert_contains(&none, &["No live subject reads lib/policy.ts:1"]);
    let (ok, declared) = source_cli(&d, &["--json", "source", "where", "https://github.com/acme/garden/blob/main/graph/garden/orchard/agent.ts"]).await;
    assert!(ok, "{declared}");
    let declared: Value = serde_json::from_str(&declared).unwrap();
    assert_eq!(declared["files"][0]["subjects"][0]["subject"], "agent/garden/orchard");
    assert_eq!(declared["files"][0]["subjects"][0]["declares"], true);

    let revisions: Value = d.client.get("/v1/source-maps/revisions?subject=mission%2Fgarden%2Fharvest").await.unwrap();
    assert_eq!(revisions["current"], second);
    assert_eq!(revisions["revisions"].as_array().unwrap().iter().map(|entry| entry["revision"].clone()).collect::<Vec<_>>(), [json!(second), json!(first)]);
    let orchard = member_revision(&d, "agent/garden/orchard");
    let kdl: Value = d.client.get(&format!("/v1/source-maps/declaration?subject=agent%2Fgarden%2Forchard&revision={orchard}")).await.unwrap();
    assert!(kdl["kdl"].as_str().unwrap().contains("garden/orchard"), "{kdl}");

    let (ok, diff) = source_cli(&d, &["source", "diff", "mission/garden/harvest"]).await;
    assert!(ok, "{diff}");
    assert_contains(&diff, &[
        &format!("mission/garden/harvest  {} → {}", &first[..7], &second[..7]),
        "WHY       dependency: The mission file did not change. The change comes from policy.ts, which this mission uses.",
        "M lib/policy.ts",
        "+// the garden's policy",
        "-  goal: 'Harvest',",
        "KDL  +1 −1",
        "+    goal \"Harvest early\"",
    ]);
    let (ok, json) = source_cli(&d, &["--json", "source", "diff", "mission/garden/harvest", &format!("{}..{}", &first[..8], &second[..8])]).await;
    assert!(ok, "{json}");
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!((json["why"]["kind"].as_str(), json["kdl"]["added"].as_u64()), (Some("dependency"), Some(1)));
}
