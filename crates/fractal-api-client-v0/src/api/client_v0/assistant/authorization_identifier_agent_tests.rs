#![allow(unused_imports)]
use super::*;


#[tokio::test]
async fn assistant_publication_offsets_outside_filtered_membership_are_private_typed_410s() {
    for mode in ["approval-once", "caller-bound-allow"] {
        let mut fixture = fixture(SCOPES, true).await;
        let source = r#"version 2
agent "one" { workspace "/tmp"; command "true"; restart always; }
agent "two" { workspace "/tmp"; command "true"; restart always; }
"#;
        let intent = crate::graph::parse_internal_intent(source, "PRIVATE-offset-fixture").unwrap();
        fixture.state.store.apply_internal(&intent, "offset-fixture").unwrap();
        let rejected = format!("agent/PRIVATE-{}", "x".repeat(256));
        fixture.state.store.append_claim(&crate::model::ClaimInput {
            subject: rejected.clone(), kind: "runtime.observed".into(), actor: None,
            fields: serde_json::from_value(json!({
                "status": "running", "runtime_id": rejected, "incarnation_id": "one"
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        for id in ["one", "two"] {
            fixture.state.store.ask_person(&serde_json::from_value(json!({
                "person": OWNER, "actor": format!("agent/PRIVATE-offset-fixture.{id}"),
                "title": "PRIVATE offset title", "reason": "PRIVATE offset reason",
                "new_run": format!("offset-{id}"), "idempotency_key": format!("offset-{id}")
            })).unwrap()).unwrap();
        }
        crate::api::refresh_agent_roster(&fixture.state.store, false).unwrap();
        crate::api::published_attention::refresh_attention_list(&fixture.state.store).unwrap();
        projection_read_mode(&mut fixture, mode).await;
        for projection in ["agents", "agents-rich", "attention", "attention-rich"] {
            let query = "?limit=1&max_response_bytes=16384";
            let payload = fixture.payload(projection, query);
            let header = fixture.header(&payload, &fixture.attestor);
            let (status, value, _) = fixture.request(
                &format!("/v1/client/assistant/{projection}{query}"),
                (mode == "approval-once").then_some(header.as_str()),
            ).await;
            assert_eq!(status, StatusCode::OK, "{projection}, {mode}: {value}");
            let cursor = decode_client_cursor(value["value"]["page"]["next_cursor"].as_str().unwrap()).unwrap();
            // Two visible rows, even though the agent publication has a third source
            // member. Equality must expire, just like beyond-end and overflow offsets.
            for offset in [2, 3, usize::MAX] {
                let mut invalid = cursor.clone();
                invalid.offset = offset;
                let encoded = encode_client_cursor(&invalid).unwrap();
                let query = format!("?max_response_bytes=16384&cursor={}", urlencoding::encode(&encoded));
                let payload = fixture.payload(projection, &query);
                let header = fixture.header(&payload, &fixture.attestor);
                let (status, value, _) = fixture.request(
                    &format!("/v1/client/assistant/{projection}{query}"),
                    (mode == "approval-once").then_some(header.as_str()),
                ).await;
                assert_eq!(status, StatusCode::GONE, "{projection}, {mode}, offset={offset}: {value}");
                assert_eq!(value["code"], "page-cursor-expired", "{value}");
                assert!(value.get("value").is_none(), "{value}");
                assert!(!value.to_string().contains("PRIVATE"), "{value}");
            }
        }
    }
}
