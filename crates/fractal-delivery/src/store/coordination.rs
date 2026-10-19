//! The part of `store::coordination` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::DeliveryStore;
use super::*;
pub use fractal_store::store::coordination::*;
pub use fractal_store::store::coordination::{backfill, cover_synced_sends, create_schema, initialize, sync};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_markers_from_raw_claims_never_enter_message_views() {
        let store = Store::open_memory("node").unwrap();
        let marker = format!("{}{}", crate::silent::REMAINING_PREFIX, usize::MAX);
        store
            .append_claim(&ClaimInput {
                subject: "message/raw-marker".into(),
                kind: "message.sent".into(),
                actor: Some("person/example".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("sent")),
                    ("from".into(), json!("person/example")),
                    ("to".into(), json!("agent/example/reader")),
                    ("content".into(), json!("raw")),
                    ("tags".into(), json!([marker, "dictated"])),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert_eq!(
            store.message("message/raw-marker").unwrap().unwrap().tags,
            ["dictated"]
        );
        assert_eq!(
            store
                .messages_for_delivery_through("agent/example/reader", store.index().unwrap())
                .unwrap()[0]
                .tags,
            ["dictated"]
        );
        assert_eq!(
            store
                .latest_claim("message/raw-marker", Some("message.sent"))
                .unwrap()
                .unwrap()
                .body["fields"]["tags"][0],
            marker
        );
    }
}
