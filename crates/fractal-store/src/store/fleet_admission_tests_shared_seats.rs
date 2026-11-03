#![allow(unused_imports)]
use crate::store::*;
use crate::fleet::{MemberKey, MemberState, envelope_signature_message, verify_signature};

pub const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

pub fn key() -> Arc<MemberKey> {
    Arc::new(MemberKey::generate().unwrap().0)
}

pub fn node(name: &str, key: Option<&Arc<MemberKey>>, anchor: Option<&Arc<MemberKey>>) -> Store {
    let store = Store::open_memory(name).unwrap();
    store.bind_fleet(FLEET).unwrap();
    if let Some(anchor) = anchor {
        store.pin_fleet_anchor(anchor.public()).unwrap();
    }
    if let Some(key) = key {
        store.set_member_key(Some(key.clone())).unwrap();
    }
    store
}

pub fn append(store: &Store, kind: &str, subject: &str, fields: Value) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

/// A plain replicated claim: a diagnostic as an older build wrote it, before it became local.
pub fn note(store: &Store, text: &str) -> ClaimRecord {
    store
        .append_as_older_build_for_test(&ClaimInput {
            subject: format!("daemon/{}", store.origin),
            kind: "daemon.diagnostic".into(),
            actor: None,
            fields: serde_json::from_value(json!({"severity": "warning", "code": "fleet-test", "reason": text})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

pub fn admit(by: &Store, name: &str, member: &MemberKey, via: &str, floor: Option<u64>) {
    let mut fields = json!({
        "fleet_id": FLEET,
        "member_key": member.public(),
        "via": via,
        "mode": "listening",
    });
    if via != "anchor" {
        fields["sponsor"] = json!(format!("host/{}", by.origin));
    }
    if let Some(floor) = floor {
        fields["writer_floor"] = json!(floor);
    }
    append(by, "fleet.member-admitted", &format!("host/{name}"), fields);
}

/// One exchange from `from` to `to`, then admission and projection on `to`.
pub fn sync(from: &Store, to: &Store) -> ReplicationAdmission {
    let exchange = from
        .export_replication_exchange_answering(
            FLEET,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        )
        .unwrap();
    to.receive_replication_exchange(&from.origin, FLEET, &exchange)
        .unwrap();
    let admission = to.validate_replication_backlog().unwrap();
    to.project_replication_backlog().unwrap();
    admission
}

pub fn admitted(store: &Store, claim: &ClaimRecord) -> bool {
    store.claim_by_id(&claim.id).unwrap().is_some()
}
