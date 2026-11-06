//! Process-local fixture tools. The tools that need only the store live in fractal-store and are
//! re-exported here; the API fixtures live in this file.

pub use fractal_reconcile::test_support::*;

/// Runs this process's client-facing clock `skew_ms` ahead of the wall clock (0 restores it), so a
/// test can pass a real freshness horizon without sleeping. It is process-wide: use it only in a
/// test that nextest runs in its own process, and restore the clock before asserting on new reports.
#[cfg(feature = "test-support")]
pub fn skew_client_clock(skew_ms: u64) {
    crate::api::set_client_clock_skew_ms(skew_ms);
}

/// A message sent through `store`: a claim that node writes, and one no published list reads.
pub fn note_local_claim(store: &crate::store::Store, subject: &str) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(serde_json::json!({"from": "person/example", "to": "agent/example",
                "status": "sent", "content": "a write someone waits to see"}))
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// The fleet [`fleet_member`] binds every member to.
pub const PAUSE_FLEET: &str = "6c5c2b9f-8d4e-4f3a-8b2c-1d7e6f5a4b3c";

/// One member of a fleet, at `dir`, as a peer that [`replicate`] reaches.
pub fn fleet_member(dir: &std::path::Path, name: &str) -> std::sync::Arc<crate::store::Store> {
    std::fs::create_dir_all(dir).unwrap();
    let store = crate::store::Store::open(&dir.join("graph.db"), name).unwrap();
    store.bind_fleet(PAUSE_FLEET).unwrap();
    std::sync::Arc::new(store)
}

/// Everything `from` holds, admitted by `to` the way a peer exchange does it.
pub fn replicate(from: &crate::store::Store, to: &crate::store::Store) {
    let exchange = from.export_replication_exchange(PAUSE_FLEET, &to.replication_inventory().unwrap()).unwrap();
    to.receive_replication_exchange(from.origin(), PAUSE_FLEET, &exchange).unwrap();
    to.validate_replication_backlog().unwrap();
    to.apply_replication_repairs().unwrap();
    assert!(to.project_replication_backlog().unwrap());
}
