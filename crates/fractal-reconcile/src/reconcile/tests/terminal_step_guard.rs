use super::*;

fn fixture() -> (Arc<Store>, MissionRunView, MissionSpec, String) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        r#"version 2
resource "terminal-guard-marker" { kind "custom.st3.document-source" }
mission "terminal-guard-caller" state="ready" {
 goal "Ignore stale agentless transitions after a normal terminal write."
 step "a" {
  agentless
  gate "keep waiting" { field "state" "resource/terminal-guard-marker" is "ready" }
 }
 step "keep-running" { agentless; depends-on "a" }
}"#,
        "terminal-guard-caller-definition",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "terminal-guard-caller".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "terminal-guard-caller-run".into(),
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
    let (saved, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
    (store, saved, mission.unwrap(), subject)
}

fn durable_step(store: &Store, subject: &str) -> (Vec<rusqlite::types::Value>, Vec<String>) {
    let row = {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare("SELECT * FROM step_runs WHERE subject=?1")
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_row(/* sqlite-large-value: allow-for-now lv-hc9439392b3a3 owner=fractal_reconcile::reconcile::tests::terminal_step_guard TODO=trace-statement-and-value-representation */ [subject], |row| {
                (0..columns)
                    .map(|column| row.get(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
    };
    let claims = store
        .claims_for(subject, Some("step-run.state"))
        .unwrap()
        .into_iter()
        .map(|claim| serde_json::to_string(&claim).unwrap())
        .collect();
    (row, claims)
}

#[test]
fn terminal_step_guard_agentless_admission_uses_the_stored_terminal_guard() {
    for completed_after_read in [false, true] {
        let (store, saved, mission, subject) = fixture();
        assert_eq!(
            saved
                .steps
                .iter()
                .find(|step| step.subject == subject)
                .unwrap()
                .status,
            "ready"
        );
        assert!(
            saved
                .steps
                .iter()
                .any(|step| step.step == "keep-running" && step.status == "pending")
        );
        if completed_after_read {
            assert!(
                store
                    .set_step_state(
                        &subject,
                        "completed",
                        Some("normal writer after saved view")
                    )
                    .unwrap()
            );
        }
        let before = durable_step(&store, &subject);
        let reconciler = Reconciler::new(
            store.clone(),
            Arc::new(FakeRuntime::default()),
            "node".into(),
            Arc::new(Notify::new()),
        );
        // The unsatisfied gate prevents an unrelated completion decision. The live
        // twin proves this fixture reaches admission instead of passing via an early skip.
        reconciler.evaluate_mission_run(&saved, &mission).unwrap();
        let after = durable_step(&store, &subject);
        if completed_after_read {
            assert_eq!(after, before);
        } else {
            assert_eq!(store.step_run(&subject).unwrap().unwrap().status, "working");
            assert_eq!(after.1.len(), before.1.len() + 1);
        }
    }
}

#[test]
fn terminal_step_guard_agentless_timeout_uses_the_stored_terminal_guard() {
    for terminal in ["completed", "failed", "cancelled"] {
        let (store, saved, mission, subject) = fixture();
        let view = saved
            .steps
            .iter()
            .find(|step| step.subject == subject)
            .unwrap();
        let flat = flatten_mission_steps(&mission);
        let step = flat.iter().find(|step| step.spec.path == "a").unwrap();
        assert!(
            store
                .set_step_state(&subject, terminal, Some("normal writer after saved view"))
                .unwrap()
        );
        let before = durable_step(&store, &subject);
        let claims_before = store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claims", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        let reconciler = Reconciler::new(
            store.clone(),
            Arc::new(FakeRuntime::default()),
            "node".into(),
            Arc::new(Notify::new()),
        );
        assert!(!reconciler.expire_step_timeout(view, step).unwrap());
        assert_eq!(durable_step(&store, &subject), before);
        assert_eq!(
            store
                .readers
                .get()
                .query_row("SELECT COUNT(*) FROM claims", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            claims_before
        );
    }
}
