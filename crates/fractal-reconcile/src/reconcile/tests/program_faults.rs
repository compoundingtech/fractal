//! A program step's faults (docs/st3/program-engine.md §8): `program/stalled` from a deadline the
//! evaluation arms on the journal's last entry, never a scan, and `program/host` from the engine.
use super::*;
use crate::program_journal::{HOST_FAULT_SCOPE, STALLED_FAULT_SCOPE};
use crate::reconcile::program::{JOURNAL_KIND, Liveness, ProgramEngine};

const MINUTE: u128 = 60_000;

/// A store with one run of a program mission whose step stalls after ten minutes.
fn program_run(store: &Store) -> (crate::model::MissionRunView, String) {
    let hash = store.put_blob(b"export default {}").unwrap();
    apply_source(
        store,
        &format!(
            r#"version 2
mission "program-faults" state="ready" {{
  goal "Run a program."
  concurrent-runs max=4
  step "program" {{ program "/bundles/program.mjs" sha256="{hash}" stalled-after="10m" }}
}}"#
        ),
        "program-faults-definition",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "program-faults".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: None,
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "program-faults-run".into(),
        })
        .unwrap();
    let step = run.steps.iter().find(|step| step.step == "program").unwrap().subject.clone();
    (run, step)
}

fn entry(store: &Store, step: &str, key: &str, status: &str) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: step.to_owned(),
            kind: JOURNAL_KIND.into(),
            actor: None,
            fields: BTreeMap::from([
                ("key".to_owned(), serde_json::Value::from(key)),
                ("status".to_owned(), serde_json::Value::from(status)),
                ("attempt".to_owned(), serde_json::Value::from(1)),
                ("program".to_owned(), serde_json::Value::from("a".repeat(64))),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

#[test]
fn a_quiet_journal_raises_program_stalled_at_its_deadline_and_an_entry_closes_it() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    let (_, step) = program_run(&store);
    store.set_step_state(&step, "working", None).unwrap();
    entry(&store, &step, "render", "started");
    entry(&store, &step, "render", "completed");
    entry(&store, &step, "merge", "started");
    let program = |at: u128| {
        let view = store.step_run(&step).unwrap().unwrap();
        let spec = crate::model::ProgramSpec {
            bundle: "/bundles/program.mjs".into(),
            bundle_hash: "a".repeat(64),
            stalled_after_ms: Some(10 * MINUTE as u64),
        };
        let ((), due) = smallclaims::touched::record_due(|| reconciler.reconcile_program_faults(&view, &spec, at).unwrap());
        due
    };
    let last = store.newest_claim_for_subject_kind(&step, JOURNAL_KIND).unwrap().unwrap().accepted_at_unix_ms;

    // Inside its stalled-after: no fault, and the evaluation is due again exactly at the deadline.
    assert_eq!(program(last + MINUTE), Some(last + 10 * MINUTE));
    assert_eq!(store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap(), None);

    // At the deadline the fault names the open key; a later evaluation records nothing new.
    assert_eq!(program(last + 10 * MINUTE), None);
    let reason = store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap().expect("program/stalled is raised");
    assert_eq!(reason, "program step `program` wrote no journal entry within its stalled-after of 10m; open key `merge`");
    program(last + 20 * MINUTE);
    let faulted = |status: &str| {
        store
            .claims_for(&step, Some("reconcile.fault"))
            .unwrap()
            .iter()
            .filter(|claim| claim.body["fields"]["scope"] == STALLED_FAULT_SCOPE && claim.body["fields"]["status"] == status)
            .count()
    };
    assert_eq!(faulted("faulted"), 1, "a lasting stall is recorded once");

    // Progress moves the deadline and closes the fault.
    entry(&store, &step, "merge", "completed");
    let moved = store.newest_claim_for_subject_kind(&step, JOURNAL_KIND).unwrap().unwrap().accepted_at_unix_ms;
    assert_eq!(program(moved + MINUTE), Some(moved + 10 * MINUTE));
    assert_eq!(store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap(), None);

    // A step that stops running cannot stall: no deadline, and an open stall closes.
    program(moved + 10 * MINUTE);
    assert!(store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap().is_some());
    store.set_step_state(&step, "completed", None).unwrap();
    assert_eq!(program(moved + 30 * MINUTE), None);
    assert_eq!(store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap(), None);
}

#[test]
fn a_host_that_cannot_start_raises_program_host_until_the_run_ends() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    // The engine records effect process groups in `groups`: a file there keeps any host from
    // starting, with or without a JavaScript runtime.
    let programs = dir.path().join("programs");
    std::fs::create_dir_all(&programs).unwrap();
    std::fs::write(programs.join("groups"), b"").unwrap();
    let engine = ProgramEngine::new(store.clone(), Arc::new(|_: &str| {}), programs, Liveness::default());
    assert!(reconciler.program_engine.set(engine.clone()).is_ok());
    let (run, step) = program_run(&store);

    let mut passes = 0;
    while store.reconcile_fault(&step, HOST_FAULT_SCOPE).unwrap().is_none() {
        passes += 1;
        assert!(passes <= 5, "program/host was not raised");
        reconciler.reconcile_once().unwrap();
    }
    let reason = store.reconcile_fault(&step, HOST_FAULT_SCOPE).unwrap().unwrap();
    assert!(reason.starts_with("the program host cannot start"), "{reason}");
    assert_eq!(engine.host_fault(&step, 1).as_deref(), Some(reason.as_str()));
    // The step keeps waiting to run; it neither fails nor counts as stalled.
    assert_eq!(store.step_run(&step).unwrap().unwrap().status, "working");
    assert_eq!(store.reconcile_fault(&step, STALLED_FAULT_SCOPE).unwrap(), None);

    // Once its run ends, the step has nothing left to fault: the inactive sweep closes it.
    store.set_step_state(&step, "completed", None).unwrap();
    let mut passes = 0;
    while store.reconcile_fault(&step, HOST_FAULT_SCOPE).unwrap().is_some() {
        passes += 1;
        assert!(passes <= 10, "program/host stayed open after run {} ended", run.subject);
        reconciler.reconcile_once().unwrap();
    }
}
