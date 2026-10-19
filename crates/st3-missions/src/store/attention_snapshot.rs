//! The part of `store::attention_snapshot` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::LoopSpec;
pub use st3_store::store::attention_snapshot::*;
pub use st3_store::store::attention_snapshot::{CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, agent, conversation_agent, native_prompt_gone_key, person_attention_item, plan_approval, resolve_native_harness_at, screen_prompt_delivered_key, screen_prompt_gone_key, screen_prompt_undelivered_key};

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_snapshot_tests_shared_seats::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_snapshot_tests_shared_seats::{parity, step_back_in_time};
    use super::*;
    use crate::model::ClaimInput;

    /// Registered custom attention is admitted with its request, so an active custom card
    /// leaves no future change, and only its recipient ever sees it.
    #[test]
    fn registered_custom_attention_has_no_future_change_once_active() {
        let store = Store::open_memory("garden").unwrap();
        let manifest: st3_schema::custom::Manifest = serde_json::from_str(include_str!(
            "../../../../examples/st3/custom-review.json"
        ))
        .unwrap();
        store
            .register_custom_kind(&custom::RegistrationRequest {
                manifest,
                actor: "agent/garden/seed".into(),
            })
            .unwrap();
        store
            .append_claim(&ClaimInput {
                subject: "custom/garden/review/v1/fence".into(),
                kind: "custom.garden.review.v1.requested".into(),
                actor: Some("agent/garden/seed".into()),
                fields: serde_json::from_value(json!({
                    "title": "Retain the seed history?",
                    "detail": "Choose Keep or Discard.",
                    "recipient": "person/lichen"
                }))
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert!(
            store
                .attention_snapshot(Some("person/lichen"), now_ms())
                .unwrap()
                .iter()
                .any(|item| item.subject == "custom/garden/review/v1/fence")
        );
        assert_eq!(
            store
                .attention_collection_snapshot(Some("person/lichen"), now_ms())
                .unwrap()
                .1,
            None
        );
        assert_eq!(
            store.attention_collection_snapshot(None, now_ms()).unwrap().1,
            None
        );
        parity(&store, Some("person/lichen"), now_ms());
        parity(&store, None, now_ms());
        parity(&store, Some("person/avery"), now_ms());
        assert!(store
            .attention_snapshot(Some("person/avery"), now_ms())
            .unwrap()
            .is_empty());
    }
}
