use super::*;
use smallclaims::store::PeerSyncProgress;

fn publish(store: &Store, placement: &str, key: &str) -> DesiredSubject {
    super::tests::apply_source(store, &format!(r#"version 2
schedule "orchard" {{ {placement}
 every "1h"; anchor "2030-01-01T00:00:00Z"
 work {{ mission "orchard"; workspace "/workspace/orchard" }}
}}"#), key);
    store.desired_subjects().unwrap().into_iter().find(|s| s.kind == "schedule").unwrap()
}

fn measure(store: &Store, peer: &str, missing: u64, at: u128) {
    store.replication_sync.lock().unwrap().insert(peer.into(), PeerSyncProgress {
        measured: Some(smallclaims::replication::ReplicationPeerSync {
            peer_only_envelopes: missing, measured_at_unix_ms: at, ..Default::default()
        }), ..Default::default()
    });
}

fn ready(store: &Store, schedule: &DesiredSubject, since: u128) -> bool {
    schedule_admission_ready(store, "node", &["peer".into(), "awake".into()], since, schedule).unwrap()
}

#[test]
fn stale_positive_admits_no_standby_schedule_but_fresh_positive_blocks() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "host \"node\";", "initial");
    let now = now_ms();
    measure(&store, "awake", 0, now);
    measure(&store, "peer", 7, now - 60_000);
    assert!(ready(&store, &schedule, now));
    measure(&store, "peer", 7, now_ms());
    assert!(!ready(&store, &schedule, now));
}

#[test]
fn cold_restart_requires_a_complete_exchange_even_when_positive_count_is_stale() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "", "initial");
    let now = now_ms();
    measure(&store, "peer", 7, now - 60_000);
    measure(&store, "awake", 0, now - 60_000);
    assert!(!ready(&store, &schedule, now));
    measure(&store, "awake", 0, now_ms());
    assert!(ready(&store, &schedule, now));
}

#[test]
fn standby_schedule_keeps_stale_positive_count_blocking_on_primary_too() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "standby { host \"peer\"; after \"1m\" }", "initial");
    let now = now_ms();
    measure(&store, "awake", 0, now);
    measure(&store, "peer", 7, now - 60_000);
    assert!(!ready(&store, &schedule, now));
}

#[test]
fn placement_change_and_removed_standby_keep_conservative_history_policy() {
    for original in ["host \"peer\";", "standby { host \"peer\"; after \"1m\" }"] {
        let store = Store::open_memory("node").unwrap();
        publish(&store, original, "initial");
        let schedule = publish(&store, "host \"node\";", "moved");
        let now = now_ms();
        measure(&store, "awake", 0, now);
        measure(&store, "peer", 7, now - 60_000);
        assert!(!ready(&store, &schedule, now));
        measure(&store, "peer", 0, now_ms());
        assert!(ready(&store, &schedule, now));
    }
}
