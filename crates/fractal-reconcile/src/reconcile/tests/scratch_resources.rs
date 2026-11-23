//! Mission-native scratch finalization, independent of JavaScript scope teardown.
use super::*;
use crate::store::scratch_resources::{OwnerObservation, ScratchRequest};

fn lifecycle_with_owner_tree(outcome: &str, deadline: bool, pending: bool) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
resource "scratch-wait" { kind "custom.st3.document-source" }
mission "scratch-lifetime" state="ready" timeout="1h" {
 goal "Keep scratch until native final work."
 step "work" {
  agentless
  gate "wait" { field "state" "resource/scratch-wait" is "ready" }
 }
 finally { step "final-work" { agentless } }
}"#, "scratch-lifetime-definition");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "scratch-lifetime".into(), revision: None,
        workspace: root.path().display().to_string(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: format!("scratch-{outcome}-{deadline}"),
    }).unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    if pending { *runtime.resource_retirement_pending.lock().unwrap() = usize::MAX; }
    let reconciler = Reconciler::new(store.clone(), runtime.clone(),
        "node".into(), Arc::new(Notify::new()));
    reconciler.ensure_resource_authority().unwrap();
    let handle = store.acquire_scratch(&ScratchRequest {
        id: format!("program:{}:1:activity/resource.scratch/output", run.steps[0].subject),
        host: "node".into(), owner: run.subject.clone(), owner_generation: run.generation.clone(),
        root: root.path().canonicalize().unwrap().join("dedicated").display().to_string(),
    }).unwrap();
    let lane = crate::store::resource_membership::LaneRequest {
        profile: None, profile_estimate: None, disk_path: None,
        id: "owner-tree".into(), name: "owner-tree".into(), host: "node".into(),
        max_members: 1, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1 << 30 }),
        coverage: crate::store::resource_membership::Coverage::Logical, admission_timeout_ms: None,
        class: fractal_runtime::RequestClass::Agent, disk: None, io: None,
        owner: crate::store::resource_membership::Owner {
            run: run.subject.clone(), step: run.steps[0].subject.clone(),
            attempt: run.steps[0].attempt, invocation: Some("scratch-owner-tree".into()),
        },
    };
    assert!(matches!(store.request_resource_membership(&lane,
        crate::store::resource_membership::Fit {
            room: Some(fractal_runtime::Capacity { cores: 32, memory: 128 << 30 }),
            ..Default::default()
        }, 1).unwrap(), crate::store::resource_membership::Admission::Admitted(_)));
    std::fs::write(std::path::Path::new(&handle.path).join("result"), "owned data").unwrap();
    assert!(std::path::Path::new(&handle.path).exists());
    if deadline {
        let expired = run.deadline_at_unix_ms.unwrap() + 1;
        smallclaims::store::set_thread_clock(Some(expired));
        store.set_write_clock_at(expired).unwrap();
    } else if outcome == "cancelled" {
        store.request_mission_run_cancellation(&run.subject, "cancel work").unwrap();
    } else {
        let step = run.steps.iter().find(|step| step.step == "work").unwrap();
        store.set_step_state(&step.subject, outcome, None).unwrap();
    }
    for _ in 0..16 {
        reconciler.reconcile_once().unwrap();
        if store.mission_run(&run.subject).unwrap().unwrap().phase == "terminal" { break; }
    }
    smallclaims::store::set_thread_clock(None);
    let ended = store.mission_run(&run.subject).unwrap().unwrap();
    assert_eq!(ended.phase, "terminal", "{ended:?}");
    assert_eq!(ended.status, if deadline { "failed" } else { outcome });
    let rows = store.scratch_inventory("node", None, 100).unwrap();
    let row = rows.iter().find(|row| row.handle.id == handle.id).unwrap();
    assert_eq!(row.owner_observation, OwnerObservation::Terminal);
    let released = cfg!(target_os = "linux") && !deadline && !pending;
    assert_eq!(row.state, if released { "released" } else { "held" });
    assert_eq!(std::path::Path::new(&handle.path).exists(), !released);
    if !deadline {
        assert!(runtime.resource_retirement_calls.load(Ordering::Relaxed) > 0);
    }
    if deadline {
        assert_ne!(ended.steps.iter().find(|step| step.step == "final-work").unwrap().status, "completed");
        // Inventory is read-only even after the exact owner is terminal.
        store.scratch_inventory("node", None, 1).unwrap();
        assert!(std::path::Path::new(&handle.path).exists());
    }
}

fn lifecycle(outcome: &str, deadline: bool) { lifecycle_with_owner_tree(outcome, deadline, false); }

#[test]
fn unconfirmed_owner_tree_retirement_retains_scratch() {
    lifecycle_with_owner_tree("completed", false, true);
}

#[test]
fn payload_exit_without_exact_native_scope_settlement_is_not_tree_retirement() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(),
        "node".into(), Arc::new(Notify::new()));
    store.append_claim(&ClaimInput {
        subject: "agent/scratch-owned-runtime".into(), kind: "runtime.observed".into(), actor: None,
        fields: BTreeMap::from([
            ("status".into(), Value::String("exited".into())),
            ("runtime_id".into(), Value::String("owned-exec".into())),
            ("incarnation_id".into(), Value::String("exact-generation".into())),
            ("terminal".into(), Value::Bool(false)),
            ("host".into(), Value::String("node".into())),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert!(!reconciler.scratch_runtime_tree_retired("agent/scratch-owned-runtime").unwrap());
    assert!(!store.runtime_cleanup_settled("owned-exec", "exact-generation").unwrap());
    runtime.settled_leftovers.lock().unwrap().insert(("owned-exec".into(), "exact-generation".into()));
    assert!(reconciler.scratch_runtime_tree_retired("agent/scratch-owned-runtime").unwrap());
    assert!(store.runtime_cleanup_settled("owned-exec", "exact-generation").unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn restarted_owner_retains_scratch_until_earlier_descendant_scope_retires() {
    struct Descendant(std::process::Child);
    impl Drop for Descendant {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, r#"version 2
agent "scratch-history" { command "true" }
mission "scratch-history" state="ready" timeout="1h" {
 goal "Retain scratch until every owner incarnation retires."
 step "work" { agentless }
}"#, "scratch-history-definition");
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "scratch-history".into(), revision: None,
        workspace: root.path().display().to_string(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: "scratch-history-run".into(),
    }).unwrap();
    let runtime = Arc::new(FakeRuntime::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(),
        "node".into(), Arc::new(Notify::new()));
    reconciler.ensure_resource_authority().unwrap();
    let handle = store.acquire_scratch(&ScratchRequest {
        id: "scratch-history-output".into(), host: "node".into(),
        owner: run.subject.clone(), owner_generation: run.generation.clone(),
        root: root.path().canonicalize().unwrap().join("dedicated").display().to_string(),
    }).unwrap();
    std::fs::write(std::path::Path::new(&handle.path).join("result"), "owned data").unwrap();
    // This predecessor descendant has neither cwd nor descriptors in scratch,
    // but can still reach its owner's allocation by pathname after restart.
    let mut descendant = Descendant(std::process::Command::new("/bin/sh")
        .args(["-c", "read stop; test -f \"$1/result\"", "scratch-descendant", &handle.path])
        .current_dir(root.path())
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).spawn().unwrap());
    let subject = "agent/scratch-history";
    for (incarnation, status) in [
        ("previous", "running"), ("previous", "stopped"),
        ("current", "running"), ("current", "exited"),
    ] {
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!(status)), ("runtime_id".into(), json!("owned-exec")),
                ("incarnation_id".into(), json!(incarnation)),
                ("terminal".into(), json!(false)), ("host".into(), json!("node")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    runtime.settled_leftovers.lock().unwrap().insert(("owned-exec".into(), "current".into()));
    assert!(store.runtime_execution_quiescent(subject).unwrap());
    assert!(!reconciler.scratch_runtime_tree_retired(subject).unwrap());
    assert!(store.runtime_cleanup_settled("owned-exec", "current").unwrap());
    let owned = store.desired_subjects().unwrap().into_iter()
        .filter(|desired| desired.subject == subject).collect::<Vec<_>>();
    assert_eq!(owned.len(), 1);
    reconciler.finalize_mission_scratch(&run, &owned).unwrap();
    assert!(std::path::Path::new(&handle.path).join("result").exists());
    assert_eq!(store.scratch_inventory("node", None, 100).unwrap()[0].state, "held");
    std::io::Write::write_all(descendant.0.stdin.as_mut().unwrap(), b"go\n").unwrap();
    assert!(descendant.0.wait().unwrap().success(), "old descendant still reaches scratch by pathname");
    store.settle_runtime_cleanup(subject, "owned-exec", "previous").unwrap();
    assert!(reconciler.scratch_runtime_tree_retired(subject).unwrap());
    reconciler.finalize_mission_scratch(&run, &owned).unwrap();
    assert!(!std::path::Path::new(&handle.path).exists());
}

#[cfg(target_os = "linux")]
#[test]
fn native_success_releases_mission_scratch() { lifecycle("completed", false); }

#[cfg(target_os = "linux")]
#[test]
fn native_failure_releases_mission_scratch() { lifecycle("failed", false); }

#[cfg(target_os = "linux")]
#[test]
fn native_cancellation_releases_mission_scratch() { lifecycle("cancelled", false); }

#[test]
fn native_deadline_skips_scratch_final_work_and_reports_terminal_owner() { lifecycle("failed", true); }

#[cfg(not(target_os = "linux"))]
#[test]
fn native_outcomes_retain_physical_scratch_on_unsupported_platforms() {
    for outcome in ["completed", "failed", "cancelled"] { lifecycle(outcome, false); }
}
