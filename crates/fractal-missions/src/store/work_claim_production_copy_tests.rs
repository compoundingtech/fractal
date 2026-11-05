//! How long claiming and completing one step holds the writer on a production-size store copy
//! (#5010). Ignored: set `ST3_WORK_CLAIM_PRODUCTION_COPY` to a disposable, checkpointed copy of
//! a store. The test writes to it. For each of the agents the copy holds the most mail for, it
//! starts one spontaneous step, claims it and completes it, twice, and prints one JSON receipt
//! with each action's longest writer hold, the SQLite work its writer job did, and how long the
//! step's wake took to read on a reader.

use super::*;
use crate::model::WorkStartRequest;
use crate::store::adhoc_work::AdhocWorkStore;
use fractal_store::store::WORK_ACTION_WRITER_WORK;
use std::time::Instant;

fn held_ms<T>(work: impl FnOnce() -> T) -> (T, f64) {
    let (result, longest) = smallclaims::windows::holds::longest_during(work);
    (result, longest.as_micros() as f64 / 1000.0)
}

#[test]
#[ignore = "needs ST3_WORK_CLAIM_PRODUCTION_COPY, a disposable production-size store copy"]
fn work_claim_writer_hold_on_production_copy() {
    let path = std::env::var_os("ST3_WORK_CLAIM_PRODUCTION_COPY")
        .expect("ST3_WORK_CLAIM_PRODUCTION_COPY names a disposable store copy");
    let store = Store::open(Path::new(&path), "work-claim-copy").unwrap();
    store.project_replication_backlog().unwrap();
    let assignees = {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare(
                "SELECT json_extract(body, '$.fields.to') AS recipient, COUNT(*) FROM claims INDEXED BY claims_message_to_index
                 WHERE kind='message.sent' AND recipient LIKE 'agent/%'
                 GROUP BY recipient ORDER BY COUNT(*) DESC, recipient LIMIT 5",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let probe = std::env::var("ST3_WORK_CLAIM_PROBE").unwrap_or_else(|_| "work-claim-copy-probe".into());
    let mut actions = Vec::new();
    for (index, (assignee, mail)) in assignees.iter().enumerate() {
        for pass in 0..2 {
            let key = format!("{probe}:{index}:{pass}");
            let request = |action: &str| WorkRequest {
                wait_ms: None,
                actor: Some(assignee.clone()),
                incarnation: Some(format!("{probe}-incarnation")),
                summary: Some("Measured on a copy.".into()),
                reason: None,
                evidence: vec!["doc/work-claim-copy".into()],
                idempotency_key: format!("{key}:{action}"),
            };
            let mut receipt = json!({"assignee": index, "mail": mail, "pass": pass});
            let started = store.start_work(&WorkStartRequest {
                actor: assignee.clone(),
                title: "Measure a work claim".into(),
                idempotency_key: key.clone(),
            });
            let subject = match started {
                Ok(step) => step.subject,
                Err(error) => {
                    receipt["error"] = json!(format!("start: {error}"));
                    actions.push(receipt);
                    continue;
                }
            };
            for action in ["claim", "complete"] {
                let wall = Instant::now();
                let (result, held) = held_ms(|| store.work_action(&subject, action, &request(action)));
                receipt[action] = json!({
                    "held_ms": held,
                    "wall_ms": wall.elapsed().as_micros() as f64 / 1000.0,
                    "error": result.as_ref().err().map(ToString::to_string),
                });
                // What the action's writer job read and wrote, to tell a cold hold from more work.
                if let Some(work) = WORK_ACTION_WRITER_WORK.lock().unwrap().get(&format!("{key}:{action}")) {
                    receipt[action]["writer"] = json!({
                        "statements": work.statements,
                        "vm_steps": work.vm_steps,
                        "fullscan_steps": work.fullscan_steps,
                        "sorts": work.sorts,
                        "autoindex_rows": work.autoindex_rows,
                    });
                }
                if let Ok(mut view) = result {
                    let read = Instant::now();
                    fractal_store::store::enrich_step_wake_at(&store.readers.get(), &mut view, now_ms()).unwrap();
                    receipt[action]["wake_read_ms"] = json!(read.elapsed().as_micros() as f64 / 1000.0);
                }
            }
            actions.push(receipt);
        }
    }
    let count = |sql: &str| -> i64 {
        store.readers.get().query_row(sql, [], |row| row.get(0)).unwrap()
    };
    println!(
        "{}",
        json!({"claims": count("SELECT COUNT(*) FROM claims"), "messages": count("SELECT COUNT(*) FROM claims WHERE kind='message.sent'"),
            "actions": actions})
    );
}
