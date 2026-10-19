#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, expire_work_lease_by_claim, failed_takeover_run, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, late_claim_matches_a_replay, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, replicated_run_tree, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;
use crate::store::tests_shared::*;
use crate::store::tests_shared_seats::*;
use crate::store::tests_shared_attention_tests::*;

pub const SEAT: &str = "agent/garden/ash";

pub fn queue_run(store: &Store, key: &str) -> String {
    store
        .create_mission_run(&MissionRunRequest {
            mission: "garden/tend".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap()
        .id
}

pub fn queue_move(store: &Store, run: &str, placement: &str, anchor: Option<&str>, key: &str) {
    store
        .move_seat_queue_run(&SeatQueueMoveRequest {
            agent: SEAT.into(),
            run: run.into(),
            placement: placement.into(),
            anchor: anchor.map(str::to_owned),
            reason: None,
            actor: "person/operator".into(),
            idempotency_key: key.into(),
        })
        .unwrap();
}

pub fn queue_snapshot(store: &Store) -> Value {
    json!({
        "one": store.seat_run_order(SEAT).unwrap(),
        "every": store.seat_run_orders().unwrap(),
    })
}
