#![allow(unused_imports)]
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;
use crate::model::ReplicationInventory;

pub const TEST_FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";

pub fn receive_and_project(
    target: &Store,
    relay: &str,
    exchange: &ReplicationExchange,
) -> ReplicationAdmission {
    target.bind_fleet(TEST_FLEET).expect("target fleet");
    target
        .receive_replication_exchange(relay, TEST_FLEET, exchange)
        .expect("replication receipt");
    let admission = target
        .validate_replication_backlog()
        .expect("replication admission");
    target
        .apply_replication_repairs()
        .expect("replication repairs");
    assert!(
        target
            .project_replication_backlog()
            .expect("replication projection")
    );
    admission
}

pub fn exchange_from(
    source: &Store,
    remote: &ReplicationInventory,
) -> ReplicationExchange {
    source.bind_fleet(TEST_FLEET).expect("source fleet");
    source
        .export_replication_exchange(TEST_FLEET, remote)
        .expect("replication exchange")
}
