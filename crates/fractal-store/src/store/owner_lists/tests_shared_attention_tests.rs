#![allow(unused_imports)]
use crate::store::owner_lists::*;
use crate::model::ReplicationInventory;
use crate::store::tests_shared::{exchange_from, receive_and_project};

pub const PEOPLE: &[&str] = &["person/ada", "person/robin", "person/visitor"];

pub const ADA_LAYOUT: &str = "arrangement/person/ada/019a0000-0000-7000-8000-000000000001";

pub const ROBIN_LAYOUT: &str = "arrangement/person/robin/019a0000-0000-7000-8000-000000000002";

pub fn glass(person: &str, id: usize, name: &str) -> ClaimInput {
    ClaimInput {
        subject: format!("glass/{person}/019a0000-0000-7000-8000-{id:012x}"),
        kind: "glass.upserted".into(),
        actor: Some(person.into()),
        fields: serde_json::from_value(json!({
            "body":{"name":name,"layout":{"tabs":[{"pane":"opaque:anything"}]}},
            "base_revision":null,
        }))
        .unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

pub fn arrangement(subject: &str, owner: &str, operations: Value) -> ClaimInput {
    ClaimInput {
        subject: subject.into(),
        kind: "arrangement.edited".into(),
        actor: Some(owner.into()),
        fields: serde_json::from_value(json!({"owner":owner,"operations":operations})).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

pub fn sync(source: &Store, target: &Store) {
    receive_and_project(
        target,
        &source.origin,
        &exchange_from(source, &ReplicationInventory::default()),
    );
}
