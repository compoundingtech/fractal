use super::*;
use super::runtime::Plain;

fn empty() -> Store {
    let store = Store::open_memory("node", Arc::new(Plain)).unwrap();
    assert!(store.project_replication_backlog().unwrap());
    store
}

#[test]
fn empty_attempt_restores_only_its_own_clear_token() {
    let store = empty();
    let before = store.replication_projection_observation();
    for _ in 0..3 {
        assert!(store.project_replication_backlog_before_clear(|| {
            let during = store.replication_projection_observation();
            assert!(during.1);
            assert_ne!(during.0, before.0);
        }).unwrap());
        assert_eq!(store.replication_projection_observation(), before);
    }
}

#[test]
fn saturated_nonce_and_wrapping_entry_never_restore() {
    let store = empty();
    store.replication_projection_deferrals.store(u64::MAX - 1, Ordering::Release);
    let before = store.replication_projection_observation();
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(store.replication_projection_deferrals.load(Ordering::Acquire), u64::MAX);
    assert_ne!(store.replication_projection_observation().0, before.0);
    let before = store.replication_projection_observation();
    assert!(store.project_replication_backlog().unwrap());
    assert_ne!(store.replication_projection_observation().0, before.0);
    let store = empty();
    store.replication_projection_state.store(u64::MAX - 1, Ordering::Release);
    assert!(store.project_replication_backlog().unwrap());
    assert_eq!(store.replication_projection_observation(), (0, false));
}

#[test]
fn prior_pending_and_unhealthy_empty_attempt_never_restore() {
    let store = empty();
    store.defer_replication_projection();
    let before = store.replication_projection_observation();
    assert!(before.1);
    assert!(store.project_replication_backlog().unwrap());
    assert_ne!(store.replication_projection_observation().0, before.0);
    store.connection.write().execute("UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap();
    let before = store.replication_projection_observation();
    assert!(store.project_replication_backlog().unwrap());
    assert_ne!(store.replication_projection_observation().0, before.0);
}

#[test]
fn newer_deferral_from_second_thread_prevents_empty_restore() {
    let store = empty();
    assert!(store.project_replication_backlog_before_clear(|| {
        std::thread::scope(|scope| { scope.spawn(|| { store.defer_replication_projection(); }).join().unwrap(); });
    }).unwrap());
    assert!(store.replication_projection_observation().1);
}
