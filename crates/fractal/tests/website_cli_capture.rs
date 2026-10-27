#![cfg(unix)]
//! Seeds an in-memory fixture daemon for the fractal.how CLI capture and serves it until stopped.
//!
//! Run (ignored by default):
//!   CAPTURE_DIR=/tmp/cli-capture cargo test -p fractal --features test-support \
//!     --test website_cli_capture -- --ignored --nocapture
//! The daemon listens on $CAPTURE_DIR/st3.sock until $CAPTURE_DIR/stop exists. Every name is invented.
//! scripts/website-cli-capture/capture.sh then prints the transcript.

#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
mod ci_test_paths;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fractal_daemon::MissionsStore;
use fractal_daemon::api::AppState;
use fractal_daemon::model::{IntentInput, MissionRunRequest, WorkRequest};
use fractal_daemon::store::Store;
use tokio::sync::{Notify, watch};

const SOURCE: &str = r#"version 2
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "serves a seeded fixture daemon for the website CLI capture until $CAPTURE_DIR/stop exists"]
async fn serve_seeded_fixture_for_the_website_cli_capture() {
    let dir = PathBuf::from(std::env::var("CAPTURE_DIR").expect("CAPTURE_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("st3.sock");
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(dir.join("stop"));
    let state = AppState { owning_person: None,
    store: Arc::new(Store::open_memory("studio").unwrap()),
    notify: Arc::new(Notify::new()),
    event_notify: watch::channel(0_u64).0,
    node: "studio".into(),
    state_dir: dir.clone(),
    pty_root: dir.join("pty"),
    pty_binary: PathBuf::from("pty"),
    fleet_id: None,
    configured_peers: Vec::new(),
    client_relay: None,
    native_session_home: None,
    planner_default: fractal_daemon::model::PlannerSpec::default(), assistant_receipts: Default::default(), private_notes: Default::default(),  };
    let store = state.store.clone();
    let intent = fractal_daemon::graph::parse_intent(SOURCE, "studio").unwrap();
    let planned = store
        .mission(&intent, IntentInput { kdl: SOURCE.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &planned.subject_tokens, "website-capture").unwrap();
    let run = |mission: &str, key: &str| {
        store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: "/srv/marrowpine".into(),
                requester: Some("person/robin".into()),
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
    let index = run("marrowpine/invoice-index", "invoice-index-1");
    let migrate = index.steps.iter().find(|step| step.step == "migrate").unwrap().clone();
    let migrator = migrate.assigned_to.clone().unwrap();
    work(&migrate.subject, &migrator, "claim", "migrate-claim", "Rehearsing on last night's snapshot");
    work(
        &migrate.subject,
        &migrator,
        "progress",
        "migrate-progress",
        "Rehearsed: a 4 minute write lock on invoices. Asking Robin when to run it.",
    );
    let release = run("marrowpine/release", "release-2-3");
    let build = release.steps[0].clone();
    let oncall = build.assigned_to.clone().unwrap();
    work(&build.subject, &oncall, "claim", "build-claim", "Building 2.3");
    work(&build.subject, &oncall, "progress", "build-progress", "Tests pass, packaging the release");

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
    let ask = fractal_daemon::test_support::command(test_bin!("st3-fixture"))
        .env("HOME", dir.join("home"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("ST_AGENT")
        .env_remove("FRACTAL_AGENT")
        .env_remove("ST_MISSION_RUN")
        .env_remove("ST3_INCARNATION")
        .env_remove("FRACTAL_INCARNATION")
        .args(["--endpoint", socket.to_str().unwrap(), "--json", "work", "ask", "--for", "person/robin"])
        .args(["--title", "Run the invoice migration now?"])
        .args(["--reason", "It locks invoice saving for about four minutes. Only you can choose when customers notice."])
        .args(["--step", &migrate.subject, "--as", &migrator, "--incarnation", &format!("{migrator}:1")])
        .args(["--idempotency-key", "website-ask"])
        .output()
        .unwrap();
    assert!(ask.status.success(), "{}", String::from_utf8_lossy(&ask.stderr));
    println!("CAPTURE READY {}", socket.display());
    while !dir.join("stop").exists() {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    server.abort();
}
