#![allow(unused_imports)]
use crate::store::directive_notes::*;
use crate::fleet::MemberKey;
use crate::model::PersonStepResponse;
use rusqlite::StatementStatus;

pub const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

pub fn claim(store: &Store, subject: &str, kind: &str, actor: Option<&str>, fields: Value) -> ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: actor.map(str::to_owned),
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap()
}

pub fn advertise(store: &Store, supported: bool) {
    claim(store, &format!("daemon/{}", store.origin), "daemon.started", None,
        json!({"status":"running", "features":{"person_directive_note": if supported { 1 } else { 0 }}}));
}

pub fn enable(store: &Store) -> Arc<MemberKey> {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    advertise(store, true);
    key
}

pub fn store() -> Store {
    let store = Store::open_memory("alder").unwrap();
    enable(&store);
    store
}

pub fn declare(store: &Store, source: &str, actor: &str, key: &str) -> NormalizedIntent {
    let intent = crate::graph::parse_internal_intent(source, &store.origin).unwrap();
    let planned = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply_as(&intent, &planned.subject_tokens, key, Some(actor)).unwrap();
    intent
}
