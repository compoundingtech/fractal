//! Authored scheduler contract controls. Qualification runs separately on the accepted source.
use super::*;

fn source(subject: &str, added: bool, protected: bool, worker: bool) -> String {
    let protection = if protected {
        "revisions=\"human-only\" revision-reviewer=\"person/lichen\""
    } else {
        ""
    };
    let policy = if added { "restart-active" } else { "when-idle" };
    let worker = if worker {
        "step \"worker\" { assigned-to \"person/lichen\"; goal \"Keep actual claimed work.\" }"
    } else {
        ""
    };
    let extra = if added {
        "step \"sorter\" { assigned-to \"person/lichen\"; goal \"Inspect invented rows.\" }"
    } else {
        ""
    };
    format!(
        r#"version 2
mission "orchid-cutover" state="ready" revision-cutover={policy:?} {protection} {{
 goal "Keep a false external hold while admitting unrelated work."
 completion {{ when "all-steps-exhausted" }}
 step "hold" timeout="1h" {{ agentless; gate "Explicit release" {{ exists {subject:?} }} }}
 {worker}
 {extra}
}}"#
    )
}

fn fixture(source: &str) -> (Arc<Store>, MissionRunView, Reconciler<FakeRuntime>) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, source, "initial");
    let run = store
        .create_mission_run(&crate::model::MissionRunRequest {
            mission: "orchid-cutover".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/lichen".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "run".into(),
        })
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    )
    .skipping_unneeded(true);
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    (store, run, reconciler)
}

fn current(store: &Store, run: &MissionRunView) -> MissionRunView {
    store.mission_run_for_reconcile(&run.id).unwrap()
}

fn propose(
    store: &Store,
    run: &MissionRunView,
    source: &str,
) -> crate::model::RevisionProposalView {
    apply_source(store, source, "candidate");
    let candidate = store.mission_spec("orchid-cutover", None).unwrap().unwrap();
    store
        .create_revision_proposal(&run.id, &candidate, "person/lichen", "Add work", "propose")
        .unwrap()
}

#[test]
fn unrelated_revision_carries_false_passive_hold_and_its_spent_execution_clock() {
    let (store, run, _) = fixture(&source("resource/orchid/release", false, false, false));
    let before = current(&store, &run).steps.remove(0);
    assert_eq!(before.status, "working");
    assert!(before.agentless && before.claimant.is_none());
    assert!(before.execution_started_at_unix_ms.is_some());
    let proposal = propose(
        &store,
        &run,
        &source("resource/orchid/release", true, false, false),
    );
    assert_eq!(proposal.cutover, crate::model::RevisionCutover::WhenIdle);
    let result = store
        .apply_drained_revision(&run.id)
        .unwrap()
        .expect("pure compatible hold can carry");
    let carried = result
        .mission_run
        .steps
        .iter()
        .find(|s| s.step == "hold")
        .unwrap();
    assert_ne!(result.mission_run.generation, run.generation);
    assert_eq!(carried.status, "working");
    assert!(carried.agentless && carried.claimant.is_none());
    assert_eq!(carried.attempt, before.attempt);
    assert_eq!(carried.readiness_epoch, before.readiness_epoch);
    assert_eq!(
        carried.execution_started_at_unix_ms,
        before.execution_started_at_unix_ms
    );
    assert!(carried.execution_elapsed_ms >= before.execution_elapsed_ms);
    assert_eq!(carried.execution_waited_ms, before.execution_waited_ms);
    assert_eq!(carried.timeout_extension_ms, before.timeout_extension_ms);
    assert!(
        store
            .claims_for(&carried.subject, Some("work.claimed"))
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .claims_for("resource/orchid/release", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn changed_or_generation_dependent_hold_predicate_cannot_be_exempted() {
    for subject in [
        "resource/orchid/release",
        "resource/orchid/${ST_RUN_GENERATION}",
    ] {
        let (store, run, _) = fixture(&source(subject, false, false, false));
        let next = if subject.contains("${") {
            subject
        } else {
            "resource/orchid/other-release"
        };
        let proposal = propose(&store, &run, &source(next, true, false, false));
        let before = store.index().unwrap();
        assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
        assert_eq!(store.index().unwrap(), before);
        assert_eq!(current(&store, &run).generation, run.generation);
        assert_eq!(
            store
                .revision_proposal(&proposal.id)
                .unwrap()
                .unwrap()
                .status,
            "draining"
        );
    }
}

#[test]
fn pure_hold_does_not_bypass_pending_review_or_an_active_worker_lease() {
    for protected in [true, false] {
        let worker = !protected;
        let (store, run, _) = fixture(&source("resource/orchid/release", false, protected, worker));
        if worker {
            let step = current(&store, &run)
                .steps
                .into_iter()
                .find(|s| s.step == "worker")
                .unwrap();
            let request = |key: &str| crate::model::WorkRequest {
                wait_ms: None,
                actor: Some("person/lichen".into()),
                incarnation: Some("lichen-one".into()),
                summary: None,
                reason: None,
                evidence: Vec::new(),
                idempotency_key: key.into(),
            };
            store
                .work_action(&step.subject, "claim", &request("claim"))
                .unwrap();
            store
                .work_action(&step.subject, "progress", &request("progress"))
                .unwrap();
        }
        let proposal = propose(
            &store,
            &run,
            &source("resource/orchid/release", true, protected, worker),
        );
        assert_eq!(
            proposal.status,
            if protected {
                "pending-approval"
            } else {
                "draining"
            }
        );
        let before = current(&store, &run);
        let index = store.index().unwrap();
        assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
        assert_eq!(store.index().unwrap(), index);
        let after = current(&store, &run);
        assert_eq!(after.generation, before.generation);
        for original in before.steps {
            let same = after
                .steps
                .iter()
                .find(|s| s.subject == original.subject)
                .unwrap();
            assert_eq!(
                (
                    &same.status,
                    &same.claimant,
                    &same.claim_incarnation,
                    same.claim_expires_at_unix_ms
                ),
                (
                    &original.status,
                    &original.claimant,
                    &original.claim_incarnation,
                    original.claim_expires_at_unix_ms
                )
            );
        }
    }
}

#[test]
fn passive_parent_hold_does_not_cancel_a_nonterminal_descendant() {
    let (store, run, _) = fixture(&source("resource/orchid/release", false, false, false));
    apply_source(
        &store,
        "version 2\nmission \"orchid-child\" state=\"ready\" { goal \"Keep pending descendant work.\"; step \"review\" { assigned-to \"person/lichen\" } }",
        "child",
    );
    let hold = current(&store, &run).steps.remove(0);
    let child = store
        .create_child_mission_run(
            &crate::model::MissionRunRequest {
                mission: "orchid-child".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/lichen".into()),
                mode: None,
                inputs: BTreeMap::new(),
                idempotency_key: "child-run".into(),
            },
            &run,
            &hold.subject,
            None,
        )
        .unwrap();
    propose(
        &store,
        &run,
        &source("resource/orchid/release", true, false, false),
    );
    let index = store.index().unwrap();
    assert!(store.apply_drained_revision(&run.id).unwrap().is_none());
    assert_eq!(store.index().unwrap(), index);
    assert_eq!(current(&store, &run).generation, run.generation);
    let preserved = store.mission_run(&child.id).unwrap().unwrap();
    assert_eq!(preserved.generation, child.generation);
    assert_eq!(preserved.status, child.status);
    assert_eq!(preserved.steps[0].status, child.steps[0].status);
}
