//! What a requester reports when a peer answers an exchange without the fleet's signature. The
//! answer is never trusted as data: a refusal is reported by its status, with a fixed reason only
//! for this protocol's own unsigned refusals, and an unsigned success still fails authentication.
//! Retained diagnostics never quote refusal bodies, signed or unsigned.
use axum::http::StatusCode as AnswerStatus;
use axum::response::IntoResponse as _;
use tokio::net::TcpListener;

use super::*;

const FLEET: &str = "1f91ca65-7793-48cc-866e-ac15690130e1";
const PEER: &str = "invented-peer";

fn auth() -> FleetAuth {
    FleetAuth::test(FLEET, &[7; 32])
}

/// Post one exchange-shaped request to a peer that answers every request with `answer`, and
/// return what the requester reports. The server is stopped and awaited before returning.
async fn answered_error(answer: impl Fn(Bytes) -> Response + Clone + Send + Sync + 'static) -> anyhow::Error {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        EXCHANGE_PATH,
        post(move |body: Bytes| {
            let answer = answer.clone();
            async move { answer(body) }
        }),
    );
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let peer = PeerConfig {
        name: PEER.into(),
        url: format!("http://{address}"),
    };
    let result = post_signed_to::<_, serde_json::Value>(
        &replication_http_client(),
        &peer,
        "invented-node",
        &auth(),
        &FleetContext::legacy(BTreeSet::from([PEER.into()])),
        EXCHANGE_PATH,
        &serde_json::json!({"inventory": {"digest": ""}}),
        false,
    )
    .await;
    server.abort();
    let _ = server.await;
    result.expect_err("none of these answers is accepted")
}

async fn answered(answer: impl Fn(Bytes) -> Response + Clone + Send + Sync + 'static) -> String {
    answered_error(answer).await.to_string()
}

async fn answered_unsigned(status: AnswerStatus, body: &'static str) -> String {
    answered(move |_| (status, body).into_response()).await
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
        "peer invented-peer answered an unsigned 500 Internal Server Error (28 body bytes, not shown)"
    );
    assert!(!reported.contains("invented-secret"));
    let empty = answered_unsigned(AnswerStatus::BAD_GATEWAY, "").await;
    assert_eq!(
        empty,
        "peer invented-peer answered an unsigned 502 Bad Gateway (0 body bytes, not shown)"
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

#[tokio::test]
async fn a_signed_refusal_retains_only_allowlisted_machine_diagnostics() {
    let error = answered_error(|request: Bytes| {
        let body = serde_json::to_vec(&PeerResponse::new(
            PEER,
            0,
            serde_json::json!({
                "code": "replication-exchange-metadata-too-large",
                "failure": {"phase": "follow-up-export"},
                "message": "secret request body must not appear in diagnostics"
            }),
        ))
        .unwrap();
        let headers = auth()
            .response_headers_for(
                EXCHANGE_PATH,
                PEER,
                &body,
                &FleetAuth::body_digest(&request),
            )
            .unwrap();
        let mut response = (AnswerStatus::INTERNAL_SERVER_ERROR, body).into_response();
        response.headers_mut().extend(headers);
        response
    })
    .await;
    let failure = error.downcast_ref::<ReplicationExchangeFailure>().unwrap();
    assert_eq!(failure.code, "replication-exchange-metadata-too-large");
    assert_eq!(failure.phase, ReplicationFailurePhase::FollowUpExport);
    assert_eq!(failure.http_status, Some(500));
    assert!(failure.cause.len() <= 512);
    let reported = error.to_string();
    assert!(!reported.contains("secret"));
    assert!(!failure.cause.contains("secret"));
}

#[tokio::test]
async fn a_refusal_with_any_signature_header_is_authenticated_not_excused() {
    for name in [HEADER_SIGNATURE, "x-fractal-signature"] {
        let reported = answered(move |_| {
            let mut response = (AnswerStatus::INTERNAL_SERVER_ERROR, "invented-body").into_response();
            response
                .headers_mut()
                .insert(name, HeaderValue::from_static("00"));
            response
        })
        .await;
        assert!(
            reported.contains("the signed message has no valid x-st3-fleet header"),
            "{reported}"
        );
        assert!(!reported.contains("unsigned"), "{reported}");
    }
}
