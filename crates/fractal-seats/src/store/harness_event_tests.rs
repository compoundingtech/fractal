#![allow(unused_imports)]
use super::*;
use crate::harness_events::Publication;
use fractal_missions::store::harness_event_tests_shared_seats::*;
use fractal_missions::store::harness_event_tests_shared_seats::{SEAT, event, runtime};
use crate::store::limits::{AccountLimit, LIMITS_REFRESH_MS, LimitsStore};


#[test]
fn harness_todo_event_replay_and_runtime_fencing_preserve_latest_snapshot() {
    let store = Store::open_memory("amber").unwrap();
    runtime(&store, "runtime-a", "running");
    let mut input = event("harness.observed");
    input.claim.kind = "harness.todo.observed".into();
    input.claim.fields = serde_json::from_value(json!({
        "harness":"omp","session_id":"native-a","incarnation_id":"runtime-a",
        "observed_at":"2026-10-03T15:00:00Z","source_op":"block",
        "phases":[{"name":"Review","tasks":[
            {"content":"Review claim","status":"blocked","blocker":"Await review"}
        ]}],"totals":{"pending":0,"in_progress":0,"completed":0,"blocked":1},
        "truncated":false
    })).unwrap();
    let (first, changed) = store.append_harness_event(&input).unwrap();
    assert!(changed);
    let (replay, changed) = store.append_harness_event(&input).unwrap();
    assert!(!changed);
    assert_eq!(replay.body["fields"], first.body["fields"]);
    let original = input.claim.fields.clone();
    input.claim.fields.insert("source_op".into(), json!("hydrate"));
    assert_eq!(store.append_harness_event(&input).unwrap_err().code, "idempotency-mismatch");
    input.claim.fields = original;
    input.sequence = 2;
    input.claim.fields.insert("phases".into(), json!([]));
    input.claim.fields.insert("totals".into(), json!({
        "pending":0,"in_progress":0,"completed":0,"blocked":0
    }));
    input.claim.fields.insert("source_op".into(), json!("clear"));
    let (cleared, changed) = store.append_harness_event(&input).unwrap();
    assert!(changed);
    assert_eq!(cleared.body["fields"]["phases"], json!([]));
    runtime(&store, "runtime-b", "running");
    input.sequence = 3;
    assert_eq!(store.append_harness_event(&input).unwrap_err().code, "stale-harness-event-session");
    let current = store.latest_observation(SEAT, "harness.todo.observed").unwrap().unwrap();
    assert_eq!(current.body["fields"]["source_op"], "clear");
    pub const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let peer = Store::open_memory("blue").unwrap();
    store.bind_fleet(FLEET).unwrap();
    peer.bind_fleet(FLEET).unwrap();
    let exchange = store.export_replication_exchange(
        FLEET, &ReplicationInventory::default(),
    ).unwrap();
    peer.receive_replication_exchange("amber", FLEET, &exchange).unwrap();
    peer.validate_replication_backlog().unwrap();
    peer.apply_replication_repairs().unwrap();
    assert!(peer.project_replication_backlog().unwrap());
    let remote = peer.agent_todo_observations_for(
        &[SEAT.to_owned()], peer.index().unwrap(),
    ).unwrap();
    let remote = &remote[SEAT]["harness.todo.observed"];
    assert_eq!(remote.id, cleared.id);
    assert_eq!(remote.body["fields"]["source_op"], "clear");
}

#[test]
fn native_event_admission_fences_every_retention_and_replays_lost_acknowledgements() {
    for kind in [
        "harness.observed",
        "harness.usage",
        "harness.limits",
        "harness.timeline",
    ] {
        let store = Store::open_memory("amber").unwrap();
        let mut input = event(kind);
        assert_eq!(
            store.append_harness_event(&input).unwrap_err().code,
            "stale-harness-event-session"
        );
        runtime(&store, "runtime-a", "running");
        let (first, changed) = store.append_harness_event(&input).unwrap();
        assert!(changed);
        let (replayed, changed) = store.append_harness_event(&input).unwrap();
        assert!(!changed);
        // Native limits reports keep the local log too, and replicate only what changes.
        let logged = latest_retention(kind) || kind == "harness.limits";
        if !logged {
            assert_eq!(first.id, replayed.id);
        }
        assert_eq!(replayed.body["fields"], json!(input.claim.fields));
        input.claim.fields.insert("driver".into(), json!("codex"));
        assert_eq!(
            store.append_harness_event(&input).unwrap_err().code,
            "idempotency-mismatch"
        );
        input.claim.fields.insert("driver".into(), json!("claude"));
        runtime(&store, "runtime-b", "running");
        assert_eq!(
            store.append_harness_event(&input).unwrap_err().code,
            "stale-harness-event-session"
        );
        input.runtime_incarnation = "runtime-b".into();
        let (replayed, changed) = store.append_harness_event(&input).unwrap();
        assert!(!changed);
        if !logged {
            assert_eq!(first.id, replayed.id);
        }
        input.sequence = 2;
        let (historical, _) = store.append_harness_event(&input).unwrap();
        assert_eq!(
            historical
                .body
                .pointer("/fields/incarnation_id")
                .and_then(Value::as_str),
            input
                .claim
                .fields
                .get("incarnation_id")
                .and_then(Value::as_str)
        );
        runtime(&store, "runtime-b", "exited");
        input.sequence = 3;
        assert_eq!(
            store.append_harness_event(&input).unwrap_err().code,
            "stale-harness-event-session"
        );
    }
}

#[test]
fn terminal_state_can_be_followed_by_its_final_timeline_before_runtime_exit() {
    let store = Store::open_memory("amber").unwrap();
    runtime(&store, "runtime-a", "running");
    let mut state = event("harness.observed");
    state.claim.fields.insert("state".into(), json!("ended"));
    store.append_harness_event(&state).unwrap();
    let mut timeline = event("harness.timeline");
    timeline.sequence = 2;
    store.append_harness_event(&timeline).unwrap();
    runtime(&store, "runtime-a", "exited");
    timeline.sequence = 3;
    assert!(store.append_harness_event(&timeline).is_err());
}

#[test]
fn event_envelopes_cannot_publish_foreign_or_non_harness_claims() {
    let store = Store::open_memory("amber").unwrap();
    runtime(&store, "runtime-a", "running");
    let mut input = event("harness.observed");
    input.claim.actor = Some("agent/example/other".into());
    assert_eq!(
        store.append_harness_event(&input).unwrap_err().code,
        "invalid-harness-event"
    );
    input.claim.actor = Some(SEAT.into());
    input.sequence = 0;
    assert!(store.append_harness_event(&input).is_err());
    input.sequence = 1;
    input.claim.kind = "intent.desired".into();
    assert!(store.append_harness_event(&input).is_err());
}

/// Four seats share one account for an hour. Each re-sends the reading it last saw every five
/// minutes, as idle seats do, while one of them measures the account three times. The graph
/// gets a claim per seat that joins the account and one per change, not one per report, and
/// reads the same account limits as a store that kept every report.
#[test]
fn limits_replicate_once_per_account_change_and_read_as_every_report_would() {
    const MINUTE: u64 = 60_000;
    const WEEK_END: u64 = 1_800_000_000_000;
    let deduped = Store::open_memory("amber").unwrap();
    let every = Store::open_memory("amber").unwrap();
    let seats = ["a", "b", "c", "d"].map(|name| format!("agent/example/limits-{name}"));
    for seat in &seats {
        deduped
            .append_claim(&ClaimInput {
                subject: seat.clone(),
                kind: "runtime.observed".into(),
                actor: Some(seat.clone()),
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!("runtime-a")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let reading = |measured_at: u64, weekly: f64, five_hour: f64| {
        BTreeMap::from([
            ("driver".into(), json!("claude")),
            ("account".into(), json!("claude/example-account")),
            ("incarnation_id".into(), json!("runtime-a")),
            ("measured_at_unix_ms".into(), json!(measured_at)),
            ("weekly_percent".into(), json!(weekly)),
            ("weekly_resets_at_unix_ms".into(), json!(WEEK_END)),
            ("five_hour_percent".into(), json!(five_hour)),
            ("five_hour_resets_at_unix_ms".into(), json!(WEEK_END - 100 * MINUTE)),
        ])
    };
    let start = WEEK_END - 1_000 * MINUTE;
    // What seat a measures at each minute; the other seats keep the first reading.
    let measured = [(0, 10.0, 30.0), (20, 11.0, 30.0), (40, 11.0, 35.0), (50, 11.0, 35.0), (60, 11.0, 35.0)];
    let mut latest = BTreeMap::new();
    let (mut reports, mut sequence) = (0, 0);
    let values = |limits: Vec<AccountLimit>| {
        limits
            .into_iter()
            .map(|limit| {
                (limit.plan, limit.weekly_percent, limit.five_hour_percent, limit.seats)
            })
            .collect::<Vec<_>>()
    };
    for minute in (0..=60).step_by(5) {
        if let Some((_, weekly, five_hour)) = measured.iter().find(|(at, ..)| *at == minute) {
            latest.insert(seats[0].clone(), reading(start + minute * MINUTE, *weekly, *five_hour));
        }
        for seat in &seats {
            let fields = latest
                .entry(seat.clone())
                .or_insert_with(|| reading(start, 10.0, 30.0))
                .clone();
            sequence += 1;
            reports += 1;
            let claim = ClaimInput {
                subject: seat.clone(),
                kind: "harness.limits".into(),
                actor: Some(seat.clone()),
                fields,
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("limits:{seat}:{sequence}")),
            };
            every.append_claim(&claim).unwrap();
            deduped
                .append_harness_event(&Publication {
                    runtime_incarnation: "runtime-a".into(),
                    sequence,
                    claim,
                })
                .unwrap();
        }
        assert_eq!(
            values(deduped.account_limits().unwrap()),
            values(every.account_limits().unwrap()),
            "minute {minute}"
        );
        let newest = |store: &Store| store.account_limits().unwrap()[0].measured_at_unix_ms;
        assert!(newest(&every) - newest(&deduped) < LIMITS_REFRESH_MS, "minute {minute}");
    }
    let claims = seats
        .iter()
        .map(|seat| deduped.claims_for(seat, Some("harness.limits")).unwrap().len())
        .sum::<usize>();
    assert_eq!(reports, 52);
    // One per seat joining the account, the weekly and five-hour changes, and the refresh at
    // minute 60 (minute 50 measured the same values within the refresh window).
    assert_eq!(claims, 4 + 2 + 1, "limits claims for {reports} reports");
    assert_eq!(
        deduped.account_limits().unwrap()[0].measured_at_unix_ms,
        every.account_limits().unwrap()[0].measured_at_unix_ms
    );
}
