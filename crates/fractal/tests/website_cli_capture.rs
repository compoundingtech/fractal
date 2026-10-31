#![cfg(unix)]
//! Seeds a fixture daemon for the fractal.how CLI and TUI captures and serves it until stopped.
//!
//! Run (ignored by default):
//!   CAPTURE_DIR=/tmp/cli-capture cargo test -p fractal --features test-support \
//!     --test website_cli_capture -- --ignored --nocapture
//! The daemon listens on $CAPTURE_DIR/st3.sock until $CAPTURE_DIR/stop exists. Every name is invented.
//! scripts/website-cli-capture/capture.sh then prints the CLI transcript, tui.sh a TUI screen.
//!
//! CAPTURE_SCENARIO picks the seed. `marrowpine` (the default) is Robin's: the CLI transcript and
//! the TUI's Now and palette screens. `app` is the website's own example, Ada's: the maintainer
//! updates acme/app's dependencies while the reviewer reads the changelogs. Ada chose parser 3.0
//! in her own step, and the maintainer, a working Codex agent with its native session, migrates the
//! CLI. It feeds the TUI's agent and mission screens.

#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;

use fractal_daemon::MissionsStore;
use fractal_daemon::api::AppState;
use fractal_daemon::model::{ClaimInput, IntentInput, MissionRunRequest, WorkRequest};
use fractal_daemon::store::Store;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, watch};

const MARROWPINE: &str = r#"version 2
agent "marrowpine/migrator" { name "Migrator"; workspace "/srv/marrowpine"; command "true" }
agent "marrowpine/oncall" { name "On-call"; workspace "/srv/marrowpine"; command "true" }
agent "marrowpine/reviewer" { name "Reviewer"; workspace "/srv/marrowpine"; command "true" }
mission "marrowpine/invoice-index" state="ready" {
  goal "Add the due-date index to the invoices table without disturbing customers."
  step "migrate" timeout="2h" {
    assigned-to "agent/marrowpine/migrator"
    goal "Write, rehearse and run the due-date index migration."
  }
  step "review" {
    assigned-to "agent/marrowpine/reviewer"
    goal "Review the migration and its rehearsal."
    depends-on { step "migrate" completed }
  }
}
mission "marrowpine/release" state="ready" {
  goal "Release Marrowpine 2.3."
  step "build" { assigned-to "agent/marrowpine/oncall"; goal "Build and test the release."; }
}
"#;

const APP: &str = r#"version 2
agent "app/maintainer" { name "Maintainer"; workspace "/work/app"; command "true" }
agent "app/reviewer" { name "Reviewer"; workspace "/work/app"; command "true" }
mission "app/update-deps" state="ready" {
  goal "Dependencies are up to date."
  step "audit" {
    assigned-to "agent/app/maintainer"
    goal "List every outdated dependency."
  }
  step "choose-parser" {
    assigned-to "person/ada"
    goal "Ship parser 3.0 and migrate the CLI, or hold at 2.x?"
    depends-on { step "audit" completed }
  }
  step "changelogs" {
    assigned-to "agent/app/reviewer"
    goal "Read the changelogs of every update."
    depends-on { step "audit" completed }
  }
  step "update" timeout="1h" {
    assigned-to "agent/app/maintainer"
    goal "A deps PR with CI green."
    depends-on { step "audit" completed }
  }
  step "review" {
    assigned-to "agent/app/reviewer"
    goal "Review the deps PR."
    depends-on { step "update" completed }
    gate "Ada merges it" type="human" { reviewer "person/ada"; question "Merge the deps PR?"; }
  }
}
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "serves a seeded fixture daemon for the website CLI capture until $CAPTURE_DIR/stop exists"]
async fn serve_seeded_fixture_for_the_website_cli_capture() {
    let dir = PathBuf::from(std::env::var("CAPTURE_DIR").expect("CAPTURE_DIR"));
    let app = match std::env::var("CAPTURE_SCENARIO").as_deref() {
        Err(_) | Ok("marrowpine") => false,
        Ok("app") => true,
        Ok(other) => panic!("CAPTURE_SCENARIO `{other}`: use marrowpine or app"),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("st3.sock");
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(dir.join("stop"));
    // A fresh store file each run. Against an in-memory store the TUI could not save its space
    // ("database table is locked: batches") and said so on screen.
    let database = dir.join("store.sqlite");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
    }
    let state = AppState {
        owning_person: None,
        store: Arc::new(Store::open(&database, "studio").unwrap()),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0_u64).0,
        node: "studio".into(),
        state_dir: dir.clone(),
        pty_root: dir.join("pty"),
        pty_binary: PathBuf::from("pty"),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        harness_question_sink: None,
        // The app seed's maintainer runs Codex. The daemon reads its native session from here.
        native_session_home: app.then(|| dir.join("home")),
        planner_default: fractal_daemon::model::PlannerSpec::default(),
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    };
    let store = state.store.clone();
    let (source, workspace, requester) = if app {
        (APP, "/work/app", "person/ada")
    } else {
        (MARROWPINE, "/srv/marrowpine", "person/robin")
    };
    let intent = fractal_daemon::graph::parse_intent(source, "studio").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "website-capture")
        .unwrap();
    let run = |mission: &str, key: &str| {
        store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: workspace.into(),
                requester: Some(requester.into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: key.into(),
            })
            .unwrap()
    };
    let work = |subject: &str, seat: &str, action: &str, key: &str, summary: &str| {
        if action == "claim" {
            store.set_step_state(subject, "ready", None).unwrap();
        }
        store
            .work_action(
                subject,
                action,
                &WorkRequest {
                    wait_ms: None,
                    actor: Some(seat.into()),
                    incarnation: Some(format!("{seat}:1")),
                    summary: Some(summary.into()),
                    reason: None,
                    evidence: Vec::new(),
                    idempotency_key: key.into(),
                },
            )
            .unwrap();
    };

    // Ada's scenario: the maintainer's step, Ada's choice and the conversation that runs it.
    let mut update = None;
    let mut choose = None;
    let mut timeline = None;
    // Robin's scenario: the migration step that asks Robin.
    let mut migrate = None;
    if app {
        let deps = run("app/update-deps", "update-deps-1");
        let step = |name: &str| {
            deps.steps
                .iter()
                .find(|step| step.step == name)
                .unwrap()
                .clone()
        };
        let (audit, step_update) = (step("audit"), step("update"));
        let maintainer = audit.assigned_to.clone().unwrap();
        let changelogs = step("changelogs");
        let reviewer = changelogs.assigned_to.clone().unwrap();
        work(
            &audit.subject,
            &maintainer,
            "claim",
            "audit-claim",
            "Listing outdated dependencies",
        );
        work(
            &audit.subject,
            &maintainer,
            "complete",
            "audit-complete",
            "Three outdated: parser 2.4.1, vitest 2.1.4, zod 3.23.8",
        );
        // A completion verifies the step's gates in the daemon's loop, which this fixture does not run.
        // The audit step has none, so it is marked completed as that loop would.
        store
            .set_step_state(&audit.subject, "completed", None)
            .unwrap();
        // With the audit done, Ada's step is ready for her, as that loop would make it.
        store
            .set_step_state(&step("choose-parser").subject, "ready", None)
            .unwrap();
        work(
            &step_update.subject,
            &maintainer,
            "claim",
            "update-claim",
            "Updating parser, vitest and zod",
        );
        work(
            &step_update.subject,
            &maintainer,
            "progress",
            "update-progress",
            "Updated parser, vitest and zod. Parser 3.0 breaks three CLI tests.",
        );
        // The reviewer reads the changelogs meanwhile, so the mission has two agents at work.
        work(
            &changelogs.subject,
            &reviewer,
            "claim",
            "changelogs-claim",
            "Reading the changelogs",
        );
        work(
            &changelogs.subject,
            &reviewer,
            "progress",
            "changelogs-progress",
            "vitest and zod are minor. Parser 3.0 drops the old flag syntax.",
        );
        // Both agents run Codex and are working. The TUI shows the maintainer's conversation (with
        // "Conversation first" on): its native Codex session, merged with its messages.
        let _ = std::fs::remove_dir_all(dir.join("home"));
        let _ = std::fs::remove_dir_all(dir.join("drivers"));
        let mut notes = CodexSession::working(&store, &dir, &reviewer);
        notes.say(
            "user",
            "Mission app/update-deps, step changelogs: read the changelogs of every update.",
        );
        notes.shell(
            "gh release view v3.0.0 -R acme/parser",
            Some("Breaking: `--flag=value` is gone. Pass `--flag value`."),
        );
        let mut entries = CodexSession::working(&store, &dir, &maintainer);
        entries.say(
            "user",
            "Mission app/update-deps, step update: a deps PR with CI green.",
        );
        entries.say(
            "assistant",
            "Updating the three outdated packages from the audit, then the tests.",
        );
        entries.shell("pnpm update parser@3 vitest zod && pnpm test", Some(
            "Tests  211 passed, 3 failed\n  cli/flags.test.ts  unknown flag `--out=dist`: parser 3 reads `--out dist`",
        ));
        entries.say(
            "assistant",
            "Parser 3.0 drops the `--flag=value` syntax and the CLI uses it in three places. Asking the reviewer.",
        );
        timeline = Some(entries);
        update = Some(step_update);
        choose = Some(step("choose-parser"));
    } else {
        let index = run("marrowpine/invoice-index", "invoice-index-1");
        let step_migrate = index
            .steps
            .iter()
            .find(|step| step.step == "migrate")
            .unwrap()
            .clone();
        let migrator = step_migrate.assigned_to.clone().unwrap();
        work(
            &step_migrate.subject,
            &migrator,
            "claim",
            "migrate-claim",
            "Rehearsing on last night's snapshot",
        );
        work(
            &step_migrate.subject,
            &migrator,
            "progress",
            "migrate-progress",
            "Rehearsed: a 4 minute write lock on invoices. Asking Robin when to run it.",
        );
        let release = run("marrowpine/release", "release-2-3");
        let build = release.steps[0].clone();
        let oncall = build.assigned_to.clone().unwrap();
        work(
            &build.subject,
            &oncall,
            "claim",
            "build-claim",
            "Building 2.3",
        );
        work(
            &build.subject,
            &oncall,
            "progress",
            "build-progress",
            "Tests pass, packaging the release",
        );
        migrate = Some(step_migrate);
    }

    // The background publications the daemon starts before it listens (crates/fractal/src/main.rs).
    // Without them the TUI reads attention unpresented, agents and missions unknown.
    fractal_daemon::api::start_agent_roster(&state);
    fractal_daemon::api::start_attention_list(&state);
    fractal_daemon::api::start_published_lists(&state);
    let served = socket.clone();
    let server = tokio::spawn(async move {
        fractal_daemon::api::serve_unix(&served, fractal_daemon::api::router(state)).await
    });
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    if let Some(migrate) = migrate {
        let migrator = migrate.assigned_to.clone().unwrap();
        fractal(
            &dir,
            &socket,
            &[
                "work",
                "ask",
                "--for",
                "person/robin",
                "--title",
                "Run the invoice migration now?",
                "--reason",
                "It locks invoice saving for about four minutes. Only you can choose when customers notice.",
                "--step",
                &migrate.subject,
                "--as",
                &migrator,
                "--incarnation",
                &format!("{migrator}:1"),
                "--idempotency-key",
                "website-ask",
            ],
        );
    }
    if let (Some(update), Some(choose), Some(mut entries)) = (update, choose, timeline) {
        let maintainer = update.assigned_to.clone().unwrap();
        let reviewer = "agent/app/reviewer";
        let send = |from: &str, to: &str, subject: &str, body: &str| {
            fractal(
                &dir,
                &socket,
                &[
                    "conversations",
                    "send",
                    to,
                    "--from",
                    from,
                    "--subject",
                    subject,
                    "--body",
                    body,
                ],
            );
        };
        send(
            &maintainer,
            reviewer,
            "Deps PR",
            "The dependency update is ready. Can you review it?",
        );
        send(
            reviewer,
            &maintainer,
            "Re: Deps PR",
            "Parser 3.0 drops the old flag syntax. The CLI breaks on it.",
        );
        // Ada's step is declared in the mission, so it shows by its name. She answers it.
        fractal(
            &dir,
            &socket,
            &[
                "work",
                "done",
                &choose.subject,
                "--as",
                "person/ada",
                "--summary",
                "Ship 3.0 and migrate the CLI.",
            ],
        );
        work(
            &update.subject,
            &maintainer,
            "progress",
            "update-resume-progress",
            "Ada chose parser 3.0. Moving the CLI to its flag syntax, then CI.",
        );
        entries.say(
            "assistant",
            "Ada chose 3.0. Moving the CLI to the new flag syntax.",
        );
        entries.shell(
            "rg -l -- '--[a-z-]+=' cli/",
            Some("cli/flags.ts\ncli/build.ts\ncli/flags.test.ts"),
        );
        entries.shell("pnpm test cli", None);
        send(
            &maintainer,
            reviewer,
            "Re: Deps PR",
            "Migrating the CLI to 3.0. Back to you when CI is green.",
        );
    }
    println!("CAPTURE READY {}", socket.display());
    let mut beat = std::time::Instant::now() - std::time::Duration::from_secs(60);
    while !dir.join("stop").exists() {
        // The app seed's agents are working Codex sessions. A real one's delivery process polls
        // its mailbox and reports itself every few seconds. Without that the daemon calls the
        // agent's message path stopped and the TUI shows it broken.
        if app && beat.elapsed() > std::time::Duration::from_secs(10) {
            let report = json!({
                "transport": "app-server",
                "pid": std::process::id(),
                "image": fractal_drivers::reexec::running_identity().map(|identity| identity.token()),
                "ready": true,
            })
            .to_string();
            for agent in ["agent/app/maintainer", "agent/app/reviewer"] {
                fractal_daemon::delivery_presence::record(agent, &report);
            }
            beat = std::time::Instant::now();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    server.abort();
}

/// Runs the CLI against the fixture, as an agent or a person would, and fails on any error.
fn fractal(dir: &Path, socket: &Path, args: &[&str]) -> Output {
    let output = fractal_daemon::test_support::command(test_bin!("fractal-fixture"))
        .env("HOME", dir.join("home"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("ST_AGENT")
        .env_remove("FRACTAL_AGENT")
        .env_remove("ST_MISSION_RUN")
        .env_remove("ST3_INCARNATION")
        .env_remove("FRACTAL_INCARNATION")
        .args(["--endpoint", socket.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A native Codex session file, as Codex writes it: one `response_item` record per line.
struct CodexSession {
    path: PathBuf,
    calls: u64,
}

impl CodexSession {
    /// Seeds `agent` as a managed Codex agent that is working: its runtime and harness
    /// observations, the driver state binding its provider incarnation to the native thread, and
    /// the native session file, which is the conversation's source. Returns that session.
    fn working(store: &Store, dir: &Path, agent: &str) -> Self {
        let name = agent.trim_start_matches("agent/");
        let incarnation = format!("{agent}:1");
        let provider_incarnation = format!("codex:{name}:1");
        let native_id = format!("website-{}", name.replace('/', "-"));
        let now = chrono::Utc::now();
        let path = dir
            .join("home/.codex/sessions")
            .join(now.format("%Y/%m/%d").to_string())
            .join(format!("rollout-{native_id}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n",
                json!({"type": "session_meta", "timestamp": now.to_rfc3339(),
                       "payload": {"id": native_id, "cwd": "/work/app", "source": "test"}})
            ),
        )
        .unwrap();
        let driver = dir
            .join("drivers")
            .join(&hex::encode(Sha256::digest(agent.as_bytes()))[..24])
            .join("state");
        std::fs::create_dir_all(&driver).unwrap();
        std::fs::write(
            driver.join("runtime.json"),
            json!({"agent": name, "incarnation": provider_incarnation}).to_string(),
        )
        .unwrap();
        std::fs::write(
            driver.join("binding.json"),
            json!({"agent": name, "runtimeIncarnation": provider_incarnation, "threadId": native_id})
                .to_string(),
        )
        .unwrap();
        for (kind, fields) in [
            (
                "runtime.observed",
                json!({"status": "running", "runtime_id": format!("{name}-runtime"),
                       "incarnation_id": incarnation, "terminal": false}),
            ),
            (
                "harness.observed",
                json!({"state": "working", "driver": "codex", "incarnation_id": incarnation,
                       "evidence_incarnation": provider_incarnation}),
            ),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: agent.into(),
                    kind: kind.into(),
                    actor: Some(agent.into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("{agent}:{kind}")),
                })
                .unwrap();
        }
        CodexSession { path, calls: 0 }
    }

    fn append(&mut self, payload: Value) {
        use std::io::Write as _;
        let record = json!({
            "type": "response_item", "timestamp": chrono::Utc::now().to_rfc3339(), "payload": payload,
        });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .unwrap();
        writeln!(file, "{record}").unwrap();
    }

    fn say(&mut self, role: &str, text: &str) {
        let part = if role == "user" {
            "input_text"
        } else {
            "output_text"
        };
        self.append(json!({
            "type": "message", "role": role, "content": [{ "type": part, "text": text }],
        }));
    }

    /// A shell call. Without output it is still running.
    fn shell(&mut self, command: &str, output: Option<&str>) {
        self.calls += 1;
        let call = format!("call-{}", self.calls);
        self.append(json!({
            "type": "function_call", "call_id": call, "name": "shell",
            "arguments": json!({ "command": command }).to_string(),
        }));
        if let Some(output) = output {
            self.append(
                json!({ "type": "function_call_output", "call_id": call, "output": output }),
            );
        }
    }
}
