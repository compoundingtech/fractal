use super::super::{FileChanges, FileWatch};
use crate::incremental::Incremental;
use futures_util::FutureExt as _;
use notify::{Event, EventKind, event::{ModifyKind, RemoveKind, RenameMode}};
use std::{collections::BTreeSet, path::Path};
use tokio::sync::Notify;

#[test]
fn external_inputs_notify_only_their_readers() {
    let incremental = Incremental::default();
    let changes = FileChanges::default();
    let global = Notify::new();
    for index in 0..64 {
        incremental.evaluated(&format!("mission-run/{index}"),
            BTreeSet::from([format!("file/input-{index}")]), None);
    }
    changes.invalidate(&incremental, "file/unrelated", false, &global);
    assert!(global.notified().now_or_never().is_none(), "zero global notifications");
    assert!(changes.notify.notified().now_or_never().is_none(), "zero scoped notifications");
    assert!(changes.runs.lock().unwrap().is_empty());

    changes.invalidate(&incremental, "file/input-7", false, &global);
    assert!(global.notified().now_or_never().is_none(), "file gates do not wake the common loop");
    assert!(changes.notify.notified().now_or_never().is_some());
    assert_eq!(*changes.runs.lock().unwrap(), BTreeSet::from(["mission-run/7".to_owned()]));

    incremental.evaluated("launch-inputs:agent/example", BTreeSet::from(["launch-input/tool".into()]), None);
    changes.invalidate(&incremental, "launch-input/tool", false, &global);
    assert!(global.notified().now_or_never().is_none(), "launch identity stays scoped");
    assert_eq!(*changes.launches.lock().unwrap(), BTreeSet::from(["agent/example".to_owned()]));
    changes.invalidate(&incremental, "file/unrelated", true, &global);
    assert!(global.notified().now_or_never().is_some(), "genuine lost-path rescans fail closed");
}

#[test]
fn subtree_rescans_keep_gate_and_launch_readers_scoped() {
    use super::super::file_subscriptions;
    let incremental = Incremental::default();
    let changes = FileChanges::default();
    let global = Notify::new();
    incremental.evaluated("mission-run/gate", BTreeSet::from(["file/input".into()]), None);
    incremental.evaluated("launch-inputs:agent/tool", BTreeSet::from(["launch-input/tool".into()]), None);
    let event = Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)
        .set_info("rescan: subtree").add_path("/workspace/input".into());
    for key in ["file/input", "launch-input/tool"] {
        changes.invalidate(&incremental, key, file_subscriptions::global_rescan(&event), &global);
    }
    assert!(global.notified().now_or_never().is_none());
    assert_eq!(*changes.runs.lock().unwrap(), BTreeSet::from(["mission-run/gate".into()]));
    assert_eq!(*changes.launches.lock().unwrap(), BTreeSet::from(["agent/tool".into()]));
    let loss = event.set_info("rescan: kernel dropped");
    changes.invalidate(&incremental, "file/input", file_subscriptions::global_rescan(&loss), &global);
    assert!(global.notified().now_or_never().is_some(), "genuine loss must wake the full pass");
}

#[test]
fn relevance_preserves_any_path_referents_and_ancestor_replacements() {
    let target = Path::new("/example/inputs/tool");
    let directory = target.parent().unwrap();
    let referent = Path::new("/example/referents/tool");
    let relevant = |event: &Event| FileWatch::relevant(event, target, directory, Some(referent));
    let modification = || Event::new(EventKind::Modify(ModifyKind::Any));
    assert!(!relevant(&modification().add_path(directory.join("unrelated"))));
    assert!(relevant(&modification().add_path(directory.join("unrelated")).add_path(referent.to_owned())));
    assert!(relevant(&modification().add_path(target.to_owned())));
    for kind in [EventKind::Remove(RemoveKind::Folder), EventKind::Modify(ModifyKind::Name(RenameMode::Both))] {
        assert!(relevant(&Event::new(kind).add_path(Path::new("/example").to_owned())));
        assert!(relevant(&Event::new(kind).add_path(referent.parent().unwrap().to_owned())));
    }
    assert!(!relevant(&Event::new(EventKind::Access(notify::event::AccessKind::Any)).add_path(target.to_owned())));
    assert!(relevant(&Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)));
    assert!(relevant(&modification()), "pathless mutations fail closed");
}

#[test]
fn launch_input_change_evaluates_only_its_declaration() {
    use super::{FakeRuntime, parse_intent};
    use super::super::Reconciler;
    use crate::store::Store;
    use crate::SeatsStoreCore as _;
    use std::sync::Arc;

    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = (0..64).fold(String::from("version 2\n"), |mut source, index| {
        source.push_str(&format!("agent \"scoped/{index}\" {{ command \"true\"; }}\n"));
        source
    });
    let mut intent = parse_intent(&source, "node").unwrap();
    for subject in intent.subjects.values_mut() {
        subject.member.as_mut().unwrap().lifecycle = crate::model::MemberLifecycle::AdoptOnly;
    }
    store.apply_internal(&intent, "scoped-launch-inputs").unwrap();
    let global = Arc::new(Notify::new());
    let reconciler = Reconciler::new(store, Arc::new(FakeRuntime::default()),
        "node".into(), global.clone());
    for index in 0..64 {
        reconciler.incremental.evaluated(&format!("launch-inputs:agent/scoped/{index}"),
            BTreeSet::from([format!("launch-input/tool-{index}")]), None);
    }
    reconciler.file_changes.invalidate(&reconciler.incremental, "launch-input/unrelated", false, &global);
    assert_eq!(reconciler.evaluate_changed_launch_inputs().unwrap(), 0);
    reconciler.file_changes.invalidate(&reconciler.incremental, "launch-input/tool-7", false, &global);
    assert_eq!(*reconciler.file_changes.launches.lock().unwrap(),
        BTreeSet::from(["agent/scoped/7".to_owned()]));
    assert!(global.notified().now_or_never().is_none(), "file invalidation stays scoped");
    *reconciler.start_stage_finished_hook.lock().unwrap() = Some(Box::new(|| {
        panic!("scoped launch evaluation must not enter a full pass");
    }));
    assert_eq!(reconciler.evaluate_changed_launch_inputs().unwrap(), 1);
    assert!((0..64).filter(|index| *index != 7).all(|index|
        !reconciler.incremental.needs(&format!("launch-inputs:agent/scoped/{index}"), 0)));
    // A selected member's real observation may publish semantic state and use its
    // normal lifecycle wake. File selection itself above produces no global wake.
}

#[tokio::test]
async fn repaired_launch_dependency_retries_only_its_guarded_member() {
    use super::{FakeRuntime, apply_source};
    use super::super::Reconciler;
    use crate::store::Store;
    use std::{os::unix::fs::PermissionsExt as _, sync::Arc};

    let root = tempfile::tempdir().unwrap();
    let dependency = root.path().join("provider");
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(&store, &format!(r#"version 2
agent "dependent" {{ workspace {:?}; argv {:?}; }}
agent "unrelated" {{ workspace {:?}; command "true"; }}
"#, root.path().display().to_string(), dependency.display().to_string(),
        root.path().display().to_string()), "repair-launch");
    let runtime = Arc::new(FakeRuntime::default());
    let global = Arc::new(Notify::new());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), global.clone());
    let desired = store.desired_subject_with_writer("agent/node.dependent").unwrap().unwrap().0;
    // FakeRuntime supplies synthetic executable identity, so represent the native
    // missing-provider refusal with its existing physical-start failure control.
    let runtime_id = desired.member.as_ref().unwrap().runtime_id.clone();
    runtime.failed_starts.lock().unwrap().insert(runtime_id.clone());
    assert!(crate::contract::declared(&store, &desired).is_err());
    reconciler.watch_launch_inputs(&desired).unwrap();
    let key = format!("launch-input/{}", dependency.display());
    assert!(reconciler.file_watchers.lock().unwrap().contains_key(&key));
    reconciler.file_changes.invalidate(&reconciler.incremental, &key, false, &global);
    assert!(global.notified().now_or_never().is_none(),
        "file selection must not notify a full pass");
    assert_eq!(reconciler.evaluate_changed_launch_inputs().unwrap(), 1);
    assert!(runtime.started_members.lock().unwrap().is_empty(),
        "a missing-provider refusal must not produce a successful member");
    assert!(store.member_reconcile_fault(&desired.subject, None).unwrap().is_some());
    assert_eq!(runtime.starts.lock().unwrap().as_slice(), std::slice::from_ref(&runtime_id),
        "only the dependent member attempted admission");

    std::fs::write(&dependency, "fixture executable identity").unwrap();
    std::fs::set_permissions(&dependency, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Settle real identity measurement, not the member evaluation under test.
    crate::contract::declared(&store, &desired).unwrap();
    runtime.failed_starts.lock().unwrap().remove(&runtime_id);
    struct ResetClock;
    impl Drop for ResetClock {
        fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
    }
    let _clock = ResetClock;
    let failed = store.observations_for(&desired.subject, "runtime.action.failed").unwrap();
    let retry_at = failed.last().expect("the native start refusal is recorded")
        .accepted_at_unix_ms + 15_001;
    // Preserve the normal failed-start admission backoff, without waiting on wall time.
    smallclaims::store::set_thread_clock(Some(retry_at));
    reconciler.file_changes.invalidate(&reconciler.incremental, &key, false, &global);
    *reconciler.start_stage_finished_hook.lock().unwrap() = Some(Box::new(|| {
        panic!("a launch-input repair must not run a full reconcile pass");
    }));
    assert_eq!(reconciler.evaluate_changed_launch_inputs().unwrap(), 1);
    let started = runtime.started_members.lock().unwrap();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].runtime_id, desired.member.as_ref().unwrap().runtime_id);
    assert_eq!(*runtime.starts.lock().unwrap(), [runtime_id.clone(), runtime_id],
        "the repaired input must retry only its dependent member");
    assert!(store.member_reconcile_fault(&desired.subject, None).unwrap().is_none());
    assert!(store.latest_actual_value("agent/node.unrelated").unwrap().is_none());
    assert!(store.member_reconcile_fault("agent/node.unrelated", None).unwrap().is_none());
    // A real dispatched start retains its existing durable-runtime lifecycle wake.
    // The hook above proves that admission itself did not execute a full pass.
}

#[tokio::test]
async fn missing_nested_watch_does_not_collapse_to_first_component() {
    use super::FakeRuntime;
    use super::super::{Reconciler, now_ms};
    use crate::store::Store;
    use std::{sync::Arc, time::Duration};

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("outer/inner/tool");
    let key = format!("launch-input/{}", target.display());
    let global = Arc::new(Notify::new());
    let reconciler = Reconciler::new(Arc::new(Store::open_memory("node").unwrap()),
        Arc::new(FakeRuntime::default()), "node".into(), global.clone());
    reconciler.incremental.evaluated("launch-inputs:agent/selected",
        BTreeSet::from([key.clone()]), None);
    reconciler.ensure_file_watch(&key, &target).unwrap();
    std::fs::create_dir(root.path().join("outer")).unwrap();
    let sibling = root.path().join("outer/unrelated");
    std::fs::write(&sibling, "existing sibling").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !reconciler.incremental.needs("launch-inputs:agent/selected", now_ms()) {
            reconciler.file_changes.notify.notified().await;
        }
    }).await.unwrap();
    while tokio::time::timeout(Duration::from_millis(100),
        reconciler.file_changes.notify.notified()).await.is_ok() {}
    reconciler.file_changes.launches.lock().unwrap().clear();
    reconciler.incremental.evaluated("launch-inputs:agent/selected",
        BTreeSet::from([key.clone()]), None);
    // Keep the original ancestor subscription: its relevance target must retain
    // inner/tool, rather than treating every descendant of outer as relevant.
    std::fs::write(&sibling, "unrelated sibling changed").unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(250),
        reconciler.file_changes.notify.notified()).await.is_err());
    assert!(!reconciler.incremental.needs("launch-inputs:agent/selected", now_ms()));
    assert!(reconciler.file_changes.launches.lock().unwrap().is_empty());
    // The scoped scheduler refreshes coverage after an ancestor invalidation.
    reconciler.ensure_file_watch(&key, &target).unwrap();
    std::fs::create_dir(root.path().join("outer/inner")).unwrap();
    tokio::time::timeout(Duration::from_secs(5),
        reconciler.file_changes.notify.notified()).await.unwrap();
    reconciler.ensure_file_watch(&key, &target).unwrap();
    while tokio::time::timeout(Duration::from_millis(100),
        reconciler.file_changes.notify.notified()).await.is_ok() {}
    reconciler.incremental.evaluated("launch-inputs:agent/selected",
        BTreeSet::from([key.clone()]), None);
    std::fs::write(&target, "repaired").unwrap();
    tokio::time::timeout(Duration::from_secs(5),
        reconciler.file_changes.notify.notified()).await.unwrap();
    assert!(reconciler.incremental.needs("launch-inputs:agent/selected", now_ms()));
    assert!(global.notified().now_or_never().is_none());
}

#[test]
fn scoped_render_ignores_lifecycle_inactive_conflicting_owners() {
    use super::*;

    for lifecycle in ["terminal-owner", "pending-owner", "stopped-root-task"] {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(&store, r#"version 2
mission "roster-owner" state="ready" {
  goal "Own the declaration."
  step "work" { assigned-to "agent/node.selected" }
}
"#, &format!("mission-{lifecycle}"));
        let run = store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "roster-owner".into(), revision: None,
            workspace: root.path().display().to_string(),
            requester: Some("person/test".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: format!("run-{lifecycle}"),
        }).unwrap();
        let stale_kind = if lifecycle == "stopped-root-task" { "exec" } else { "agent" };
        let source = format!(r#"version 2
agent "selected" {{
  workspace {workspace:?}
  command "true"
  render {{ file "shared.txt" "selected" }}
}}
{stale_kind} "stale" {{
  workspace {workspace:?}
  command "true"
  render {{ file "shared.txt" "stale" }}
}}
"#, workspace = root.path().display().to_string());
        let mut intent = parse_intent(&source, "node").unwrap();
        for subject in intent.subjects.values_mut() {
            subject.member.as_mut().unwrap().lifecycle = crate::model::MemberLifecycle::AdoptOnly;
        }
        let stale = intent.subjects.values_mut()
            .find(|subject| subject.subject != "agent/node.selected").unwrap();
        let stale_subject = stale.subject.clone();
        if lifecycle == "stopped-root-task" {
            stale.member.as_mut().unwrap().tags.insert("st3.agent".into(), "agent/node.root".into());
        } else {
            stale.owner_run = Some(run.subject.clone());
        }
        store.apply_internal(&intent, &format!("members-{lifecycle}")).unwrap();
        if lifecycle == "stopped-root-task" {
            apply_source(&store, "version 2\nstop \"agent/node.root\"\n", "stop-root");
        } else if lifecycle == "terminal-owner" {
            store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
            assert!(store.terminal_owned_runtime_subjects().unwrap().contains(&stale_subject));
        } else {
            // Retain the published declaration while its owner's replacement
            // projection is pending, as in the dispatch-deferral regression.
            let pending = serde_json::to_string(&crate::store::Aggregate::RunPart(run.id.clone())).unwrap();
            store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h832cd32cd126 owner=fractal_reconcile::reconcile::tests::file_watch_scope TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
                rusqlite::params![format!("projection-run-rebuild:{pending}"), pending],
            ).unwrap();
            assert!(store.run_projection_pending(&run.subject).unwrap());
        }
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(store.clone(), runtime.clone(),
            "node".into(), Arc::new(Notify::new()));
        reconciler.file_changes.launches.lock().unwrap().insert("agent/node.selected".into());
        *reconciler.start_stage_finished_hook.lock().unwrap() = Some(Box::new(|| {
            panic!("scoped render must not execute a full pass");
        }));
        assert_eq!(reconciler.evaluate_changed_launch_inputs().unwrap(), 1, "{lifecycle}");
        assert_eq!(std::fs::read_to_string(root.path().join("shared.txt")).unwrap(),
            "selected", "{lifecycle}: inactive declarations must not retain render ownership");
        assert!(store.member_reconcile_fault("agent/node.selected", None).unwrap().is_none(),
            "{lifecycle}");
        assert!(store.observations_for(&stale_subject, "render.applied").unwrap().is_empty(),
            "{lifecycle}: an inactive declaration must not publish a render receipt");
        assert!(runtime.started_members.lock().unwrap().is_empty(),
            "adopt-only declarations must not start");
    }
}
