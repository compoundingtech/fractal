#![allow(unused_imports)]
use super::*;


fn seed_identifier_agents(state: &AppState, published: bool, mixed: bool) -> (Vec<String>, Vec<String>) {
    let rejected = vec![
        format!("agent/00-{}", "x".repeat(256)),
        "agent/01-決定".into(),
        "agent/02-hash#worker".into(),
    ];
    let allowed = if mixed {
        (0..12).map(|index| format!("agent/worker-{index:02}-{}", "bounded-id-".repeat(12)))
            .collect::<Vec<_>>()
    } else { Vec::new() };
    for id in rejected.iter().chain(&allowed) {
        state.store.append_claim(&crate::model::ClaimInput {
            subject: id.clone(), kind: "runtime.observed".into(), actor: None,
            fields: serde_json::from_value(json!({
                "status": "running", "runtime_id": id, "incarnation_id": "one"
            })).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    // Prove the native source contains the rejected identifiers: assistant membership,
    // rather than the fixture or native roster, must perform the exclusion.
    let refs = client_agent_page_refs(&state.store, false, state.store.index().unwrap()).unwrap();
    for id in rejected.iter().chain(&allowed) {
        assert!(refs.iter().any(|reference| reference["id"] == *id), "missing source {id}");
    }
    if published {
        state.store.start_agent_roster_refresher().unwrap();
        crate::api::refresh_agent_roster(&state.store, false).unwrap();
    }
    (allowed, rejected)
}

#[tokio::test]
async fn agent_identifier_membership_precedes_offsets_and_byte_packing() {
    for published in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let (expected, rejected) = seed_identifier_agents(&state, published, true);
        for projection in ["agents", "agents-rich"] {
            let admitted = new_client_snapshot(&state);
            let context = response_body::Context {
                state: state.clone(), client_request: true, observation_report: false,
                request_id: "request/00000000-0000-0000-0000-000000000000".into(),
                client_snapshot: Some(admitted.clone()), server: None, roster: false,
            };
            let full_query = params(&[("limit", "12")]);
            let (full_snapshot, full) = project_with_context(&state, &admitted, projection,
                &full_query, MAX_RESPONSE_BYTES, "identifier-agents:1", &context).await.unwrap();
            assert_eq!(full["items"].as_array().unwrap().len(), expected.len());
            assert_eq!(full["page"]["has_more"], false);
            let short_budget = serde_json::to_vec(&json!({
                "api_version": CLIENT_API_VERSION, "request_id": context.request_id,
                "snapshot": full_snapshot, "value": full
            })).unwrap().len() - 1;
            for (limit, budget) in [("1", MAX_RESPONSE_BYTES), ("12", short_budget)] {
                let (mut snapshot, mut page) = project_with_context(&state, &admitted, projection,
                    &params(&[("limit", limit)]), budget, "identifier-agents:1", &context).await.unwrap();
                if limit == "12" {
                    assert!(page["items"].as_array().unwrap().len() < expected.len());
                }
                let mut delivered = Vec::new();
                loop {
                    assert!(serde_json::to_vec(&json!({
                        "api_version": CLIENT_API_VERSION, "request_id": context.request_id,
                        "snapshot": snapshot, "value": page
                    })).unwrap().len() <= budget);
                    let items = page["items"].as_array().unwrap();
                    assert!(!items.is_empty(), "empty cursor loop: {projection}; published={published}");
                    delivered.extend(items.iter().map(|item| item["id"].as_str().unwrap().to_owned()));
                    assert!(delivered.len() <= expected.len(), "duplicate cursor loop");
                    let Some(cursor) = page["page"]["next_cursor"].as_str() else {
                        assert_eq!(page["page"]["has_more"], false);
                        break;
                    };
                    assert_eq!(page["page"]["has_more"], true);
                    let decoded = decode_client_cursor(cursor).unwrap();
                    assert_eq!(decoded.offset, delivered.len());
                    let encoded = serde_json::to_string(&decoded).unwrap();
                    for id in &rejected { assert!(!encoded.contains(id)); }
                    (snapshot, page) = project_with_context(&state, &snapshot, projection,
                        &params(&[("cursor", cursor)]), budget, "identifier-agents:1", &context)
                        .await.unwrap();
                }
                assert_eq!(delivered, expected, "{projection}; published={published}; limit={limit}");
            }
        }
    }
}

#[tokio::test]
async fn agent_identifier_membership_all_filtered_is_a_terminal_empty_first_page() {
    for published in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let (_, rejected) = seed_identifier_agents(&state, published, false);
        for projection in ["agents", "agents-rich"] {
            let page = read(&state, projection, &[("limit", "1")], MAX_RESPONSE_BYTES).await.unwrap();
            assert!(page["items"].as_array().unwrap().is_empty());
            assert_eq!(page["page"]["has_more"], false);
            assert!(page["page"]["next_cursor"].is_null());
            for id in &rejected { assert!(!page.to_string().contains(id)); }
        }
    }
}
