//! A one-field status read of a seat costs the same however many claims the seat has. Drivers
//! and `st work` read a seat's incarnation from its status, and the full reduction folded every
//! runtime observation of the seat and listed every claim id: seconds on a busy host's store.
use super::*;
use serde_json::json;

const SEAT: &str = "agent/example/worker";

fn observe(store: &Store, fields: serde_json::Value) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn seat_with(observations: usize) -> Store {
    let store = Store::open_memory("node").unwrap();
    for index in 0..observations {
        observe(&store, json!({
            "status": if index % 2 == 0 { "running" } else { "stopped" },
            "incarnation_id": format!("incarnation-{index}"),
            "runtime_id": format!("runtime-{index}"),
        }));
    }
    store
}

fn work(read: impl FnOnce() -> StatusResponse) -> (smallclaims::sqlite::work::SqliteWork, StatusResponse) {
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let status = read();
    (scope.finish(), status)
}

#[test]
fn a_summary_status_read_of_a_seat_does_not_read_its_history() {
    let (few, many) = (seat_with(50), seat_with(3_000));
    let (few_summary, _) = work(|| few.status_summary_at(SEAT, None).unwrap());
    let (many_summary, summary) = work(|| many.status_summary_at(SEAT, None).unwrap());
    let (few_full, _) = work(|| few.status_at(Some(SEAT), None, None).unwrap());
    let (many_full, full) = work(|| many.status_at(Some(SEAT), None, None).unwrap());
    // The full reduction grows with the history; this is what the summary avoids.
    assert!(many_full.vm_steps > few_full.vm_steps * 10, "50={few_full:?}, 3000={many_full:?}");
    assert!(
        many_summary.vm_steps <= few_summary.vm_steps + 2_000,
        "the summary read the seat's history: 50={few_summary:?}, 3000={many_summary:?}"
    );
    // It answers what the one-field callers read, as the full reduction does.
    let (summary, full) = (&summary.subjects[0], &full.subjects[0]);
    assert!(summary.claims.is_empty());
    assert_eq!(full.claims.len(), 3_000);
    let (summary_actual, full_actual) = (summary.actual.as_ref().unwrap(), full.actual.as_ref().unwrap());
    for field in ["incarnation_id", "status"] {
        assert_eq!(summary_actual[field], full_actual[field], "{field}");
    }
    assert_eq!(summary_actual["incarnation_id"], "incarnation-2999");
    assert_eq!(summary.desired, full.desired);
    assert!(summary.harness.is_none());
    assert_eq!(summary.projection.layer, full.projection.layer);
}

#[test]
fn a_summary_takes_each_field_from_the_newest_observation_that_has_it() {
    let store = seat_with(10);
    observe(&store, json!({"status": "stopped"}));
    let summary = store.status_summary_at(SEAT, None).unwrap();
    let full = store.status_at(Some(SEAT), None, None).unwrap();
    let (summary, full) = (summary.subjects[0].actual.as_ref().unwrap(), full.subjects[0].actual.as_ref().unwrap());
    assert_eq!(summary["status"], "stopped");
    assert_eq!(summary["incarnation_id"], "incarnation-9");
    for field in ["incarnation_id", "status"] {
        assert_eq!(summary[field], full[field], "{field}");
    }
}

/// Times a seat's full and incarnation reductions on a store copy, read-only and unchanged:
/// `ST_STATUS_SUMMARY_BENCH_STORE` names the file and `ST_STATUS_SUMMARY_BENCH_SEATS` the seats,
/// comma-separated. Each seat's incarnation reduction runs first, on a cold cache.
#[test]
#[ignore = "requires ST_STATUS_SUMMARY_BENCH_STORE, a store copy, and ST_STATUS_SUMMARY_BENCH_SEATS"]
fn status_summary_benchmark() {
    let path = std::env::var("ST_STATUS_SUMMARY_BENCH_STORE").unwrap();
    let connection = Connection::open_with_flags(
        format!("file:{path}?mode=ro&immutable=1"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    for seat in std::env::var("ST_STATUS_SUMMARY_BENCH_SEATS").unwrap().split(',') {
        for claims in [ClaimsReduction::Incarnation, ClaimsReduction::Full, ClaimsReduction::Incarnation, ClaimsReduction::Full] {
            let started = std::time::Instant::now();
            let (status, _) = subject_status_at(&connection, seat, None, None, claims).unwrap().unwrap();
            let actual = status.actual.unwrap_or_default();
            eprintln!(
                "{seat} {claims:?}: {:.1} ms, {} claim ids, incarnation {}, status {}",
                started.elapsed().as_secs_f64() * 1000.0, status.claims.len(), actual["incarnation_id"], actual["status"]
            );
        }
    }
}

