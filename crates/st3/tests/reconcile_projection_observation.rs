//! Real producer/Incremental controls. These cache selection probes do not execute runtime stops.
use std::{collections::BTreeSet, sync::Arc};
use serde_json::json;
use st3::{incremental::Incremental, model::ClaimInput, store::{Store, owned_sets::{Options, Source}}};

const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";

fn managed_store() -> Store {
    let store = Store::open_memory("amber").unwrap();
    let input = st3::parse_intent("version 2\nagent \"garden/orchard\" { command \"true\" }\nagent \"garden/meadow\" { command \"true\" }", "amber").unwrap();
    let mut options = Options { set: "garden".into(), source: Source { repository: "acme/garden".into(), r#ref: "refs/heads/main".into(), sha: format!("{:040x}", 1), sequence: 1 }, expected_set: "absent".into(), rollout: None, adopt: Default::default(), allow_empty: false, confirm_retire: None, expected_subjects: Default::default() };
    options.expected_subjects = store.owned_set_preview(&input, &options).unwrap().expected_subjects;
    store.apply_owned_set(&input, &options, "managed", "person/operator").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    store
}

fn cached_stop(store: &Store) -> Incremental {
    let incremental = Incremental::default();
    assert!(incremental.needs("stop:agent/invented-settled", 0));
    incremental.observe(store).unwrap();
    incremental.evaluated("stop:agent/invented-settled", BTreeSet::from(["agent/invented-settled".into()]), None);
    assert!(!incremental.needs("stop:agent/invented-settled", 0));
    incremental
}

#[test]
fn repeated_empty_managed_projection_keeps_cached_stop_clean() {
    let store = managed_store();
    let incremental = cached_stop(&store);
    let before = store.replication_projection_observation();
    let desired = serde_json::to_value(store.desired_subjects().unwrap()).unwrap();
    for _ in 0..3 {
        let writes = store.connection.write().total_changes();
        assert!(store.project_replication_backlog().unwrap());
        let writes_after = store.connection.write().total_changes();
        assert_eq!(writes_after - writes, 1, "only projection_health bookkeeping may write");
        assert_eq!(store.replication_projection_observation(), before);
        incremental.observe(&store).unwrap();
        assert!(!incremental.needs("stop:agent/invented-settled", 0));
        assert_eq!(serde_json::to_value(store.desired_subjects().unwrap()).unwrap(), desired);
        store.owned_member_guard("agent/garden/orchard").unwrap();
    }
}

#[test]
fn zero_count_replay_and_after_projection_write_do_not_restore_token() {
    for replay in [true, false] {
        let store = managed_store();
        let incremental = cached_stop(&store);
        let before = store.replication_projection_observation();
        if replay {
            store.connection.write().execute("UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
        } else {
            store.connection.write().execute("INSERT INTO local_resource_projection_pending(subject) VALUES('resource/invented-empty')", []).unwrap();
        }
        assert!(store.project_replication_backlog().unwrap());
        assert_ne!(store.replication_projection_observation().0, before.0);
        incremental.observe(&store).unwrap();
        assert!(incremental.needs("stop:agent/invented-settled", 0));
    }
}

fn resource(source: &Store, name: &str) {
    source.append_claim(&ClaimInput { subject: format!("resource/{name}"), kind: "resource.observed".into(), actor: None, fields: std::collections::BTreeMap::from([("kind".into(), json!("custom.st3.test"))]), evidence: vec![], expected_subject: None, idempotency_key: Some(name.into()) }).unwrap();
}
fn admit(source: &Store, target: &Store) {
    let inventory = target.replication_inventory().unwrap();
    let exchange = source.export_replication_exchange(FLEET, &inventory).unwrap();
    assert!(!exchange.envelopes.is_empty());
    assert!(target.receive_replication_exchange("source", FLEET, &exchange).unwrap().received > 0);
    assert!(target.validate_replication_backlog().unwrap().changed);
}

#[test]
fn second_writer_admission_between_snapshot_and_restore_stays_pending() {
    let source = Store::open_memory("source").unwrap(); source.bind_fleet(FLEET).unwrap();
    let target = Arc::new(Store::open_memory("target").unwrap()); target.bind_fleet(FLEET).unwrap();
    assert!(target.project_replication_backlog().unwrap());
    resource(&source, "foreign");
    let before = target.replication_projection_observation();
    assert!(target.project_replication_backlog_before_clear(|| {
        std::thread::scope(|scope| { scope.spawn(|| admit(&source, &target)).join().unwrap(); });
    }).unwrap());
    let after = target.replication_projection_observation();
    assert_ne!(after.0, before.0);
    assert!(after.1, "new admission must remain pending after the empty attempt");
    let incremental = cached_stop(&target);
    incremental.observe(&target).unwrap();
    assert!(incremental.needs("stop:agent/invented-settled", 0));
    assert!(target.project_replication_backlog().unwrap());
    assert!(!target.replication_projection_observation().1);
    incremental.observe(&target).unwrap();
    assert!(incremental.needs("stop:agent/invented-settled", 0));
}

#[test]
fn failed_feed_and_fresh_consumer_remain_conservative() {
    let store = managed_store();
    let incremental = cached_stop(&store);
    store.connection.write().execute("ALTER TABLE local_observations RENAME TO unavailable_local", []).unwrap();
    assert!(incremental.observe(&store).is_err());
    assert!(incremental.needs("stop:agent/invented-settled", 0));
    store.connection.write().execute("ALTER TABLE unavailable_local RENAME TO local_observations", []).unwrap();
    let fresh = Incremental::default(); fresh.observe(&store).unwrap();
    assert!(fresh.needs("stop:agent/invented-settled", 0));
}
