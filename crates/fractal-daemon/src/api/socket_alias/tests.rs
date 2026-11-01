// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-01-socket-names-fractal — DELETE at contraction
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
            async move { axum::Json(count) }
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
        reserved, Router::new().route("/test", axum::routing::get(|| async { axum::Json(42) })),
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
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-01-socket-names-fractal
