#![allow(unused_imports)]
use crate::store::arrangements::*;
use crate::store::*;

pub const SUBJECT: &str = "arrangement/person/ada/019a0000-0000-7000-8000-000000000001";

pub const A: &str = "019a0000-0000-7000-8000-000000000010";

pub const B: &str = "019a0000-0000-7000-8000-000000000020";

pub fn input(operations: Value) -> ClaimInput {
    ClaimInput { subject: SUBJECT.into(), kind: "arrangement.edited".into(), actor: Some("person/ada".into()),
        fields: serde_json::from_value(json!({"owner":"person/ada","operations":operations})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None }
}

pub fn append(store: &Store, operations: Value) -> ClaimRecord { store.append_client_claim(&input(operations)).unwrap() }

pub fn create(store: &Store) {
    append(store,json!([{"op":"create","name":"Work"},{"op":"folder.create","id":A,"name":"A","parent":null,"key":"a0"},
        {"op":"folder.create","id":B,"name":"B","parent":null,"key":"a1"}]));
}
