//! The fleet membership read falls back to the internal route only when the daemon has no public
//! route, never to hide a broken answer.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fractal_client::{Client, ClientError};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// A daemon on a Unix socket that answers the public route with `public` and counts the requests
/// that reach the internal one, which answers an empty membership.
async fn daemon(public: &'static str) -> (tempfile::TempDir, Client, Arc<AtomicUsize>) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("st.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let internal = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&internal);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = vec![0_u8; 4096];
            let read = stream.read(&mut request).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            let answer = if request.starts_with("GET /v1/internal/fleet/membership ") {
                counted.fetch_add(1, Ordering::SeqCst);
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 14\r\n\r\n{\"members\":[]}"
            } else {
                public
            };
            let _ = stream.write_all(answer.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (root, Client::unix(&path), internal)
}

#[tokio::test]
async fn a_daemon_without_the_public_route_is_read_through_the_internal_one() {
    let (_root, client, internal) = daemon("HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n").await;
    assert_eq!(client.fleet_membership().await.unwrap(), serde_json::json!({"members": []}));
    assert_eq!(internal.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_malformed_answer_from_the_public_route_is_not_hidden() {
    let (_root, client, internal) = daemon("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 8\r\n\r\nnot json").await;
    let error = client.fleet_membership().await.unwrap_err();
    assert!(matches!(error, ClientError::Protocol(_)) && !error.is_missing_route(), "{error}");
    assert_eq!(internal.load(Ordering::SeqCst), 0, "no fallback");
}

#[tokio::test]
async fn a_refusal_from_the_public_route_is_not_hidden() {
    let body = r#"{"code":"forbidden","message":"refused","retryable":false}"#;
    let answer: &'static str = Box::leak(
        format!("HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len())
            .into_boxed_str(),
    );
    let (_root, client, internal) = daemon(answer).await;
    let error = client.fleet_membership().await.unwrap_err();
    assert!(!error.is_missing_route(), "{error}");
    assert_eq!(internal.load(Ordering::SeqCst), 0, "no fallback");
}

#[tokio::test]
async fn a_404_with_a_body_from_the_public_route_is_not_taken_as_missing() {
    let (_root, client, internal) = daemon("HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: 8\r\n\r\nnot json").await;
    let error = client.fleet_membership().await.unwrap_err();
    assert!(!error.is_missing_route(), "{error}");
    assert_eq!(internal.load(Ordering::SeqCst), 0, "no fallback");
}
