use super::*;

/// Exercise source paging and the real envelope together, not just the generic cached
/// page helper. In particular a fetched final page must become a resumable prefix.
#[tokio::test]
async fn rich_source_pages_resume_after_the_last_delivered_item_without_loss() {
    for published_agents in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let suffix = "bounded-rich-source-".repeat(6);
        let declarations = (0..12).map(|index| format!(
            "agent \"worker-{index:02}-{suffix}\" {{ workspace \"/tmp\"; command \"true\"; restart always; }}"
        )).collect::<Vec<_>>().join("\n");
        let intent = crate::graph::parse_internal_intent(
            &format!("version 2\n{declarations}"), "reads-fixture",
        ).unwrap();
        state.store.apply_internal(&intent, "rich-source-envelope").unwrap();
        for index in 0..12 {
            state.store.ask_person(&serde_json::from_value(json!({
                "person": OWNER, "actor": format!("agent/reads-fixture.worker-00-{suffix}"),
                "title": format!("question {index}: {}", "決定\n\"".repeat(30)),
                "reason": "Grüße 決定", "new_run": format!("rich-page-{index}"),
                "idempotency_key": format!("rich-page-{index}")
            })).unwrap()).unwrap();
        }
        if published_agents {
            state.store.start_agent_roster_refresher().unwrap();
            crate::api::refresh_agent_roster(&state.store, false).unwrap();
        }
        state.store.start_attention_list_refresher().unwrap();
        crate::api::published_attention::refresh_attention_list(&state.store).unwrap();
        for projection in ["missions-rich", "attention-rich", "agents-rich"] {
            let admitted = new_client_snapshot(&state);
            let context = response_body::Context {
                state: state.clone(), client_request: true, local_only_response: false,
                request_id: "request/00000000-0000-0000-0000-000000000000".into(),
                client_snapshot: Some(admitted.clone()), server: None, roster: false,
            };
            let query = params(&[("limit", "12")]);
            let (full_snapshot, full) = project_with_context(&state, &admitted, projection, &query,
                MAX_RESPONSE_BYTES, "rich-source:1", &context).await.unwrap();
            assert!(full["page"]["next_cursor"].is_null());
            let expected = full["items"].as_array().unwrap();
            assert_eq!(expected.len(), 12);
            if projection == "attention-rich" {
                assert!(full.to_string().contains("決定"));
            }
            // Leave all source rows within the requested item limit while forcing the
            // actual envelope to shorten it. The full page has no continuation cursor.
            let budget = serde_json::to_vec(&json!({
                "api_version": CLIENT_API_VERSION, "request_id": context.request_id,
                "snapshot": full_snapshot, "value": full
            })).unwrap().len() - 1;
            let (mut snapshot, mut page) = project_with_context(&state, &admitted,
                projection, &query, budget, "rich-source:1", &context).await.unwrap();
            assert!(page["items"].as_array().unwrap().len() < 12);
            let mut delivered = Vec::new();
            loop {
                let bytes = serde_json::to_vec(&json!({
                    "api_version": CLIENT_API_VERSION, "request_id": context.request_id,
                    "snapshot": snapshot, "value": page
                })).unwrap().len();
                assert!(bytes <= budget, "{projection}: {bytes} > {budget}");
                let items = page["items"].as_array().unwrap();
                assert!(!items.is_empty());
                delivered.extend(items.iter().cloned());
                let Some(cursor) = page["page"]["next_cursor"].as_str() else { break };
                let decoded = decode_client_cursor(cursor).unwrap();
                assert_eq!(decoded.offset, delivered.len());
                if projection == "missions-rich" {
                    assert_eq!(decoded.after_key.as_ref().unwrap().1,
                        delivered.last().unwrap()["id"].as_str().unwrap());
                }
                (snapshot, page) = project_with_context(&state, &snapshot, projection,
                    &params(&[("cursor", cursor)]), budget, "rich-source:1", &context)
                    .await.unwrap();
            }
            assert_eq!(&delivered, expected, "{projection}; published={published_agents}");
            let error = project_with_context(&state, &admitted, projection, &query,
                1, "rich-source:1", &context).await.unwrap_err();
            assert_eq!((error.status, error.code.as_str()),
                (StatusCode::PAYLOAD_TOO_LARGE, "response-budget-exceeded"));
        }
    }
}
