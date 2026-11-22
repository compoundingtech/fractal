use serde_json::json;
use fractal_daemon::api::{AppState, router};
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
    let mut command = fractal_daemon::test_support::async_command(test_env!("CARGO_BIN_EXE_fractal-fixture"));
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
    rusqlite::Connection::open(root.path().join("source.db")).unwrap()
        .execute("INSERT OR REPLACE INTO meta(key,value) VALUES('local_node_writer','alder')", []).unwrap();
    for index in 0..1100 {
        // Plain replicated claims: diagnostics as an older build wrote them.
        store
            .append_as_older_build_for_test(&ClaimInput {
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
    let state = AppState {
        browser_pairing_login: None,
        seat_admission: Default::default(),
        owning_person: None,
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0).0,
        node: "alder".into(),
        api_socket: root.path().join("api.sock"),
        state_dir: root.path().into(),
        pty_root: root.path().join("pty"),
        pty_binary: PathBuf::from("pty"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
        harness_question_sink: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    };
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    let endpoint = Endpoint::Unix(socket.clone());
    let client = Client::new(endpoint.clone());
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
    let streamed = backup_cli(root.path(), &socket)
        .args(["backup", "create", "-"])
        .output().await.unwrap();
    let header = assert_stdout_receipt(&streamed);
    let streamed_file = root.path().join("streamed.jsonl");
    std::fs::write(&streamed_file, &streamed.stdout).unwrap();
    assert_restores_snapshot(root.path(), &socket, &streamed_file, &header, "streamed-restored.db").await;
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
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router = axum::Router::new().route("/v1/backup", axum::routing::get(|| async {
        let mut bytes = serde_json::to_vec(&backup::Record::Failure {
            code: "backup-fixture-io".into(),
            message: "export fixture ran out of disk space".into(),
        }).unwrap();
        bytes.push(b'\n');
        ([(axum::http::header::CONTENT_TYPE, "application/x-ndjson")], bytes)
    }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
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

#[tokio::test]
async fn configured_store_directory_backup_roundtrip_keeps_bodies_in_product_state() {
    configured_store_directory_roundtrip(false).await;
    #[cfg(unix)]
    configured_store_directory_roundtrip(true).await;
}

async fn configured_store_directory_roundtrip(database_alias: bool) {
    use fractal_daemon::message_body;
    use sha2::{Digest as _, Sha256};

    let root = tempfile::tempdir().unwrap();
    let source_root = root.path().join("source");
    let source_state = source_root.join("state/st3");
    let source_database_dir = root.path().join("source-database");
    let source_database = source_database_dir.join("claims.sqlite3");
    std::fs::create_dir_all(source_root.join("config/st3")).unwrap();
    std::fs::create_dir_all(&source_database_dir).unwrap();
    std::fs::write(
        source_root.join("config/st3/config.toml"),
        format!("node = \"alder\"\nstore_dir = {:?}\n", source_database_dir),
    ).unwrap();
    let text = "long message body\n".repeat(1024);
    let subject = "message/separate-store";
    let hash = hex::encode(Sha256::digest(text.as_bytes()));
    let source = Store::open(&source_database, "alder").unwrap();
    source.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: "message.sent".into(),
        actor: None,
        fields: BTreeMap::from([
            ("from".into(), json!("person/alex")),
            ("to".into(), json!("agent/worker")),
            ("content".into(), json!(message_body::preview(&text))),
            ("status".into(), json!("sent")),
            ("attachments".into(), json!([{
                "sha256": hash, "media_type": "text/plain",
                "size": text.len(), "origin": "host/alder"
            }])),
        ]),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: Some("separate-store-body".into()),
    }).unwrap();
    message_body::bodies(&source_state).put("person/alex", subject, text.as_bytes()).unwrap();
    let message = source.message(subject).unwrap().unwrap();
    assert_eq!(message.body_bytes, Some(text.len() as u64));
    assert_eq!(message.body_origin.as_deref(), Some("host/alder"));
    drop(source);
    let archive = root.path().join("separate-store.jsonl");
    let socket = root.path().join("unused.sock");
    let source_database_argument = if database_alias {
        #[cfg(unix)]
        {
            let alias = root.path().join("source-database-alias");
            std::os::unix::fs::symlink(&source_database_dir, &alias).unwrap();
            alias.join("claims.sqlite3")
        }
        #[cfg(not(unix))]
        {
            unreachable!("database alias coverage requires Unix symlinks")
        }
    } else {
        source_database.clone()
    };
    let output = backup_cli(&source_root, &socket)
        .args(["backup", "create"]).arg(&archive)
        .arg("--database").arg(&source_database_argument)
        .output().await.unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!source_database_dir.join("message-bodies").exists());
    assert_eq!(message_body::bodies(&source_state).read("person/alex", subject).unwrap().unwrap(), text.as_bytes());

    let restored_root = root.path().join("restored");
    let restored_state = restored_root.join("state/st3");
    let restored_database_dir = root.path().join("restored-database");
    std::fs::create_dir_all(restored_root.join("config/st3")).unwrap();
    std::fs::write(
        restored_root.join("config/st3/config.toml"),
        format!("node = \"unused\"\nstore_dir = {:?}\n", restored_database_dir),
    ).unwrap();
    let output = backup_cli(&restored_root, &socket)
        .args(["backup", "restore"]).arg(&archive)
        .output().await.unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: backup::RestoreReport = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.projections_match);
    assert_eq!(report.message_bodies_restored, 1);
    assert_eq!(report.message_bodies_referenced, 1);
    assert_eq!(report.message_bodies_unmatched, 0);
    assert_eq!(message_body::bodies(&restored_state).read("person/alex", subject).unwrap().unwrap(), text.as_bytes());
    assert!(!restored_database_dir.join("message-bodies").exists());
    assert!(!restored_state.join("claims.sqlite3").exists());
    let restored = Store::open(&restored_database_dir.join("claims.sqlite3"), &report.writer).unwrap();
    assert_eq!(restored.message(subject).unwrap().unwrap().body_bytes, Some(text.len() as u64));
}

fn assert_stdout_receipt(output: &std::process::Output) -> backup::Header {
    use sha2::{Digest as _, Sha256};

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    let lines: Vec<_> = output.stdout.split_inclusive(|byte| *byte == b'\n').collect();
    assert!(lines.iter().all(|line| line.ends_with(b"\n")));
    let backup::Record::Header(header) = serde_json::from_slice(lines[0]).unwrap() else {
        panic!("stdout must start with the unchanged archive header");
    };
    let end: serde_json::Value = serde_json::from_slice(lines.last().unwrap()).unwrap();
    assert_eq!(end["record"], "end");
    let sha256 = end["data"]["sha256"].as_str().unwrap();
    let end_offset = output.stdout.len() - lines.last().unwrap().len();
    assert_eq!(sha256, hex::encode(Sha256::digest(&output.stdout[..end_offset])));
    assert_eq!(receipt["format"], header.format);
    assert_eq!(receipt["version"], header.version);
    assert_eq!(receipt["records"], lines.len() as u64);
    assert_eq!(receipt["bytes"], output.stdout.len() as u64);
    assert_eq!(receipt["sha256"], hex::encode(Sha256::digest(&output.stdout)));
    assert_ne!(receipt["sha256"], sha256, "receipt hashes the end record too");
    assert_eq!(receipt["writer_id"], "alder");
    assert!(receipt["max_store_index"].as_u64().unwrap() > 0);
    assert!(receipt["peak_wal_growth"].as_u64().unwrap() <= 4294967296);
    let peaks = receipt["peak_wal_growth_by_file"].as_object().unwrap();
    assert!(peaks.values().all(|value| value.as_u64().is_some()));
    assert_eq!(receipt["peak_wal_growth_by_file"], end["data"].get("peak_wal_growth_by_file").cloned().unwrap_or_else(|| json!({})));
    let started = receipt["started_at"].as_str().unwrap().parse::<chrono::DateTime<chrono::Utc>>().unwrap();
    let finished = receipt["finished_at"].as_str().unwrap().parse::<chrono::DateTime<chrono::Utc>>().unwrap();
    assert!(finished >= started);
    header
}

async fn assert_restores_snapshot(
    root: &Path,
    socket: &Path,
    archive: &Path,
    header: &backup::Header,
    destination: &str,
) {
    let output = backup_cli(root, socket)
        .args(["backup", "restore"]).arg(archive)
        .arg("--database").arg(root.join(destination))
        .output().await.unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: backup::RestoreReport = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.projections_match);
    assert_eq!(report.source_graph_digest, header.graph_digest);
    assert_eq!(report.graph_digest, header.graph_digest);
    assert_eq!(report.log_digest, header.log_digest);
    assert_eq!(report.tables, header.tables);
}

#[tokio::test]
async fn offline_stdout_backup_preserves_source_bytes_and_restores_the_source_graph_and_log() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/st3")).unwrap();
    std::fs::write(root.path().join("config/st3/config.toml"), "node = \"alder\"\n").unwrap();
    let source_directory = root.path().join("source-store");
    std::fs::create_dir(&source_directory).unwrap();
    let database = source_directory.join("source.db");
    let source = Store::open(&database, "alder").unwrap();
    source.put_document("doc/backup", b"Offline streaming backup.\n", &None, "backup-document").unwrap();
    let mut expected_archive = Vec::new();
    let expected = source.write_backup_with_bodies(&mut expected_archive, None).unwrap();
    drop(source);
    rusqlite::Connection::open(&database).unwrap()
        .execute("INSERT OR REPLACE INTO meta(key,value) VALUES('local_node_writer','alder')", []).unwrap();
    let source_paths = [
        database.clone(), source_directory.join("source.db-wal"), source_directory.join("source.db-shm"),
    ];
    let digests = || {
        use sha2::{Digest as _, Sha256};
        source_paths.iter().map(|path| std::fs::read(path).ok().map(|bytes| hex::encode(Sha256::digest(bytes)))).collect::<Vec<_>>()
    };
    let directory_entries = || {
        let mut entries = std::fs::read_dir(&source_directory).unwrap()
            .map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        entries.sort();
        entries
    };
    let before = digests();
    let entries_before = directory_entries();
    let socket = root.path().join("unused.sock");
    let output = backup_cli(root.path(), &socket)
        .args(["backup", "create", "-", "--database"]).arg(&database)
        .args(["--max-wal-growth", "4294967296"])
        .output().await.unwrap();
    let header = assert_stdout_receipt(&output);
    assert_eq!(header.graph_digest, expected.graph_digest);
    assert_eq!(header.log_digest, expected.log_digest);
    assert_eq!(digests(), before, "source database, WAL and SHM must remain unchanged");
    assert_eq!(directory_entries(), entries_before, "offline export must not create or remove files");
    let archive = root.path().join("saved-stdout.jsonl");
    std::fs::write(&archive, &output.stdout).unwrap();
    assert_restores_snapshot(root.path(), &socket, &archive, &header, "restored/claims.sqlite3").await;
}

#[tokio::test]
async fn truncated_http_stdout_backup_fails_without_a_success_receipt() {
    use axum::extract::Query;

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/st3")).unwrap();
    std::fs::write(root.path().join("config/st3/config.toml"), "node = \"alder\"\n").unwrap();
    let source = Store::open_memory("alder").unwrap();
    source.put_document("doc/truncated", b"Incomplete archive.\n", &None, "truncated-document").unwrap();
    let mut archive = Vec::new();
    source.write_backup_with_bodies(&mut archive, None).unwrap();
    let end_offset = archive[..archive.len() - 1].iter().rposition(|byte| *byte == b'\n').unwrap() + 1;
    archive.truncate(end_offset);
    let app = axum::Router::new().route("/v1/backup", axum::routing::get(
        move |Query(query): Query<BTreeMap<String, String>>| {
            let archive = archive.clone();
            async move {
                assert_eq!(query.get("stream").map(String::as_str), Some("true"));
                assert_eq!(query.get("max_wal_growth").map(String::as_str), Some("4294967296"));
                ([
                    ("content-type", "application/x-ndjson"),
                    ("x-fractal-backup-stream", "1"),
                ], archive)
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let output = {
        let mut command = fractal_daemon::test_support::async_command(test_env!("CARGO_BIN_EXE_fractal-fixture"));
        command.env("XDG_CONFIG_HOME", root.path().join("config"));
        command.env("XDG_STATE_HOME", root.path().join("state"));
        command.env("XDG_DATA_HOME", root.path().join("data"));
        command.args(["--json", "--endpoint"]).arg(format!("http://{address}"))
            .args(["backup", "create", "-"]).output().await.unwrap()
    };
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    for line in output.stderr.split(|byte| *byte == b'\n') {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) {
            assert!(value.get("sha256").is_none(), "failed export emitted a success receipt: {value}");
        }
    }
    server.abort();
}
