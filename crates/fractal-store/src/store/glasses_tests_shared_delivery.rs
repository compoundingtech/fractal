#![allow(unused_imports)]
use crate::store::*;
use crate::store::tests_shared::{exchange_from, receive_and_project};
use fractal_schema::glasses::MAX_GLASSES;
use crate::store::glasses::*;
use crate::model::ReplicationInventory;

pub fn input(id: usize, name: &str) -> ClaimInput {
        ClaimInput { subject:format!("glass/person/ada/019a0000-0000-7000-8000-{id:012x}"), kind:"glass.upserted".into(), actor:Some("person/ada".into()),
            fields:serde_json::from_value(json!({"body":{"name":name,"layout":{"tabs":[{"pane":"opaque:anything"}]}},"base_revision":null})).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None }
    }

pub fn sync(source: &Store, target: &Store) {
        let exchange = exchange_from(source, &ReplicationInventory::default());
        receive_and_project(target, &source.origin, &exchange);
    }

pub fn history_oracle(store: &Store) -> Vec<Value> {
        let connection = store.readers.get();
        let mut resources: Vec<_> =
            live_claims(glass_claims(&connection, "person/ada", u64::MAX).unwrap())
                .iter()
                .map(|claim| resource(claim).unwrap())
                .collect();
        resources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        resources
    }
