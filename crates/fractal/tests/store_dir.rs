use std::path::Path;

async fn refused_start(root: &Path, state: &Path, store: &Path) -> String {
    let config = root.join("config.toml");
    std::fs::write(&config, "node = \"orchid\"\n").unwrap();
    let mut command = fractal_daemon::test_support::async_command(test_env!("CARGO_BIN_EXE_fractal-fixture"));
    command.env("XDG_CONFIG_HOME", root.join("config"));
    command.env("XDG_STATE_HOME", root.join("default-state"));
    command.args(["up", "--config"]).arg(config)
        .arg("--state-dir").arg(state)
        .arg("--store-dir").arg(store)
        .arg("--socket").arg(root.join("daemon.sock"))
        .args(["--web-listen", "off"]);
    command.kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await.expect("invalid store directory must refuse startup promptly").unwrap();
    assert!(!output.status.success());
    assert!(!root.join("daemon.sock").exists());
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn daemon_refuses_missing_explicit_store_directory() {
    let root = fractal_daemon::test_support::memory_tempdir();
    let state = root.path().join("state");
    let stderr = refused_start(root.path(), &state, &root.path().join("missing")).await;
    assert!(stderr.contains("store_dir"), "{stderr}");
    assert!(!state.exists());
}

#[tokio::test]
async fn daemon_refuses_database_in_both_locations() {
    let root = fractal_daemon::test_support::memory_tempdir();
    let state = root.path().join("state");
    let store = root.path().join("store");
    std::fs::create_dir(&state).unwrap();
    std::fs::create_dir(&store).unwrap();
    std::fs::write(state.join("claims.sqlite3"), b"original").unwrap();
    std::fs::write(store.join("claims.sqlite3-wal"), b"copied sidecar").unwrap();
    let stderr = refused_start(root.path(), &state, &store).await;
    assert!(stderr.contains("both"), "{stderr}");
}

#[tokio::test]
async fn daemon_refuses_store_writer_that_disagrees_with_state_identity() {
    let root = fractal_daemon::test_support::memory_tempdir();
    let state = root.path().join("state");
    let store = root.path().join("store");
    std::fs::create_dir(&state).unwrap();
    std::fs::create_dir(&store).unwrap();
    std::fs::write(state.join("node-identity.json"), b"\"orchid\"\n").unwrap();
    let connection = rusqlite::Connection::open(store.join("claims.sqlite3")).unwrap();
    connection.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); INSERT INTO meta VALUES('backup_restore_writer','fern');").unwrap();
    drop(connection);
    let stderr = refused_start(root.path(), &state, &store).await;
    assert!(stderr.contains("fern") && stderr.contains("orchid"), "{stderr}");
}
