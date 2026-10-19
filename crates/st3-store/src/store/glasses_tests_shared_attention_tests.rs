#![allow(unused_imports)]
use crate::store::*;
use st3_schema::glasses::MAX_GLASSES;
use crate::store::glasses::*;
use crate::model::ReplicationInventory;

pub fn input(id: usize, name: &str) -> ClaimInput {
        ClaimInput { subject:format!("glass/person/ada/019a0000-0000-7000-8000-{id:012x}"), kind:"glass.upserted".into(), actor:Some("person/ada".into()),
            fields:serde_json::from_value(json!({"body":{"name":name,"layout":{"tabs":[{"pane":"opaque:anything"}]}},"base_revision":null})).unwrap(), evidence:vec![], expected_subject:None, idempotency_key:None }
    }
