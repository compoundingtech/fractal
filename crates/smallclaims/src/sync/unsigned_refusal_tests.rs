//! What a requester reports when a peer answers an exchange without the fleet's signature. The
//! answer is never trusted as data: a refusal is reported by its status, with a fixed reason only
//! for this protocol's own unsigned refusals, and an unsigned success still fails authentication.
use axum::http::StatusCode as AnswerStatus;
use axum::response::IntoResponse as _;
use tokio::net::TcpListener;

use super::*;

const FLEET: &str = "1f91ca65-7793-48cc-866e-ac15690130e1";

/// Post one exchange-shaped request to a peer that answers every request with `status` and
/// `body`, unsigned, and return what the requester reports.
async fn answered_unsigned(status: AnswerStatus, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        EXCHANGE_PATH,
        post(move |_: Bytes| async move { (status, body).into_response() }),
    );
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let peer = PeerConfig {
        name: "invented-peer".into(),
        url: format!("http://{address}"),
    };
    let result = post_signed_to::<_, serde_json::Value>(
        &replication_http_client(),
        &peer,
        "invented-node",
        &FleetAuth::test(FLEET, &[7; 32]),
        &FleetContext::legacy(BTreeSet::from(["invented-peer".into()])),
        EXCHANGE_PATH,
        &serde_json::json!({"inventory": {"digest": ""}}),
        false,
    )
    .await;
    server.abort();
    format!(
        "{:#}",
        result.expect_err("an unsigned answer is never accepted")
    )
}

#[tokio::test]
async fn an_unsigned_refusal_is_reported_by_its_status_and_reason() {
    for (status, body, expected) in [
        (
            AnswerStatus::BAD_REQUEST,
            "replication request body: the exchange body inflates past 67108864 bytes",
            "peer invented-peer refused the exchange with an unsigned 400 Bad Request: it could not \
             read the request body (larger than it accepts, or not valid deflate)",
        ),
        (
            AnswerStatus::UNAUTHORIZED,
            "replication authentication failed: the replication signature does not match",
            "peer invented-peer refused the exchange with an unsigned 401 Unauthorized: it could \
             not authenticate the request",
        ),
        (
            AnswerStatus::PAYLOAD_TOO_LARGE,
            "Failed to buffer the request body: length limit exceeded",
            "peer invented-peer refused the exchange with an unsigned 413 Payload Too Large: the \
             request body is larger than it accepts",
        ),
    ] {
        assert_eq!(answered_unsigned(status, body).await, expected);
    }
}

#[tokio::test]
async fn an_unsigned_refusal_with_another_body_is_counted_not_quoted() {
    let reported = answered_unsigned(
        AnswerStatus::INTERNAL_SERVER_ERROR,
        "invented-secret-looking-text",
    )
    .await;
    assert_eq!(
        reported,
        "peer invented-peer answered an unsigned 500 Internal Server Error (28 body bytes, not read)"
    );
    assert!(!reported.contains("invented-secret"));
    let empty = answered_unsigned(AnswerStatus::BAD_GATEWAY, "").await;
    assert_eq!(
        empty,
        "peer invented-peer answered an unsigned 502 Bad Gateway (0 body bytes, not read)"
    );
}

#[tokio::test]
async fn an_unsigned_success_still_fails_authentication() {
    let reported = answered_unsigned(AnswerStatus::OK, r#"{"value": {"envelopes": []}}"#).await;
    assert!(
        reported.contains("the signed message has no valid x-st3-fleet header"),
        "{reported}"
    );
}
