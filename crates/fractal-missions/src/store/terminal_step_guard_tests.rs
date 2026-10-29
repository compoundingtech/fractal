use super::*;
use crate::store::tests::publish_mission;

fn fixture() -> (Store, MissionRunView, String) {
    let store = Store::open_memory("node").unwrap();
    publish_mission(
        &store,
        r#"version 2
mission "terminal-step-guard" state="ready" {
 concurrent-runs
 goal "Keep terminal work terminal despite stale evaluator decisions."
 step "a" { agentless }
 step "keep-running" { agentless }
}"#,
        "terminal-step-guard-definition",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "terminal-step-guard".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-step-guard-run".into(),
        })
        .unwrap();
    let subject = run
        .steps
        .iter()
        .find(|step| step.step == "a")
        .unwrap()
        .subject
        .clone();
    assert!(store.set_step_state(&subject, "ready", None).unwrap());
    (store, run, subject)
}

#[derive(Debug, PartialEq)]
struct Durable {
    // Preserve every stored column, including lease, readiness and timestamp fields.
    row: Vec<rusqlite::types::Value>,
    state_claims: Vec<(String, String, String)>,
    total_claims: u64,
}

fn durable(store: &Store, subject: &str) -> Durable {
    let connection = store.readers.get();
    let mut statement = connection
        .prepare("SELECT * FROM step_runs WHERE subject=?1")
        .unwrap();
    let columns = statement.column_count();
    let row = statement
        .query_row(/* sqlite-large-value: allow-for-now lv-h197e92111ab4 owner=fractal_missions::store::terminal_step_guard_tests TODO=trace-statement-and-value-representation */ [subject], |row| {
            (0..columns)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap();
    let state_claims = connection.prepare(
        "SELECT id, body, accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='step-run.state' ORDER BY store_index",
    ).unwrap().query_map([subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let total_claims = connection
        .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
        .unwrap();
    Durable {
        row,
        state_claims,
        total_claims,
    }
}

#[test]
fn terminal_step_guard_rejects_saved_ready_admission_and_timeout_without_writes() {
    for terminal in ["completed", "failed", "cancelled"] {
        let (store, run, subject) = fixture();
        let (saved, _) = store.mission_run_for_evaluation(&run.id).unwrap();
        let saved_step = saved
            .steps
            .iter()
            .find(|step| step.subject == subject)
            .unwrap();
        assert!(saved_step.agentless);
        assert_eq!(saved_step.status, "ready");
        assert!(
            saved
                .steps
                .iter()
                .any(|step| step.step == "keep-running" && step.status == "pending")
        );
        // Deterministic interleaving: the normal writer commits after the evaluator's read.
        assert!(
            store
                .set_step_state(&subject, terminal, Some("normal writer wins"))
                .unwrap()
        );
        let current = store.mission_run_for_reconcile(&run.id).unwrap();
        assert_eq!(current.status, "running");
        assert_eq!(current.generation, saved.generation);
        let before = durable(&store, &subject);
        for target in ["working", "failed"] {
            assert!(
                !store
                    .set_step_state_unless_terminal(
                        &saved_step.subject,
                        target,
                        Some("stale decision")
                    )
                    .unwrap()
            );
            assert_eq!(durable(&store, &subject), before, "{terminal} -> {target}");
        }
    }
}

#[test]
fn terminal_step_guard_keeps_ready_transitions_equal_target_and_missing_noops() {
    for target in ["working", "failed"] {
        let (store, _, subject) = fixture();
        let before = durable(&store, &subject);
        assert!(
            store
                .set_step_state_unless_terminal(&subject, target, Some("current decision"))
                .unwrap()
        );
        let after = durable(&store, &subject);
        assert_eq!(after.total_claims, before.total_claims + 1);
        assert_eq!(after.state_claims.len(), before.state_claims.len() + 1);
        assert_eq!(store.step_run(&subject).unwrap().unwrap().status, target);
        assert!(
            !store
                .set_step_state_unless_terminal(&subject, target, Some("repeat"))
                .unwrap()
        );
        assert!(
            !store
                .set_step_state_unless_terminal("step-run/missing", "working", None)
                .unwrap()
        );
        assert_eq!(durable(&store, &subject), after);
    }
}

#[test]
fn terminal_step_guard_accepts_expired_lease_views_without_saved_epoch_cas() {
    for stored in ["claimed", "working"] {
        for target in ["working", "failed"] {
            let (store, run, subject) = fixture();
            assert!(store.set_step_state(&subject, stored, None).unwrap());
            store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h6726abdbf56a owner=fractal_missions::store::terminal_step_guard_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_owner='person/test', lease_incarnation='old-worker', lease_expires_at_unix_ms='0' WHERE subject=?1",
                [&subject],
            ).unwrap();
            let (saved, _) = store.mission_run_for_evaluation(&run.id).unwrap();
            let effective = saved
                .steps
                .iter()
                .find(|step| step.subject == subject)
                .unwrap();
            assert_eq!(effective.status, "ready");
            let stored_epoch: u32 = store
                .readers
                .get()
                .query_row(
                    "SELECT readiness_epoch FROM step_runs WHERE subject=?1",
                    [&subject],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(effective.readiness_epoch > stored_epoch);
            let before = durable(&store, &subject);
            let changed = store
                .set_step_state_unless_terminal(&subject, target, None)
                .unwrap();
            assert_eq!(changed, stored != target);
            if changed {
                assert_eq!(
                    durable(&store, &subject).total_claims,
                    before.total_claims + 1
                );
                let persisted: String = store
                    .readers
                    .get()
                    .query_row(
                        "SELECT status FROM step_runs WHERE subject=?1",
                        [&subject],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(persisted, target);
            } else {
                assert_eq!(durable(&store, &subject), before);
            }
        }
    }
}

#[test]
fn terminal_step_guard_preserves_owner_and_noncurrent_generation_guards() {
    for owner in [
        "run-status",
        "run-phase",
        "generation-status",
        "root-status",
        "root-phase",
        "noncurrent",
    ] {
        let (store, run, mut subject) = fixture();
        // A child distinguishes its live direct owner from the terminal root owner.
        if owner.starts_with("root-") {
            let child = store
                .create_child_mission_run(
                    &MissionRunRequest {
                        mission: "terminal-step-guard".into(),
                        revision: None,
                        workspace: "/tmp".into(),
                        requester: None,
                        mode: None,
                        inputs: BTreeMap::new(),
                        idempotency_key: "guard-child".into(),
                    },
                    &run,
                    &subject,
                    None,
                )
                .unwrap();
            subject = child
                .steps
                .iter()
                .find(|step| step.step == "a")
                .unwrap()
                .subject
                .clone();
            assert!(store.set_step_state(&subject, "ready", None).unwrap());
        }
        let sql = match owner {
            "run-status" | "root-status" => {
                /* sqlite-large-value: allow-for-now lv-hcfb1c0360b24 owner=fractal_missions::store::terminal_step_guard_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE mission_runs SET status='completed' WHERE id=?1"
            }
            "run-phase" | "root-phase" => {
                /* sqlite-large-value: allow-for-now lv-hab21a6e0c8bd owner=fractal_missions::store::terminal_step_guard_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE mission_runs SET phase='terminal' WHERE id=?1"
            }
            "generation-status" => {
                /* sqlite-large-value: allow-for-now lv-h2d453ea189fe owner=fractal_missions::store::terminal_step_guard_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE run_generations SET status='completed' WHERE run_id=?1"
            }
            "noncurrent" => {
                /* sqlite-large-value: allow-for-now lv-h616e5943bc18 owner=fractal_missions::store::terminal_step_guard_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE mission_runs SET current_generation_id='replacement-generation' WHERE id=?1"
            }
            _ => unreachable!(),
        };
        store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h1cbf9f654bef owner=fractal_missions::store::terminal_step_guard_tests TODO=trace-statement-and-value-representation */ sql, [&run.id]).unwrap();
        let before = durable(&store, &subject);
        assert!(
            !store
                .set_step_state_unless_terminal(&subject, "working", None)
                .unwrap(),
            "{owner}"
        );
        assert!(
            !store.set_step_state(&subject, "working", None).unwrap(),
            "{owner}"
        );
        assert_eq!(durable(&store, &subject), before, "{owner}");
        if owner != "noncurrent" {
            // Existing guards permit terminal targets even when the owner is terminal.
            assert!(
                store
                    .set_step_state_unless_terminal(&subject, "failed", None)
                    .unwrap(),
                "{owner}"
            );
        } else {
            assert!(
                !store
                    .set_step_state_unless_terminal(&subject, "failed", None)
                    .unwrap()
            );
            assert_eq!(durable(&store, &subject), before);
        }
    }
}

#[test]
fn terminal_step_guard_leaves_administrative_reopening_and_retry_available() {
    for terminal in ["completed", "failed", "cancelled"] {
        let (store, _, subject) = fixture();
        assert!(store.set_step_state(&subject, terminal, None).unwrap());
        assert!(
            !store
                .set_step_state_unless_terminal(&subject, "working", None)
                .unwrap()
        );
        assert!(
            store
                .set_step_state(&subject, "working", Some("explicit administration"))
                .unwrap()
        );
        assert_eq!(store.step_run(&subject).unwrap().unwrap().status, "working");
    }
    let (store, _, subject) = fixture();
    assert!(store.set_step_state(&subject, "failed", None).unwrap());
    let attempt = store.step_run(&subject).unwrap().unwrap().attempt;
    assert!(store.retry_step(&subject, "legitimate retry", 0).unwrap());
    let retried = store.step_run(&subject).unwrap().unwrap();
    assert_eq!(retried.attempt, attempt + 1);
    assert!(
        store
            .set_step_state_unless_terminal(&subject, "working", None)
            .unwrap()
    );
}
