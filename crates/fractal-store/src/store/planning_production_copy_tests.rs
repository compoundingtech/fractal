//! How long one replicated planning claim holds the writer on a production-size store copy
//! (#4756). Ignored: set `ST3_PLANNING_PRODUCTION_COPY` to a disposable, checkpointed copy of a
//! store. The test writes to it. It prints one JSON receipt: the drain of any backlog the copy
//! held, then the projection pass for each planning claim, with its writer-lend stages and the
//! claims each stage rebuilt. `ST3_PLANNING_PROBE_SESSION` names the probe session, so several
//! builds can be measured in turn on one copy.

use super::*;
use std::time::Instant;

fn lend_receipt(took: std::time::Duration) -> Value {
    let (stages, rebuilt) = smallclaims::windows::holds::recent_staged()
        .into_iter()
        .rev()
        .find(|(stages, _)| stages.iter().any(|(stage, _)| *stage == "incremental"))
        .unwrap_or_default();
    json!({
        "wall_ms": took.as_millis(),
        "held_ms": stages.iter().map(|(_, spent)| spent.as_millis()).sum::<u128>(),
        "stages": stages.iter().filter(|(_, spent)| spent.as_millis() > 0)
            .map(|(stage, spent)| json!([stage, spent.as_millis()])).collect::<Vec<_>>(),
        "rebuilt": rebuilt.iter().map(|(stage, rebuilt)| json!([stage, rebuilt.aggregates, rebuilt.claims]))
            .collect::<Vec<_>>(),
    })
}

#[test]
#[ignore = "needs ST3_PLANNING_PRODUCTION_COPY, a disposable production-size store copy"]
fn planning_claim_writer_hold_on_production_copy() {
    let path = std::env::var_os("ST3_PLANNING_PRODUCTION_COPY")
        .expect("ST3_PLANNING_PRODUCTION_COPY names a disposable store copy");
    let opened = Instant::now();
    let store = Store::open(Path::new(&path), "planning-copy").unwrap();
    let open_ms = opened.elapsed().as_millis();
    let count = |sql: &str| -> i64 {
        store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-h6bb45b2be831 owner=fractal_store::store::planning_production_copy_tests TODO=trace-statement-and-value-representation */ sql, [], |row| row.get(0)).unwrap()
    };
    let claims = count("SELECT COUNT(*) FROM claims");
    let planning = count(
        "SELECT COUNT(*) FROM claims WHERE kind >= 'planning-session.' AND kind < 'planning-session/'",
    );
    let drained = Instant::now();
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog().unwrap();
    let drain = json!({"wall_ms": drained.elapsed().as_millis(), "full_replays": FULL_REPLAYS.with(std::cell::Cell::get)});
    let session = std::env::var("ST3_PLANNING_PROBE_SESSION").unwrap_or_else(|_| "production-copy-probe".into());
    let mut passes = Vec::new();
    let mut predecessors = Vec::new();
    for (event, fields) in [
        ("started", json!({"mission":"mission/example", "request":"doc/request", "workspace":"workspace",
            "requester":"person/operator", "planner":"agent/planner"})),
        ("candidate-submitted", json!({"variant":"default", "candidate_revision":1, "markdown":"doc/candidate",
            "kdl":"doc/definition", "mission_revision":"revision-one"})),
    ] {
        let claim = {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            let claim = append_claim_record_tx(
                &transaction, "example-peer", &format!("planning-session/{session}"),
                &format!("planning-session.{event}"), Some("person/operator"),
                &json!({"fields":fields, "_operation":{"id":format!("op/{session}-{event}"), "request_digest":event}}),
                &predecessors, None,
            ).unwrap();
            transaction.commit().unwrap();
            claim
        };
        predecessors = vec![claim.id];
        FULL_REPLAYS.with(|count| count.set(0));
        let started = Instant::now();
        store.project_replication_backlog().unwrap();
        let mut receipt = lend_receipt(started.elapsed());
        receipt["event"] = json!(event);
        receipt["full_replays"] = json!(FULL_REPLAYS.with(std::cell::Cell::get));
        passes.push(receipt);
    }
    println!(
        "{}",
        json!({"claims": claims, "planning_claims": planning, "open_ms": open_ms, "drain": drain, "passes": passes})
    );
}
