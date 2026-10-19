#![allow(unused_imports)]
use super::*;
use std::time::Duration;
use st3_store::store::writer_bound_tests_shared_missions::*;
use st3_store::store::writer_bound_tests_shared_missions::{BOUND, stage};


/// A late claim about one request under a standing root with 200 child runs rebuilds that
/// request's part of the tree, not all 200 runs in one hold.
#[test]
fn a_late_claim_in_a_large_standing_tree_holds_the_writer_for_its_own_part() {
    let (controller, worker, _root, children) = crate::store::tests_shared_seats::replicated_run_tree(200);
    let earlier = worker
        .claims_for(&children[0].subject, None)
        .unwrap()
        .iter()
        .map(|claim| claim.accepted_at_unix_ms)
        .min()
        .unwrap();
    worker
        .set_step_state(&children[199].steps[0].subject, "blocked", Some("waits on a person"))
        .unwrap();
    controller.set_write_clock_at(earlier).unwrap();
    controller
        .set_step_state(&children[100].steps[0].subject, "blocked", Some("blocked early"))
        .unwrap();
    let inventory = worker.replication_inventory().unwrap();
    let (exchange, _) = stage("bound/standing-tree/export", || {
        crate::store::tests_shared::exchange_from(&controller, &inventory)
    });
    stage("bound/standing-tree/receive", || {
        worker
            .receive_replication_exchange("controller", &exchange.fleet_id, &exchange)
            .unwrap()
    });
    stage("bound/standing-tree/admit", || worker.validate_replication_backlog().unwrap());
    stage("bound/standing-tree/repairs", || worker.apply_replication_repairs().unwrap());
    FULL_REPLAYS.with(|replays| replays.set(0));
    let (projected, held) = stage("bound/standing-tree/project", || {
        worker.project_replication_backlog().unwrap()
    });
    assert!(projected);
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "the late claim replayed the store");
    assert!(held > Duration::ZERO, "the projection asked for no writer hold");
}
