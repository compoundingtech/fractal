//! The version is an envelope label, independent of the registry and signed bytes.
use super::*;

#[tokio::test]
async fn peer_envelope_versions_are_accepted_by_each_signed_reader() {
    for version in ["st3.v1", "fractal.v1", "future.v9"] {
        for path in [EXCHANGE_PATH, CHECKPOINT_PATH, crate::sync::ANSWERS_PATH] {
            let auth = FleetAuth::test("fleet-test", &[7; 32]);
            let signing = auth.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let app = Router::new().route(
                path,
                post(move |request: Bytes| {
                    let signing = signing.clone();
                    async move {
                        let value = if path == CHECKPOINT_PATH {
                            serde_json::json!({"checkpoint": "test", "cut_unix_ms": 0,
                            "envelopes": [], "claims": []})
                        } else {
                            serde_json::json!({})
                        };
                        let mut envelope = PeerResponse::new("peer", 0, value);
                        assert_eq!(envelope.api_version, "st3.v1");
                        envelope.api_version = version.into();
                        let body = serde_json::to_vec(&envelope).unwrap();
                        let headers = signing
                            .response_headers_for(
                                path,
                                "peer",
                                &body,
                                &FleetAuth::body_digest(&request),
                            )
                            .unwrap();
                        let mut response = (StatusCode::OK, body).into_response();
                        response.headers_mut().extend(headers);
                        response
                    }
                }),
            );
            let server = tokio::spawn(axum::serve(listener, app).into_future());
            let peer = PeerConfig {
                name: "peer".into(),
                url: format!("http://{address}"),
            };
            let fleet = FleetContext::legacy(BTreeSet::from(["peer".into()]));
            let http = replication_http_client();
            let result = if path == CHECKPOINT_PATH {
                fetch_checkpoint_manifest_page(
                    &http,
                    &peer,
                    "reader",
                    &auth,
                    &fleet,
                    &CheckpointManifestRequest {
                        checkpoint: "test".into(),
                        cut_unix_ms: 0,
                        after: None,
                    },
                )
                .await
                .map(|_| ())
            } else if path == crate::sync::ANSWERS_PATH {
                answers_check::fetch_answers(
                    &http,
                    &peer,
                    "reader",
                    &auth,
                    &fleet,
                    &crate::answers::AnswersRequest::default(),
                )
                .await
                .map(|reply| assert!(reply.is_some()))
            } else {
                post_signed_to::<_, serde_json::Value>(
                    &http,
                    &peer,
                    "reader",
                    &auth,
                    &fleet,
                    path,
                    &serde_json::json!({}),
                    false,
                )
                .await
                .map(|_| ())
            };
            server.abort();
            let _ = server.await;
            if version == "future.v9" {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("the peer API version differs"),
                    "{path}"
                );
            } else {
                result.unwrap_or_else(|error| panic!("{path} rejected {version}: {error:#}"));
            }
        }
    }
}
