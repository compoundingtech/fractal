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

#[tokio::test(start_paused = true)]
async fn occurrence_fires_at_deadline_with_stale_peer_only_envelopes() {
    // Keep UTC reads fixed while Tokio advances, so the timer's exact delay and the
    // peer comparison ages do not depend on how long this test takes to execute.
    let _clock = smallclaims::store::clock_snapshot();
    let now = now_ms();
    let due = now + 60_000;
    let at = chrono::DateTime::from_timestamp_millis(due as i64).unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let store = Arc::new(Store::open_memory("node").unwrap());
    super::tests::apply_source(&store, &format!(r#"version 2
schedule "orchard" {{
 at "{at}"
 work {{ mission "orchard"; workspace "/workspace/orchard" }}
}}"#), "timed");
    let schedule = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.kind == "schedule").unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(super::tests::FakeRuntime::default()),
        "node".into(), Arc::new(Notify::new()))
        .with_schedule_peers(vec!["peer".into(), "awake".into()]);
    measure(&store, "awake", 0, now);
    measure(&store, "peer", 7, now - 60_000);

    reconciler.reconcile_schedule(&schedule).unwrap();
    assert_eq!(store.claims_for(&schedule.subject, Some("schedule.occurrence-scheduled")).unwrap().len(), 1,
        "a stale gap must not veto arming the occurrence");
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(59_999)).await;
    tokio::task::yield_now().await;
    assert!(store.claims_for(&schedule.subject, Some("schedule.occurrence-reached")).unwrap().is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let reached = store.claims_for(&schedule.subject, Some("schedule.occurrence-reached")).unwrap();
    assert_eq!(reached.len(), 1, "the due callback must also admit the stale gap, without another event");
    assert_eq!(reached[0].body["fields"]["scheduled_at_unix_ms"], due.to_string());
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

#[tokio::test(start_paused = true)]
async fn revised_owner_generation_fires_at_deadline_with_stale_peer_gap() {
    let _clock = smallclaims::store::clock_snapshot();
    let now = now_ms();
    let due = now + 60_000;
    let at = chrono::DateTime::from_timestamp_millis(due as i64).unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let store = Arc::new(Store::open_memory("node").unwrap());
    let publish_parent = |goal: &str, key: &str| {
        super::tests::apply_source(&store, &format!(r#"version 2
mission "orchard" state="ready" {{
 goal "Tend the orchard."
 step "tend" {{ goal "Tend the trees." }}
}}
mission "parent" state="ready" {{
 goal "{goal}"
 step "hold" {{ goal "Keep the parent active." }}
 schedule "orchard" {{
  host "local"; at "{at}"
  work {{ mission "orchard"; workspace "/workspace/orchard" }}
 }}
}}"#), key);
        store.mission_spec("parent", None).unwrap().unwrap()
    };
    let initial = publish_parent("Keep the orchard scheduled.", "initial");
    let parent = store.create_mission_run(&MissionRunRequest {
        mission: initial.id.clone(), revision: Some(initial.revision.clone()),
        workspace: "/workspace/orchard".into(), requester: None, mode: None,
        inputs: BTreeMap::new(), idempotency_key: "parent-run".into(),
    }).unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(super::tests::FakeRuntime::default()),
        "node".into(), Arc::new(Notify::new()))
        .with_schedule_peers(vec!["peer".into(), "awake".into()]);
    measure(&store, "awake", 0, now);
    measure(&store, "peer", 7, now - 60_000);
    reconciler.reconcile_once().unwrap();
    let subject = format!("schedule/{}/orchard", parent.id);
    let replacement = publish_parent("Keep the revised orchard scheduled.", "replacement");
    let revised = store.adopt_mission_revision(&parent.id, &replacement, "person/test",
        "Revise the same owning run.", "revise-parent").unwrap();
    assert_ne!(parent.generation, revised.generation);
    reconciler.reconcile_once().unwrap();
    let schedule = store.desired_subjects_named(std::slice::from_ref(&subject)).unwrap().pop().unwrap();
    assert_eq!(schedule.owner_run.as_deref(), Some(parent.subject.as_str()));
    assert_eq!(schedule.owner_generation.as_deref(), Some(revised.generation.as_str()));
    assert!(!schedule_requires_complete_history(&store, &schedule).unwrap(),
        "a revision of the same owner run is not an ownership change");
    assert!(ready(&store, &schedule, now));
    // Materializing the revised declaration wakes the next reconciliation pass.
    reconciler.reconcile_schedule(&schedule).unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(59_999)).await;
    tokio::task::yield_now().await;
    assert!(store.claims_for(&subject, Some("schedule.occurrence-reached")).unwrap().is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    let reached = store.claims_for(&subject, Some("schedule.occurrence-reached")).unwrap();
    assert_eq!(reached.len(), 1, "the revised schedule fires without a peer exchange");
    assert_eq!(reached[0].body["fields"]["scheduled_at_unix_ms"], due.to_string());
}

#[test]
fn owner_run_and_declaring_step_changes_still_require_complete_history() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "host \"node\";", "initial");
    for (owner_run, owner_step) in [
        (Some("mission-run/replacement".into()), None),
        (None, Some("step-run/replacement".into())),
    ] {
        let changed = DesiredSubject { owner_run, owner_step, ..schedule.clone() };
        assert!(schedule_requires_complete_history(&store, &changed).unwrap());
    }
}

#[tokio::test(start_paused = true)]
async fn moved_primary_with_stale_peer_gap_stays_blocked_and_warns() {
    let _clock = smallclaims::store::clock_snapshot();
    let now = now_ms();
    let store = Arc::new(Store::open_memory("node").unwrap());
    for (host, key) in [("peer", "initial"), ("node", "moved")] {
        super::tests::apply_source(&store, &format!(r#"version 2
schedule "orchard" {{
 host "{host}"; at "2020-01-01T00:00:00Z"
 work {{ mission "orchard"; workspace "/workspace/orchard" }}
}}"#), key);
    }
    let schedule = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.kind == "schedule").unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(super::tests::FakeRuntime::default()),
        "node".into(), Arc::new(Notify::new()))
        .with_schedule_peers(vec!["peer".into(), "awake".into()]);
    measure(&store, "awake", 0, now);
    measure(&store, "peer", 7, now - 60_000);
    assert!(!ready(&store, &schedule, now));
    reconciler.reconcile_schedule(&schedule).unwrap();
    tokio::time::advance(Duration::from_secs(15 * 60)).await;
    reconciler.reconcile_schedule(&schedule).unwrap();
    assert!(store.claims_for(&schedule.subject, Some("schedule.occurrence-reached")).unwrap().is_empty());
    let failures = store.claims_for(&schedule.subject, Some("operational.failure")).unwrap();
    assert_eq!(failures.len(), 1, "a real placement change remains visibly fail-closed");
    assert!(failures[0].body["fields"]["reason"].as_str().unwrap()
        .contains("peer_only_envelopes=7"));
}
