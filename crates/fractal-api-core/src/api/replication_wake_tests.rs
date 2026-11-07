//! A replication receive that wakes the reconciler is counted under the claim kind it brought,
//! as an API write is, so the wake mix names what other hosts' changes cost this one.
use super::*;
use std::collections::BTreeMap;

const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";

fn wakes(cause: &str) -> u64 {
    crate::performance::snapshot()["reconciler_wakes"].as_array().unwrap().iter()
        .find(|row| row["cause"] == cause).map_or(0, |row| row["count"].as_u64().unwrap())
}

#[tokio::test]
async fn a_waking_replication_receive_is_counted_under_the_kind_it_brought() {
    let root = tempfile::tempdir().unwrap();
    let state = tests_shared::state(root.path());
    state.store.bind_fleet(FLEET).unwrap();
    state.store.project_replication_backlog().unwrap();
    let source = Store::open_memory("source").unwrap();
    source.append_client_claim(&smallclaims::ClaimInput {
        subject: "resource/receive-wake".into(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.test.replication"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    source.bind_fleet(FLEET).unwrap();
    let inventory = state.store.export_replication_summary(FLEET).unwrap().inventory;
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    let cause = "replication receive · resource.observed";
    let before = wakes(cause);

    let Json(response) = replication::replication_receive(State(state.clone()), Json(ReplicationReceiveRequest {
        peer: "source".into(), fleet_id: FLEET.into(), exchange, round_trip_ms: None, carrier: Default::default(),
    })).await.unwrap();

    assert!(response.changed);
    assert_eq!(wakes(cause) - before, 1, "the receive's wake was not counted under resource.observed");
    tokio::time::timeout(std::time::Duration::from_secs(5), state.notify.notified()).await
        .expect("a receive that brings a graph change must wake the reconciler");
}
