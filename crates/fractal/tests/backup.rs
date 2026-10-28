use serde_json::json;
use fractal_daemon::api::{AppState, router, serve_unix};
use fractal_daemon::backup;
use fractal_daemon::client::{Client, Endpoint};
use fractal_daemon::model::ClaimInput;
use fractal_daemon::store::Store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, watch};

fn backup_cli(root: &Path, socket: &Path) -> tokio::process::Command {
    let mut command = fractal_daemon::test_support::async_command(test_env!("CARGO_BIN_EXE_st3-fixture"));
    command.env("XDG_CONFIG_HOME", root.join("config"));
    command.env("XDG_STATE_HOME", root.join("state"));
    command.env("XDG_DATA_HOME", root.join("data"));
    command.args(["--json", "--endpoint"]);
    command.arg(format!("unix://{}", socket.display()));
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_backup_download_restores_and_leaves_the_daemon_serving_reads() {
    let root = fractal_daemon::test_support::memory_tempdir();
    std::fs::create_dir_all(root.path().join("config/st3")).unwrap();
    std::fs::write(
        root.path().join("config/st3/config.toml"),
        r#"node = "alder"
"#,
    )
    .unwrap();
    let store = Arc::new(Store::open(&root.path().join("source.db"), "alder").unwrap());
    for index in 0..1100 {
        store
            .append_claim(&ClaimInput {
                subject: "daemon/alder".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("backup-test")),
                    ("reason".into(), json!(format!("note-{index}"))),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let socket = root.path().join("api.sock");
    let state = AppState { owning_person: None,
    store: store.clone(),
    notify: Arc::new(Notify::new()),
    event_notify: watch::channel(0).0,
    node: "alder".into(),
    state_dir: root.path().into(),
    pty_root: root.path().join("pty"),
    pty_binary: PathBuf::from("pty"),
    fleet_id: None,
    configured_peers: vec![],
    client_relay: None,
    native_session_home: None,
    planner_default: Default::default(), assistant_receipts: Default::default(), private_notes: Default::default(),  };
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        serve_unix(&server_socket, router(state)).await.unwrap();
    });
    let endpoint = Endpoint::Unix(socket.clone());
    let client = Client::new(endpoint.clone());
    for _ in 0..100 {
        if client.get::<serde_json::Value>("/v1/health").await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let file = root.path().join("backup.jsonl");
    let mut create = backup_cli(root.path(), &socket);
    create.args(["backup", "create"]).arg(&file);
    let download = tokio::spawn(async move { create.output().await.unwrap() });
    for _ in 0..10 {
        tokio::time::timeout(
            Duration::from_secs(2),
            client.get::<serde_json::Value>("/v1/claims?subject=daemon%2Falder"),
        )
        .await
        .unwrap()
        .unwrap();
    }
    let output = download.await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(backup::create(&endpoint, &file).await.is_err());
    let target = root.path().join("restored.db");
    let output = backup_cli(root.path(), &socket)
        .args(["backup", "restore"])
        .arg(&file)
        .arg("--database")
        .arg(&target)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: backup::RestoreReport = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.projections_match);
    assert_eq!(report.tables["claim_sources"].count, 1100);
    let remote = Store::open_memory("birch").unwrap();
    remote
        .put_document(
            "doc/remote",
            b"A replicated document.\n",
            &None,
            "remote-document",
        )
        .unwrap();
    let exchange = remote
        .export_replication_exchange("test-fleet", &Default::default())
        .unwrap();
    store
        .receive_replication_exchange("birch", "test-fleet", &exchange)
        .unwrap();
    store.validate_replication_backlog().unwrap();
    let pending = root.path().join("pending.jsonl");
    let error = backup::create(&endpoint, &pending).await.unwrap_err();
    assert!(error.to_string().contains("fully projected"), "{error:#}");
    assert!(!pending.exists());
    store.project_replication_backlog().unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_streamed_export_failure_names_the_cause_and_never_publishes_a_cli_backup() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/st3")).unwrap();
    std::fs::write(root.path().join("config/st3/config.toml"), "node = \"alder\"\n").unwrap();
    let socket = root.path().join("error.sock");
    let server_socket = socket.clone();
    let router = axum::Router::new().route("/v1/backup", axum::routing::get(|| async {
        let mut bytes = serde_json::to_vec(&backup::Record::Failure {
            code: "backup-fixture-io".into(),
            message: "export fixture ran out of disk space".into(),
        }).unwrap();
        bytes.push(b'\n');
        ([(axum::http::header::CONTENT_TYPE, "application/x-ndjson")], bytes)
    }));
    let server = tokio::spawn(async move { serve_unix(&server_socket, router).await.unwrap(); });
    // A missing endpoint is the only startup condition being awaited here.
    for _ in 0..100 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let archive = root.path().join("failed-backup.jsonl");
    let output = backup_cli(root.path(), &socket)
        .args(["backup", "create"]).arg(&archive).output().await.unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("backup-fixture-io"), "{error}");
    assert!(error.contains("export fixture ran out of disk space"), "{error}");
    assert!(!archive.exists());
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
        entry.unwrap().file_name().to_string_lossy().starts_with(".tmp")
    }), "the failed download must also remove its private temporary file");
    server.abort();
}
