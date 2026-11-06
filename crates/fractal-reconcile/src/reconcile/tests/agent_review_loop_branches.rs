use super::*;
use crate::model::LoopForEachSpec;
use crate::references::Scope;
use crate::store::missions::validate_run_reviewer;
use serde_json::json;

fn fixture() -> (
    Arc<Store>,
    Reconciler<FakeRuntime>,
    MissionRunView,
    MissionSpec,
) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        r#"version 2
agent "worker" { command "true"; workspace "/tmp" }
agent "free" { command "true"; workspace "/tmp" }
resource "batch" { kind "custom.test.batch" }
mission "review" state="ready" { goal "Review branch responsibility."
  loop "cycle" { max-rounds 2
    metric "score" direction="higher" { field "score" "resource/batch" }
    round { completion { when "all-steps-exhausted" }; step "work" { agentless } }
    keep-if metric="score"
    on-keep { completion { when "all-steps-exhausted" }; step "keep" { agentless } }
    on-discard { completion { when "all-steps-exhausted" }; step "discard" { agentless } }
  }
}
mission "extra" state="ready" { goal "An unrelated child definition."
  loop "other" { max-rounds 1; round {
    completion { when "all-steps-exhausted" }; step "work" { agentless }
  } }
}"#,
        "declare-review",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "review".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "review-run".into(),
        })
        .unwrap();
    let mission = store
        .mission_spec("review", Some(&run.revision))
        .unwrap()
        .unwrap();
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    (store, reconciler, run, mission)
}

fn check(store: &Store, run: &MissionRunView, reviewer: &str) -> String {
    let scope = Scope {
        run: BTreeSet::new(),
        intent: None,
    };
    validate_run_reviewer(
        &scope,
        &store.connection.write(),
        &run.subject,
        "person/test",
        reviewer,
    )
    .map_or_else(|error| error.code.to_owned(), |_| "ok".into())
}

#[test]
fn agent_review_loop_branches_include_workers_and_reject_unknown_children() {
    for keep in [true, false] {
        let (store, reconciler, run, mission) = fixture();
        let steps = flatten_mission_steps(&mission);
        let view = &run.steps[0];
        let spec = steps[0].spec.loop_spec.as_ref().unwrap();
        reconciler
            .evaluate_loop_step(&run, &steps[0], view, spec)
            .unwrap();
        reconciler
            .evaluate_loop_branch(&run, view, spec, keep, "", None, None)
            .unwrap();
        let label = if keep { "keep" } else { "discard" };
        let subject = store.mission_run_subject_for_idempotency_key(&format!(
            "loop-{label}:{}:1:round",
            view.subject
        ));
        let branch = store.mission_run(&subject).unwrap().unwrap();
        store
            .append_claim(&ClaimInput {
                subject: branch.steps[0].subject.clone(),
                kind: "work.progress".into(),
                actor: Some("agent/worker".into()),
                fields: BTreeMap::new(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some("branch-progress".into()),
            })
            .unwrap();
        assert_eq!(check(&store, &run, "agent/free"), "ok");
        assert_eq!(
            check(&store, &run, "agent/worker"),
            "reviewer-not-independent"
        );
        let extra = store.mission_spec("extra", None).unwrap().unwrap();
        let extra = extra.steps["other"].loop_spec.as_ref().unwrap();
        reconciler
            .ensure_loop_child(
                &run,
                view,
                &extra.round,
                "unknown-child".into(),
                1,
                "",
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            check(&store, &run, "agent/free"),
            "reviewer-owner-unresolved"
        );
    }
}

#[test]
fn agent_review_loop_empty_legacy_collection_and_missing_item() {
    for items in [json!([]), json!([{"id":"one"}])] {
        let (store, reconciler, run, mut mission) = fixture();
        mission
            .steps
            .get_mut("cycle")
            .unwrap()
            .loop_spec
            .as_mut()
            .unwrap()
            .for_each = Some(LoopForEachSpec {
            resource: "resource/batch".into(),
            field: "items".into(),
            max_parallel: 1,
        });
        // Restore a historical shape: current KDL deliberately rejects for-each declarations.
        store
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h71aa3a6171e0 owner=fractal_reconcile::reconcile::tests::agent_review_loop_branches TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_revisions SET body=?1 WHERE mission_id=?2 AND revision=?3",
                rusqlite::params![
                    serde_json::to_string(&mission).unwrap(),
                    mission.id,
                    run.revision
                ],
            )
            .unwrap();
        store
            .append_claim(&ClaimInput {
                subject: "resource/batch".into(),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("kind".into(), json!("custom.test.batch")),
                    ("items".into(), items.clone()),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some("items".into()),
            })
            .unwrap();
        let steps = flatten_mission_steps(&mission);
        let spec = steps[0].spec.loop_spec.as_ref().unwrap();
        // Both calls use the real entry-state/snapshot/completion writer; no claims are synthesized.
        reconciler
            .evaluate_loop_step(&run, &steps[0], &run.steps[0], spec)
            .unwrap();
        if items.as_array().unwrap().is_empty() {
            reconciler
                .evaluate_loop_step(&run, &steps[0], &run.steps[0], spec)
                .unwrap();
            assert_eq!(check(&store, &run, "agent/free"), "ok");
        } else {
            assert_eq!(
                check(&store, &run, "agent/free"),
                "reviewer-owner-unresolved"
            );
        }
    }
}
