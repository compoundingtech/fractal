use super::*;
use crate::model::WorkRequest;

const SOURCE: &str = r#"version 2
mission "review" state="ready" {
  goal "Review one immutable pull request snapshot."
  concurrent-runs max=4
  input "source" kind="resource"
  completion { when "all-steps-exhausted" }
  step "review" { agentless; gate "wait" { document "doc/review-ready" } }
}
resource "repo" { kind "vcs.repository" }
observer "repo" { resource "resource/repo"; provider "github.repository"; locator "acme/garden"; field "pull_requests" }
subscription "reviews" { observer "observer/repo"; on "pull_requests"; delivery "mission" { mission "review"; resource "source"; workspace "/tmp/example-reviews" } }
"#;

const PR: &str = "resource/repo/pull-request/7";

fn observe(store: &Store, head: &str, state: &str) -> String {
    observe_state(store, head, state, false)
}

fn observe_state(store: &Store, head: &str, state: &str, draft: bool) -> String {
    store
        .append_claim(&ClaimInput {
            subject: PR.into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("kind".into(), Value::String("vcs.pull-request".into())),
                (
                    "facts".into(),
                    serde_json::json!({"number":7, "head_sha":head, "state":state, "draft":draft}),
                ),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}

fn request(store: &Store, snapshot: &str, key: &str) -> String {
    request_with_policy(store, snapshot, key, None)
}

fn request_with_policy(store: &Store, snapshot: &str, key: &str, policy: Option<Value>) -> String {
    store
        .append_claim(&ClaimInput {
            subject: "subscription/reviews".into(),
            kind: "subscription.mission-requested".into(),
            actor: None,
            fields: BTreeMap::from([
                ("mission".into(), Value::String("mission/review".into())),
                ("resource".into(), Value::String(PR.into())),
                ("discovery".into(), Value::String(snapshot.into())),
                ("resource_input".into(), Value::String("source".into())),
                (
                    "workspace".into(),
                    Value::String("/tmp/example-reviews".into()),
                ),
                ("requester".into(), Value::String("person/operator".into())),
            ]).into_iter().chain(
                policy.and_then(|value| value.as_object().cloned()).into_iter().flatten(),
            ).collect(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(key.into()),
        })
        .unwrap()
        .id
}

fn reconciler(store: &Arc<Store>) -> Reconciler<FakeRuntime> {
    Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    )
}

fn start(store: &Arc<Store>, snapshot: &str, key: &str) -> MissionRunView {
    request(store, snapshot, key);
    reconciler(store)
        .reconcile_subscription_missions(&store.desired_subjects().unwrap())
        .unwrap();
    let started = store
        .claims_for("subscription/reviews", Some("subscription.mission-started"))
        .unwrap();
    store
        .mission_run(
            started.last().unwrap().body["fields"]["mission_run"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
        .unwrap()
}

#[test]
fn completed_pull_request_snapshot_is_deduplicated_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.db");
    let store = Arc::new(Store::open(&path, "node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let run = start(&store, &snapshot, "first");
    store
        .set_mission_run_state(&run.id, "completed", "terminal", None)
        .unwrap();
    drop(store);

    let store = Arc::new(Store::open(&path, "node").unwrap());
    let duplicate = request(&store, &snapshot, "duplicate");
    let r = reconciler(&store);
    for _ in 0..2 {
        r.reconcile_subscription_missions(&store.desired_subjects().unwrap())
            .unwrap();
    }
    assert_eq!(
        store
            .claims_for("subscription/reviews", Some("subscription.mission-started"))
            .unwrap()
            .len(),
        1
    );
    let cancelled = store
        .claims_for(
            "subscription/reviews",
            Some("subscription.mission-request-cancelled"),
        )
        .unwrap();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].body["fields"]["request"], duplicate);
    assert!(
        cancelled[0].body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("already completed")
    );

    // A fresh observation of the same head is a different immutable snapshot.
    let next = observe(&store, &"a".repeat(40), "open");
    assert_ne!(snapshot, next);
    let next_run = start(&store, &next, "next-snapshot");
    assert_ne!(run.id, next_run.id);
    assert_eq!(
        next_run.inputs["source"].claim_id.as_deref(),
        Some(next.as_str())
    );
}

#[test]
fn completed_pull_request_snapshot_is_deduplicated_after_replication() {
    let remote = Arc::new(Store::open_memory("remote").unwrap());
    apply_source(&remote, SOURCE, "fixture");
    let snapshot = observe(&remote, &"a".repeat(40), "open");
    let mut run_request = MissionRunRequest {
        mission: "review".into(),
        revision: None,
        workspace: "/tmp/example-reviews".into(),
        requester: Some("person/operator".into()),
        mode: None,
        inputs: BTreeMap::from([("source".into(), format!("{PR}@{snapshot}"))]),
        idempotency_key: "first".into(),
    };
    let completed = remote
        .create_subscription_mission_run(&run_request, None, "subscription/reviews", PR, &snapshot, None, None)
        .unwrap();
    remote
        .set_mission_run_state(&completed.id, "completed", "terminal", None)
        .unwrap();

    let local = Arc::new(Store::open_memory("node").unwrap());
    local
        .import_replication("remote", &remote.export_replication(0).unwrap())
        .unwrap();
    assert_eq!(
        local.mission_run(&completed.id).unwrap().unwrap().status,
        "completed"
    );
    run_request.idempotency_key = "local-duplicate".into();
    let error = local
        .create_subscription_mission_run(&run_request, None, "subscription/reviews", PR, &snapshot, None, None)
        .unwrap_err();
    assert_eq!(error.code, "completed-subscription-snapshot");
    assert!(
        local
            .active_mission_runs_for_mission("review")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_pull_request_snapshot_can_be_retried() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let first = start(&store, &snapshot, "first");
    store
        .set_mission_run_state(&first.id, "failed", "terminal", Some("review failed"))
        .unwrap();
    let retry = start(&store, &snapshot, "retry");
    assert_ne!(first.id, retry.id);
}

#[test]
fn superseded_subscription_pull_request_runs_cancel_but_authored_runs_keep_their_input() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let run = start(&store, &snapshot, "first");
    let authored = store
        .create_mission_run(&MissionRunRequest {
            mission: "review".into(),
            revision: None,
            workspace: "/tmp/example-authored".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::from([("source".into(), format!("{PR}@{snapshot}"))]),
            idempotency_key: "authored".into(),
        })
        .unwrap();
    let r = reconciler(&store);
    r.evaluate_mission_runs().unwrap();
    assert_eq!(store.mission_run(&run.id).unwrap().unwrap().phase, "normal");
    let (_, reads) =
        smallclaims::touched::record(|| store.stale_subscription_pull_request_run(&run).unwrap());
    assert!(
        reads.contains(PR),
        "the next observation must wake the run evaluator"
    );
    assert!(reads.contains(&format!("subscription-run:{}", run.subject)));
    assert!(!reads.contains("kind:subscription.mission-started"));

    observe(&store, &"b".repeat(40), "open");
    for _ in 0..5 {
        r.evaluate_mission_runs().unwrap();
    }
    let ended = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(ended.status, "cancelled");
    assert!(ended.steps.iter().all(|step| step.status == "cancelled"));
    assert_eq!(
        ended.inputs["source"].claim_id.as_deref(),
        Some(snapshot.as_str())
    );
    assert_eq!(
        store.mission_run(&authored.id).unwrap().unwrap().status,
        "running"
    );
    let outcomes = store
        .claims_for(&run.subject, Some("mission-run.state"))
        .unwrap();
    assert!(outcomes.iter().any(|c| {
        c.body["fields"]["reason"]
            .as_str()
            .is_some_and(|s| s.contains("moved from head"))
    }));
    assert!(
        !outcomes
            .iter()
            .any(|c| c.body["fields"]["status"] == "failed")
    );
}

#[test]
fn supersession_revokes_a_claimed_worker_and_runs_final_cleanup() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = SOURCE.replace(
        "step \"review\" { agentless; gate \"wait\" { document \"doc/review-ready\" } }",
        "step \"review\" { assigned-to \"agent/node.worker\" }\n  finally { step \"cleanup\" { agentless } }",
    ) + "\nagent \"worker\" { workspace \".\"; command \"true\" }\n";
    apply_source(&store, &source, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let run = start(&store, &snapshot, "first");
    let step = run.steps.iter().find(|step| step.step == "review").unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.observed".into(),
            actor: None,
            fields: BTreeMap::from([
                ("state".into(), Value::String("ready".into())),
                (
                    "incarnation_id".into(),
                    Value::String("worker-incarnation".into()),
                ),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    store.set_step_state(&step.subject, "ready", None).unwrap();
    let work = |key: &str| WorkRequest {
        wait_ms: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some("worker-incarnation".into()),
        summary: None,
        reason: None,
        evidence: vec![],
        idempotency_key: key.into(),
    };
    store
        .work_action(&step.subject, "claim", &work("claim"))
        .unwrap();
    observe(&store, &"b".repeat(40), "open");
    let r = reconciler(&store);
    let (view, mission) = store.mission_run_for_evaluation(&run.id).unwrap();
    r.evaluate_active_mission_run(&view, mission)
        .unwrap();
    let cancelled = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(cancelled.phase, "final-cancelled");
    let cancelled_step = cancelled
        .steps
        .iter()
        .find(|step| step.step == "review")
        .unwrap();
    assert_eq!(cancelled_step.status, "cancelled");
    assert!(cancelled_step.claimant.is_none());
    assert!(cancelled_step.claim_incarnation.is_none());
    assert!(cancelled_step.claim_expires_at_unix_ms.is_none());
    assert_eq!(
        cancelled
            .steps
            .iter()
            .find(|step| step.step == "cleanup")
            .unwrap()
            .status,
        "pending"
    );
    assert!(
        store
            .work_action(&step.subject, "complete", &work("late-complete"))
            .is_err()
    );
    let notices = store.messages(None, true).unwrap();
    assert!(
        notices
            .iter()
            .any(|message| message.title.as_deref() == Some("Mission work cancelled"))
    );
    for _ in 0..5 {
        r.evaluate_mission_runs().unwrap();
    }
    let ended = store.mission_run(&run.id).unwrap().unwrap();
    assert_eq!(ended.status, "cancelled");
    assert_eq!(
        ended
            .steps
            .iter()
            .find(|step| step.step == "cleanup")
            .unwrap()
            .status,
        "completed"
    );
}

#[test]
fn run_creation_rechecks_a_pull_request_that_moved_after_queue_inspection() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    observe(&store, &"b".repeat(40), "open");
    let error = store
        .create_subscription_mission_run(
            &MissionRunRequest {
                mission: "review".into(),
                revision: None,
                workspace: "/tmp/example-reviews".into(),
                requester: Some("person/operator".into()),
                mode: None,
                inputs: BTreeMap::from([("source".into(), format!("{PR}@{snapshot}"))]),
                idempotency_key: "stale-creation".into(),
            },
            None,
            "subscription/reviews",
            PR,
            &snapshot,
            None,
            None,
        )
        .unwrap_err();
    assert_eq!(error.code, "stale-pull-request");
    assert!(
        store
            .active_mission_runs_for_mission("review")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn review_goal_product_and_contract_gates_share_the_same_run_context() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    apply_source(
        &store,
        r#"version 2
mission "contract" state="ready" {
  goal "Verify the review artifact contract."
  input "source" kind="resource"
  completion { when "all-steps-exhausted" }
  step "inspect" {
    agentless
    goal "Publish resource/mission-run/${ST_MISSION_RUN}/review for ${input.source}."
    produces { resource "mission-run/${ST_MISSION_RUN}/review" { kind "human.review" } }
    gate "target" { field "target" "resource/mission-run/${ST_MISSION_RUN}/review" "is" "${input.source}" }
    gate "document" { field "document" "resource/mission-run/${ST_MISSION_RUN}/review" "contains" "mission-run/${ST_MISSION_RUN}/review@" }
  }
}"#,
        "contract",
    );
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let target = format!("{PR}@{snapshot}");
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "contract".into(),
            revision: None,
            workspace: "/tmp/example-contract".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::from([("source".into(), target.clone())]),
            idempotency_key: "contract-run".into(),
        })
        .unwrap();
    assert_eq!(
        run.steps[0].goals,
        [format!(
            "Publish resource/mission-run/{}/review for {target}.",
            run.id
        )]
    );
    let write = |document: String| {
        store
            .append_claim(&ClaimInput {
                subject: format!("resource/mission-run/{}/review", run.id),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("kind".into(), Value::String("human.review".into())),
                    ("target".into(), Value::String(target.clone())),
                    ("document".into(), Value::String(document)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    };
    let r = reconciler(&store);
    write(format!(
        "doc/mission-run/another-round/review@{}",
        "a".repeat(64)
    ));
    for _ in 0..3 {
        r.evaluate_mission_runs().unwrap();
    }
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().status,
        "running"
    );
    write(format!(
        "doc/mission-run/{}/review@{}",
        run.id,
        "b".repeat(64)
    ));
    for _ in 0..5 {
        r.evaluate_mission_runs().unwrap();
    }
    assert_eq!(
        store.mission_run(&run.id).unwrap().unwrap().status,
        "completed"
    );
}

#[test]
fn request_snapshot_admission_controls_draft_and_closed_eligibility() {
    for (state, draft) in [("open", true), ("closed", false), ("merged", false)] {
        for relevant in [false, true] {
            let store = Arc::new(Store::open_memory("node").unwrap());
            // Deliberately leave the current declaration at ready-heads: requests own policy.
            apply_source(&store, SOURCE, "fixture");
            let snapshot = observe_state(&store, &"a".repeat(40), state, draft);
            request_with_policy(&store, &snapshot, "request", relevant.then(|| {
                serde_json::json!({"admission":"relevant-changes", "authors":["orchid-bot"]})
            }));
            reconciler(&store)
                .reconcile_subscription_missions(&store.desired_subjects().unwrap())
                .unwrap();
            let started = store.claims_for("subscription/reviews", Some("subscription.mission-started")).unwrap();
            let cancelled = store.claims_for("subscription/reviews", Some("subscription.mission-request-cancelled")).unwrap();
            assert_eq!(started.len(), usize::from(relevant), "{state}, draft={draft}");
            assert_eq!(cancelled.len(), usize::from(!relevant), "{state}, draft={draft}");
            if relevant {
                let run = store.mission_run(started[0].body["fields"]["mission_run"].as_str().unwrap()).unwrap().unwrap();
                let created = store.latest_claim(&run.subject, Some("mission-run.created")).unwrap().unwrap();
                assert_eq!(created.body["fields"]["admission"], "relevant-changes");
                assert_eq!(created.body["fields"]["authors"], serde_json::json!(["orchid-bot"]));
                assert!(store.stale_subscription_pull_request_run(&run).unwrap().is_none());
            }
        }
    }
}

#[test]
fn relevant_changes_run_policy_survives_restart_and_replication_but_not_a_head_move() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.db");
    let store = Arc::new(Store::open(&path, "node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    request_with_policy(&store, &snapshot, "relevant", Some(serde_json::json!({"admission":"relevant-changes"})));
    reconciler(&store).reconcile_subscription_missions(&store.desired_subjects().unwrap()).unwrap();
    let run = store.active_mission_runs_for_mission("review").unwrap().pop().unwrap();
    observe_state(&store, &"a".repeat(40), "closed", true);
    assert!(store.stale_subscription_pull_request_run(&run).unwrap().is_none());
    drop(store);

    let store = Arc::new(Store::open(&path, "node").unwrap());
    let run = store.mission_run(&run.id).unwrap().unwrap();
    assert!(store.stale_subscription_pull_request_run(&run).unwrap().is_none());
    let replica = Store::open_memory("replica").unwrap();
    replica.import_replication("node", &store.export_replication(0).unwrap()).unwrap();
    let replicated_run = replica.mission_run(&run.id).unwrap().unwrap();
    assert!(replica.stale_subscription_pull_request_run(&replicated_run).unwrap().is_none());
    observe(&replica, &"b".repeat(40), "closed");
    assert!(replica.stale_subscription_pull_request_run(&replicated_run).unwrap().unwrap().contains("moved from head"));
}

#[test]
fn relevant_changes_writer_rechecks_head_and_snapshots_policy_in_idempotency() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe_state(&store, &"a".repeat(40), "closed", true);
    let policy = serde_json::json!({"admission":"relevant-changes", "authors":["orchid-bot"]});
    let request = MissionRunRequest {
        mission: "review".into(), revision: None, workspace: "/tmp/example-reviews".into(),
        requester: Some("person/operator".into()), mode: None,
        inputs: BTreeMap::from([("source".into(), format!("{PR}@{snapshot}"))]),
        idempotency_key: "writer-policy".into(),
    };
    let run = store.create_subscription_mission_run(&request, None, "subscription/reviews", PR, &snapshot, Some(&policy), None).unwrap();
    let created = store.latest_claim(&run.subject, Some("mission-run.created")).unwrap().unwrap();
    assert_eq!(created.body["fields"]["authors"], policy["authors"]);
    assert!(store.create_subscription_mission_run(&request, None, "subscription/reviews", PR, &snapshot, None, None).is_err());
    observe(&store, &"b".repeat(40), "closed");
    let stale_request = MissionRunRequest { idempotency_key: "stale-relevant".into(), ..request };
    let error = store.create_subscription_mission_run(&stale_request, None, "subscription/reviews", PR, &snapshot, Some(&policy), None).unwrap_err();
    assert_eq!(error.code, "stale-pull-request");
}

#[test]
fn relevant_changes_writer_rejects_same_head_supersession_but_replays_created_runs() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, SOURCE, "fixture");
    let snapshot = observe(&store, &"a".repeat(40), "open");
    let policy = serde_json::json!({"admission":"relevant-changes", "authors":["orchid-bot"]});
    let original = request_with_policy(&store, &snapshot, "original", Some(policy.clone()));
    let selected = store.pending_subscription_mission_requests("subscription/reviews").unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, original);
    let item = store.desired_subjects().unwrap().into_iter()
        .find(|item| item.subject == "subscription/reviews").unwrap();
    let mut newest = String::new();
    let mut latest_snapshot = String::new();
    let mut attempted = false;
    reconciler(&store).reconcile_subscription_mission_with_start(
        &item, &[], |run_request, parent, subscription, resource, discovery, fields, request_id| {
            attempted = true;
            assert_eq!(request_id, original);
            // A new immutable snapshot with the same head lands after selection.
            latest_snapshot = observe_state(&store, &"a".repeat(40), "closed", true);
            newest = request_with_policy(&store, &latest_snapshot, "newest", Some(policy.clone()));
            let error = store.create_subscription_mission_run(
                run_request, parent, subscription, resource, discovery, Some(fields), Some(request_id),
            ).unwrap_err();
            assert_eq!(error.code, "stale-pull-request");
            Err(error)
        },
    ).unwrap();
    assert!(attempted);
    assert!(store.active_mission_runs_for_mission("review").unwrap().is_empty());
    let cancelled = store.claims_for("subscription/reviews", Some("subscription.mission-request-cancelled")).unwrap();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].body["fields"]["request"], original);

    let mut admitted = None;
    reconciler(&store).reconcile_subscription_mission_with_start(
        &item, &[], |run_request, parent, subscription, resource, discovery, fields, request_id| {
            assert_eq!(request_id, newest);
            assert_eq!(discovery, latest_snapshot);
            let run = store.create_subscription_mission_run(
                run_request, parent, subscription, resource, discovery, Some(fields), Some(request_id),
            )?;
            admitted = Some((run_request.clone(), run.clone()));
            Ok(run)
        },
    ).unwrap();
    let (run_request, run) = admitted.unwrap();
    let next_snapshot = observe(&store, &"a".repeat(40), "open");
    request_with_policy(&store, &next_snapshot, "after-start", Some(policy.clone()));
    let replayed = store.create_subscription_mission_run(
        &run_request, None, "subscription/reviews", PR, &latest_snapshot, Some(&policy), Some(&newest),
    ).unwrap();
    assert_eq!(replayed.id, run.id);
    assert_eq!(store.active_mission_runs_for_mission("review").unwrap().len(), 1);
}

#[test]
fn replicated_relevant_requests_coalesce_without_resurrecting_superseded_work() {
    let left = Store::open_memory("left").unwrap();
    apply_source(&left, SOURCE, "fixture");
    let snapshot = observe(&left, &"a".repeat(40), "open");
    let right = Store::open_memory("right").unwrap();
    right.import_replication("left", &left.export_replication(0).unwrap()).unwrap();
    let policy = || Some(serde_json::json!({"admission":"relevant-changes"}));
    request_with_policy(&left, &snapshot, "left-request", policy());
    request_with_policy(&right, &snapshot, "right-request", policy());
    let left_export = left.export_replication(0).unwrap();
    let right_export = right.export_replication(0).unwrap();
    left.import_replication("right", &right_export).unwrap();
    right.import_replication("left", &left_export).unwrap();
    let selected = left.pending_subscription_mission_requests("subscription/reviews").unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(right.pending_subscription_mission_requests("subscription/reviews").unwrap()[0].id, selected[0].id);
    let views = left.subscription_requests("subscription/reviews").unwrap();
    assert_eq!(views.iter().filter(|view| view.status == "cancelled").count(), 1);
    left.append_claim(&ClaimInput {
        subject: "subscription/reviews".into(), kind: "subscription.mission-started".into(), actor: None,
        fields: BTreeMap::from([
            ("request".into(), Value::String(selected[0].id.clone())),
            ("mission_run".into(), Value::String("mission-run/selected".into())),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(left.pending_subscription_mission_requests("subscription/reviews").unwrap().is_empty());
    request(&left, &snapshot, "default-one");
    request(&left, &snapshot, "default-two");
    assert_eq!(left.pending_subscription_mission_requests("subscription/reviews").unwrap().len(), 2);
}

#[test]
fn default_run_lineage_omits_policy_and_keeps_ready_head_fences() {
    for (state, draft) in [("open", true), ("closed", false)] {
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(&store, SOURCE, "fixture");
        let snapshot = observe(&store, &"a".repeat(40), "open");
        let run = start(&store, &snapshot, "ready-head");
        let created = store.latest_claim(&run.subject, Some("mission-run.created")).unwrap().unwrap();
        assert!(created.body["fields"].get("admission").is_none());
        assert!(created.body["fields"].get("authors").is_none());
        observe_state(&store, &"a".repeat(40), state, draft);
        assert!(store.stale_subscription_pull_request_run(&run).unwrap().is_some());
    }
}
