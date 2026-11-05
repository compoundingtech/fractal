// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-01-socket-names-fractal — DELETE at contraction
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::os::unix::fs::FileTypeExt as _;

fn response(value: usize) -> axum::Json<crate::model::ApiResponse<usize>> {
    axum::Json(crate::model::ApiResponse {
        api_version: "st3.v1".into(), request_id: "socket-alias-test".into(),
        snapshot_host: "test-node".into(), store_index: 1, value,
    })
}

#[tokio::test]
async fn primary_and_legacy_clients_reach_the_same_handler() {
    for (primary, legacy) in [("fractal.sock", "st3.sock"), ("fractal-client.sock", "st3-client.sock")] {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join(primary);
        let alias = root.path().join(legacy);
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let app = Router::new().route("/test", axum::routing::get(move || {
            let count = handler_calls.fetch_add(1, Ordering::SeqCst) + 1;
            async move { response(count) }
        }));
        let reserved = reserve_unix_socket(&socket, None).await.unwrap();
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new(primary));
        assert!(tokio::net::UnixStream::connect(&alias).await.is_err());
        let (ready, serving) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve_reserved_unix_with_ready(reserved, app, None, || {
            let _ = ready.send(());
        }));
        serving.await.unwrap();
        for (path, expected) in [(&socket, 1), (&alias, 2)] {
            // Explicit legacy Endpoint::Unix models an already installed old client.
            let client = crate::client::Client::new(crate::client::Endpoint::Unix(path.clone()));
            assert_eq!(client.get::<usize>("/test").await.unwrap(), expected);
        }
        assert!(reserve_unix_socket(&socket, None).await.is_err());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert!(fs::symlink_metadata(&alias).is_err());
        // A crashed primary and a stale legacy socket can both be reclaimed.
        drop(std::os::unix::net::UnixListener::bind(&alias).unwrap());
        drop(reserve_unix_socket(&socket, None).await.unwrap());
        assert!(fs::symlink_metadata(&alias).is_err());
    }
}

#[tokio::test]
async fn live_legacy_listener_and_pending_legacy_reservation_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("fractal.sock");
    let alias = root.path().join("st3.sock");
    let legacy = tokio::net::UnixListener::bind(&alias).unwrap();
    let inode = fs::metadata(&alias).unwrap().ino();
    assert!(reserve_unix_socket(&socket, None).await.is_err());
    assert_eq!(fs::metadata(&alias).unwrap().ino(), inode);
    drop(legacy);
    let pending = core::reserve_unix_socket(&alias, None).await.unwrap();
    assert!(reserve_unix_socket(&socket, None).await.is_err());
    drop(pending);
    assert!(reserve_unix_socket(&socket, None).await.is_ok());
}

#[tokio::test]
async fn legacy_state_discovery_reaches_a_custom_control_socket() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("custom/control.sock");
    let discovery = root.path().join("state/run/fractal.sock");
    let legacy = discovery.with_file_name("st3.sock");
    let reserved = reserve_unix_socket(&socket, Some(&discovery)).await.unwrap();
    assert!(!socket.with_file_name("st3.sock").exists());
    let (ready, serving) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_reserved_unix_with_ready(
        reserved, Router::new().route("/test", axum::routing::get(|| async { response(42) })),
        None, || { let _ = ready.send(()); },
    ));
    serving.await.unwrap();
    let client = crate::client::Client::new(crate::client::Endpoint::Unix(legacy.clone()));
    assert_eq!(client.get::<usize>("/test").await.unwrap(), 42);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    assert!(fs::symlink_metadata(legacy).is_err());
}

#[tokio::test]
async fn cleanup_does_not_unlink_a_replacement_and_custom_paths_have_no_alias() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("fractal-client.sock");
    let alias = root.path().join("st3-client.sock");
    let reserved = reserve_unix_socket(&socket, None).await.unwrap();
    fs::remove_file(&alias).unwrap();
    fs::write(&alias, "replacement").unwrap();
    drop(reserved);
    assert_eq!(fs::read_to_string(&alias).unwrap(), "replacement");
    let custom = root.path().join("custom.sock");
    drop(reserve_unix_socket(&custom, None).await.unwrap());
    assert!(!root.path().join("st3.sock").exists());
    assert_eq!(fs::read_to_string(&alias).unwrap(), "replacement");
    assert!(reserve_unix_socket(&socket, None).await.is_err());
    assert_eq!(fs::read_to_string(&alias).unwrap(), "replacement");
}

#[tokio::test]
async fn overlong_primary_names_serve_only_reachable_legacy_paths() {
    #[cfg(target_os = "macos")]
    let limit = 104;
    #[cfg(not(target_os = "macos"))]
    let limit = 108;
    for (primary, legacy) in [("fractal.sock", "st3.sock"), ("fractal-client.sock", "st3-client.sock")] {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let padding = limit - legacy.len() - root.path().as_os_str().as_bytes().len() - 3;
        let parent = root.path().join("x".repeat(padding));
        let socket = parent.join(primary);
        let legacy_socket = parent.join(legacy);
        let discovery = root.path().join("y".repeat(padding)).join(primary);
        let legacy_discovery = discovery.with_file_name(legacy);
        assert!(crate::config::validate_unix_socket_path(&socket, "--socket").is_err());
        crate::config::validate_unix_socket_path(&legacy_socket, "--socket").unwrap();
        let reserved = reserve_unix_socket(&socket, Some(&discovery)).await.unwrap();
        assert!(!socket.exists());
        assert!(!discovery.exists());
        let (ready, serving) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve_reserved_unix_with_ready(
            reserved, Router::new().route("/test", axum::routing::get(|| async { response(42) })),
            None, || { let _ = ready.send(()); },
        ));
        serving.await.unwrap();
        for path in [legacy_socket, legacy_discovery] {
            let client = crate::client::Client::new(crate::client::Endpoint::Unix(path));
            assert_eq!(client.get::<usize>("/test").await.unwrap(), 42);
        }
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn explicit_legacy_listener_is_not_reserved_as_its_own_discovery_alias() {
    for aliased_parent in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let run = root.path().join("state/run");
        fs::create_dir_all(&run).unwrap();
        let socket = run.join("st3.sock");
        let discovery_parent = if aliased_parent {
            let alias = root.path().join("run-alias");
            symlink(&run, &alias).unwrap();
            alias
        } else {
            run
        };
        let discovery = discovery_parent.join("fractal.sock");
        let reserved = reserve_unix_socket(&socket, Some(&discovery)).await.unwrap();
        assert!(fs::symlink_metadata(&socket).unwrap().file_type().is_socket());
        assert_eq!(fs::canonicalize(&socket).unwrap(), fs::canonicalize(&discovery).unwrap());
        let (ready, serving) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve_reserved_unix_with_ready(
            reserved, Router::new().route("/test", axum::routing::get(|| async { response(42) })),
            None, || { let _ = ready.send(()); },
        ));
        serving.await.unwrap();
        for path in [socket, discovery] {
            let client = crate::client::Client::new(crate::client::Endpoint::Unix(path));
            assert_eq!(client.get::<usize>("/test").await.unwrap(), 42);
        }
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn short_parent_symlinks_keep_alias_probes_addressable() {
    for (primary, legacy) in [("fractal.sock", "st3.sock"), ("fractal-client.sock", "st3-client.sock")] {
        for discovery_only in [false, true] {
            let root = tempfile::tempdir_in("/tmp").unwrap();
            let mut long_parent = root.path().to_path_buf();
            while crate::config::validate_unix_socket_path(&long_parent.join(primary), "--socket").is_ok() {
                long_parent = long_parent.join("long-socket-directory");
            }
            fs::create_dir_all(&long_parent).unwrap();
            let short_parent = root.path().join("run");
            symlink(&long_parent, &short_parent).unwrap();
            let published = short_parent.join(primary);
            let alias = short_parent.join(legacy);
            crate::config::validate_unix_socket_path(&published, "--socket").unwrap();
            assert!(crate::config::validate_unix_socket_path(
                &fs::canonicalize(&short_parent).unwrap().join(legacy), "--socket",
            ).is_err());
            let socket = if discovery_only { root.path().join("custom.sock") } else { published.clone() };
            let state_socket = discovery_only.then_some(published.as_path());
            let reserved = reserve_unix_socket(&socket, state_socket).await.unwrap();
            assert_eq!(fs::read_link(&alias).unwrap(), Path::new(primary));
            let (ready, serving) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(serve_reserved_unix_with_ready(
                reserved, Router::new().route("/test", axum::routing::get(|| async { response(42) })),
                None, || { let _ = ready.send(()); },
            ));
            serving.await.unwrap();
            for path in [published, alias.clone()] {
                let client = crate::client::Client::new(crate::client::Endpoint::Unix(path));
                assert_eq!(client.get::<usize>("/test").await.unwrap(), 42);
            }
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
            assert!(fs::symlink_metadata(alias).is_err());
        }
    }
}

#[tokio::test]
async fn retargeted_alias_parent_rejects_another_directorys_stale_probe() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let replacement = root.path().join("replacement");
    let supplied = root.path().join("run");
    fs::create_dir_all(&original).unwrap();
    fs::create_dir_all(&replacement).unwrap();
    symlink(&original, &supplied).unwrap();
    let socket = original.join("st3.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let metadata = fs::metadata(fs::canonicalize(&supplied).unwrap()).unwrap();
    let expected = (metadata.dev(), metadata.ino());
    check_alias_parent(&supplied, expected).unwrap();

    // Exercise the post-probe guard directly, deterministically retargeting between the two checks.
    fs::remove_file(&supplied).unwrap();
    symlink(&replacement, &supplied).unwrap();
    assert_eq!(
        tokio::net::UnixStream::connect(supplied.join("st3.sock")).await.unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
    );
    let error = check_alias_parent(&supplied, expected).unwrap_err();
    assert!(error.to_string().contains("changed during reservation"));
    assert!(fs::symlink_metadata(&socket).unwrap().file_type().is_socket());
    let _client = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let _accepted = listener.accept().await.unwrap();
    fs::remove_file(&supplied).unwrap();
    assert!(check_alias_parent(&supplied, expected).is_err());
}

// LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-01-socket-names-fractal
