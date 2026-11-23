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
