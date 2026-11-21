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

fn traffic_measurement(store: &Store, peer: &str, missing: u64, at: u128) {
    let healthy = store.replication_admission_healthy().unwrap();
    let basis = smallclaims::store::InventoryBasis {
        local_inventory_generation: store.replication_snapshot().unwrap().inventory_generation,
        ..Default::default()
    };
    let mut sync = store.replication_sync.lock().unwrap();
    let progress = sync.entry(peer.into()).or_default();
    progress.observe_on(5, Some((missing, missing + 7)), basis, at);
    progress.observe_schedule_progress(healthy, at);
}

#[test]
fn continuous_traffic_needs_three_distinct_measurements_since_boundary() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "host \"node\";", "initial");
    let since = now_ms();
    for (offset, missing) in [(0, 16), (1, 59)] {
        traffic_measurement(&store, "peer", missing, since + offset);
        assert!(!ready(&store, &schedule, since));
    }
    for _ in 0..5 {
        traffic_measurement(&store, "peer", 23, since + 1);
        assert!(!ready(&store, &schedule, since), "one measurement is not a streak");
    }
    traffic_measurement(&store, "peer", 23, since + 2);
    assert!(ready(&store, &schedule, since));
    assert!(!ready(&store, &schedule, since + 1), "old samples cannot release a new boundary");
}

#[test]
fn replacement_membership_relayed_before_exchange_cannot_reuse_healthy_streak() {
    use crate::schedule_standby::tests::{FLEET, pair, sync};
    let _clock = smallclaims::store::clock_snapshot();
    let (relay, old_peer, _) = pair();
    let store = Store::open_memory("node").unwrap();
    let node_key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(&relay.member_public_key().unwrap()).unwrap();
    store.set_member_key(Some(node_key.clone())).unwrap();
    let membership_claim = |subject: &str, kind: &str, fields: Value| {
        relay.append_claim(&crate::model::ClaimInput {
            subject: subject.into(), kind: kind.into(), actor: None,
            fields: serde_json::from_value(fields).unwrap(), evidence: vec![],
            expected_subject: None, idempotency_key: None,
        }).unwrap();
    };
    membership_claim("host/node", "fleet.member-admitted", serde_json::json!({
        "fleet_id": FLEET, "member_key": node_key.public(), "via": "invite",
        "sponsor": "host/ivory", "mode": "listening"
    }));
    sync(&relay, &store);
    let schedule = publish(&store, "host \"node\";", "initial");
    let since = now_ms();
    for offset in 0..3 { traffic_measurement(&store, "jade", 20, since + offset); }
    let old_key = old_peer.member_public_key().unwrap();
    store.replication_sync.lock().unwrap().get_mut("jade").unwrap()
        .measured_member_key = Some(old_key.clone());
    let admitted = || schedule_admission_ready(&store, "node", &["jade".into()], since, &schedule).unwrap();
    assert!(admitted(), "the original authenticated incarnation has a healthy streak");

    let replacement = crate::fleet::MemberKey::generate().unwrap().0;
    membership_claim("host/jade", "fleet.member-removed", serde_json::json!({
        "member_key": old_key, "high_water": 100, "reason": "replacement",
        "removed_by": "person/test"
    }));
    membership_claim("host/jade", "fleet.member-admitted", serde_json::json!({
        "fleet_id": FLEET, "member_key": replacement.public(), "via": "invite",
        "sponsor": "host/ivory", "mode": "listening", "writer_floor": 100
    }));
    // Only the relay exchanges with this observer. The replacement never contacts it.
    sync(&relay, &store);
    assert!(matches!(store.fleet_membership().unwrap().state("jade"),
        crate::fleet::MemberState::Current(member) if member.member_key == replacement.public()));
    assert!(!admitted(), "a fresh old-incarnation measurement cannot authorize its replacement");
    assert!(!store.replication_sync.lock().unwrap()["jade"].schedule_progress_since(since),
        "discard old progress immediately, rather than waiting for a direct exchange");
}

#[test]
fn admission_discards_a_streak_measured_against_a_different_inventory_basis() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "host \"node\";", "initial");
    let since = now_ms();
    for offset in 0..3 { traffic_measurement(&store, "peer", 20, since + offset); }
    assert!(ready(&store, &schedule, since));
    store.replication_sync.lock().unwrap().get_mut("peer").unwrap()
        .window_basis.local_inventory_generation += 1;
    assert!(!ready(&store, &schedule, since));
    assert!(!store.replication_sync.lock().unwrap()["peer"].schedule_progress_since(since));
}

#[test]
fn invalid_and_fenced_state_hold_healthy_traffic_and_reset_streak() {
    for sql in [
        /* sqlite-large-value: allow-for-now lv-he6c21184bcd0 owner=fractal_reconcile::reconcile::schedule_admission_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,updated_at_unix_ms)
         VALUES('bad','peer',1,'hash',0,X'00','invalid','0')",
        /* sqlite-large-value: allow-for-now lv-h4786749395c7 owner=fractal_reconcile::reconcile::schedule_admission_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelope_holds VALUES('peer',1,'hash','fenced','0')",
    ] {
        let store = Store::open_memory("node").unwrap();
        let schedule = publish(&store, "host \"node\";", "initial");
        let since = now_ms();
        for offset in 0..3 { traffic_measurement(&store, "peer", 20, since + offset); }
        assert!(ready(&store, &schedule, since));
        store.connection.lock().unwrap().execute(/* sqlite-large-value: allow-for-now lv-h5c8ae9829e96 owner=fractal_reconcile::reconcile::schedule_admission_tests TODO=trace-statement-and-value-representation */ sql, []).unwrap();
        assert!(!ready(&store, &schedule, since), "current unhealthy state vetoes an old streak");
        traffic_measurement(&store, "peer", 20, since + 3);
        store.connection.lock().unwrap().execute_batch(
            /* sqlite-large-value: allow-for-now lv-hb1c9f0d22451 owner=fractal_reconcile::reconcile::schedule_admission_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_records WHERE record_ref='bad'; DELETE FROM replica_envelope_holds;"
        ).unwrap();
        for offset in 4..6 {
            traffic_measurement(&store, "peer", 20, since + offset);
            assert!(!ready(&store, &schedule, since));
        }
        traffic_measurement(&store, "peer", 20, since + 6);
        assert!(ready(&store, &schedule, since));
    }
}

#[test]
fn moved_host_and_standby_never_use_traffic_fallback() {
    for original in ["host \"peer\";", "standby { host \"peer\"; after \"1m\" }"] {
        let store = Store::open_memory("node").unwrap();
        publish(&store, original, "initial");
        let schedule = publish(&store, "host \"node\";", "moved");
        let since = now_ms();
        measure(&store, "awake", 0, since);
        for offset in 0..3 { traffic_measurement(&store, "peer", 20, since + offset); }
        assert!(!ready(&store, &schedule, since), "fresh in-flight traffic cannot release moved ownership");
        measure(&store, "peer", 0, since + 3);
        assert!(ready(&store, &schedule, since));
    }
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "standby { host \"peer\"; after \"1m\" }", "initial");
    let since = now_ms();
    measure(&store, "awake", 0, since);
    for offset in 0..3 { traffic_measurement(&store, "peer", 20, since + offset); }
    assert!(!ready(&store, &schedule, since));
}

#[test]
fn stale_strict_peer_holds_even_with_a_healthy_streak_and_fresh_other_peer() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "standby { host \"peer\"; after \"1m\" }", "initial");
    let since = now_ms() - 60_000;
    for offset in 0..3 { traffic_measurement(&store, "peer", 20, since + offset); }
    measure(&store, "awake", 0, now_ms());
    assert!(!ready(&store, &schedule, since));
}

#[test]
fn strict_schedule_requires_a_measurement_from_each_peer() {
    let store = Store::open_memory("node").unwrap();
    let schedule = publish(&store, "standby { host \"peer\"; after \"1m\" }", "initial");
    let since = now_ms();
    measure(&store, "awake", 0, since);
    assert!(!ready(&store, &schedule, since));
    measure(&store, "peer", 0, since);
    assert!(ready(&store, &schedule, since));
}

#[test]
fn quiet_gap_unhealthy_exchange_and_clock_regression_break_progress_streak() {
    for (healthy, next) in [(false, 103), (true, 60_103), (true, 99)] {
        let mut progress = PeerSyncProgress::default();
        for at in 100..103 { progress.observe_schedule_progress(true, at); }
        assert!(progress.schedule_progress_since(100));
        progress.observe_schedule_progress(healthy, next);
        assert!(!progress.schedule_progress_since(100));
    }
}

#[tokio::test(start_paused = true)]
async fn continuous_nonzero_traffic_fires_due_occurrence() {
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
}}"#), "timed-traffic");
    let schedule = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.kind == "schedule").unwrap();
    let reconciler = Reconciler::new(store.clone(), Arc::new(super::tests::FakeRuntime::default()),
        "node".into(), Arc::new(Notify::new())).with_schedule_peers(vec!["peer".into()]);
    for (offset, missing) in [(0, 16), (1, 59), (2, 23)] {
        traffic_measurement(&store, "peer", missing, now + offset);
    }
    reconciler.reconcile_schedule(&schedule).unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(60_000)).await;
    tokio::task::yield_now().await;
    assert_eq!(store.claims_for(&schedule.subject, Some("schedule.occurrence-reached")).unwrap().len(), 1);
    assert!(store.replication_peer_sync(&["peer".into()])["peer"].peer_only_envelopes > 0);
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
    // The timer contract concerns explicit same-run adoption, not S3 publication.
    let replacement = publish_parent("Keep the revised orchard scheduled.", "replacement");
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
