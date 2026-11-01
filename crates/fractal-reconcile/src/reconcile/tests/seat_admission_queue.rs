use super::*;
use fractal_runtime::seat_admission::{Config, Decision};
use fractal_runtime::seat_admission_status::Status;
use serde_json::json;

fn fixture() -> (Arc<Store>, Arc<FakeRuntime>, Reconciler<FakeRuntime>, Arc<Status>) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let runtime = Arc::new(FakeRuntime::default());
    let status = Arc::new(Status::default());
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()))
        .with_seat_admission(None, status.clone());
    status.state().decision = Decision {
        mode: "enforce".into(), reason: "test pressure".into(), queue: true, observed_at: None,
    };
    (store, runtime, reconciler, status)
}

fn claim(store: &Store, subject: &str, kind: &str, fields: Value, evidence: Vec<String>) -> crate::model::ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some("person/test".into()),
        fields: serde_json::from_value(fields).unwrap(), evidence,
        expected_subject: None, idempotency_key: None,
    }).unwrap()
}

#[test]
fn pending_precedes_workspace_preparation_and_durable_start() {
    for kind in ["agent", "exec", "pty"] {
        let (store, runtime, reconciler, status) = fixture();
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("not-created");
        apply_source(&store, &format!("version 2\n{kind} \"worker\" {{ workspace {:?}; command \"true\" }}", workspace.display().to_string()), "pending");
        let subject = store.desired_subjects().unwrap().remove(0);
        for _ in 0..2 { reconciler.perform_start(&subject, subject.member.as_ref().unwrap(), "admission boundary").unwrap(); }
        assert!(!workspace.exists());
        assert!(store.runtime_start_for(&subject.subject).unwrap().is_none());
        assert!(store.claims_for(&subject.subject, Some("runtime.action.requested")).unwrap().is_empty());
        assert!(store.claims_for(&subject.subject, Some("runtime.action.succeeded")).unwrap().is_empty());
        assert!(runtime.starts.lock().unwrap().is_empty());
        assert_eq!(status.state().pending.len(), 1);
    }
}

#[test]
fn fifo_deduplicates_and_changed_tokens_rejoin_at_the_tail() {
    let status = Status::default();
    status.state().decision.queue = true;
    assert!(!status.admit("a", Some("old".into()), None, false, 10));
    assert!(!status.admit("b", None, None, false, 11));
    assert!(!status.admit("a", Some("old".into()), None, false, 12));
    assert_eq!(status.state().pending.front().unwrap().since, 10);
    assert_eq!(status.state().pending.len(), 2);
    assert!(!status.admit("a", Some("new".into()), None, false, 13));
    assert_eq!(status.state().pending.front().unwrap().subject, "b");
    status.state().decision.queue = false;
    assert!(!status.admit("a", Some("new".into()), None, false, 14));
    assert!(status.admit("b", None, None, false, 14));
    assert!(status.admit("a", Some("new".into()), None, false, 14));
    assert!(status.state().pending.is_empty());
}

#[test]
fn declaration_change_stop_suspend_and_superseded_request_cancel_pending() {
    for cause in ["edit", "stop", "suspend", "request"] {
        let (store, _, reconciler, status) = fixture();
        let source = "version 2\nagent \"worker\" { workspace \"/tmp\"; command \"true\" }";
        apply_source(&store, source, "initial");
        let subject = store.desired_subjects().unwrap().remove(0);
        let member = subject.member.as_ref().unwrap();
        let request = (cause == "request").then(|| claim(&store, &subject.subject, "runtime.action.requested",
            json!({"action":"restart", "incarnation_id":"stopped"}), vec![]));
        assert!(!reconciler.admit_seat_start(&subject, member, request.as_ref()).unwrap());
        match cause {
            "edit" => apply_source(&store, &source.replace("true", "false"), "edited"),
            "stop" => apply_source(&store, &format!("version 2\nstop {:?}", subject.subject), "stopped"),
            "suspend" => { claim(&store, &subject.subject, "runtime.action.requested", json!({"action":"suspend"}),
                vec![store.selected_desired_token(&subject.subject).unwrap().unwrap()]); }
            _ => { claim(&store, &subject.subject, "runtime.action.requested", json!({"action":"restart"}), vec![]); }
        }
        reconciler.cancel_invalid_admissions(&store.desired_subjects().unwrap()).unwrap();
        assert!(status.state().pending.is_empty(), "{cause}");
    }
}

#[test]
fn default_off_and_internal_gate_bypass_pressure() {
    let (store, _, reconciler, status) = fixture();
    apply_source(&store, "version 2\nagent \"worker\" { workspace \"/tmp\"; command \"true\" }", "initial");
    let mut subject = store.desired_subjects().unwrap().remove(0);
    status.state().decision = Status::default().state().decision.clone();
    assert!(reconciler.admit_seat_start(&subject, subject.member.as_ref().unwrap(), None).unwrap());
    status.state().decision.mode = "enforce".into();
    status.state().decision.queue = true;
    subject.kind = "gate".into();
    assert!(reconciler.admit_seat_start(&subject, subject.member.as_ref().unwrap(), None).unwrap());
    assert!(status.state().pending.is_empty());
}

#[test]
fn explicit_restart_only_bypasses_for_an_observed_live_predecessor() {
    for observed in ["running", "stopped"] {
        let (store, _, reconciler, status) = fixture();
        apply_source(&store, "version 2\nagent \"worker\" { workspace \"/tmp\"; command \"true\" }", "initial");
        let subject = store.desired_subjects().unwrap().remove(0);
        claim(&store, &subject.subject, "runtime.observed",
            serde_json::to_value(member_fields(subject.member.as_ref().unwrap(), observed, Some("old"), observed == "running")).unwrap(), vec![]);
        let request = claim(&store, &subject.subject, "runtime.action.requested", json!({"action":"restart", "incarnation_id":"old"}), vec![]);
        assert_eq!(reconciler.admit_seat_start(&subject, subject.member.as_ref().unwrap(), Some(&request)).unwrap(), observed == "running");
        if observed == "running" {
            assert_eq!(status.subject(&subject.subject, 0).unwrap()["reason"], "admitted: replacement");
            claim(&store, &subject.subject, "runtime.observed",
                serde_json::to_value(member_fields(subject.member.as_ref().unwrap(), "stopped", Some("old"), false)).unwrap(), vec![]);
            assert!(reconciler.admit_seat_start(&subject, subject.member.as_ref().unwrap(), Some(&request)).unwrap());
        } else { assert_eq!(status.state().pending.len(), 1); }
    }
}

#[test]
fn ended_helper_launcher_cancels_its_queued_helper() {
    let (store, _, reconciler, status) = fixture();
    apply_source(&store, "version 2\nagent \"launcher\" { workspace \"/tmp\"; harness \"omp\" {} }", "launcher");
    let launcher = store.desired_subjects().unwrap().remove(0);
    claim(&store, &launcher.subject, "runtime.observed",
        serde_json::to_value(member_fields(launcher.member.as_ref().unwrap(), "running", Some("old"), true)).unwrap(), vec![]);
    let source = "version 2\nagent \"helper\" { workspace \"/tmp\"; harness \"omp\" {}; tags st3.client.create-key=\"create\" }";
    let intent = parse_intent(source, "node").unwrap();
    let helper = intent.subjects.values().next().unwrap();
    store.bind_helper_launcher(helper, "create", &launcher.subject, "omp").unwrap();
    let plan = store.mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply_as(&intent, &plan.subject_tokens, "helper", Some(&launcher.subject)).unwrap();
    assert!(!reconciler.admit_seat_start(helper, helper.member.as_ref().unwrap(), None).unwrap());
    claim(&store, &launcher.subject, "runtime.observed",
        serde_json::to_value(member_fields(launcher.member.as_ref().unwrap(), "exited", Some("old"), false)).unwrap(), vec![]);
    assert!(!store.helper_launcher_alive(&helper.subject).unwrap());
    reconciler.cancel_invalid_admissions(&store.desired_subjects().unwrap()).unwrap();
    assert!(status.state().pending.is_empty());
}
#[cfg(target_os = "linux")]
#[test]
fn controller_refresh_releases_missing_and_stale_envelopes_without_waiting() {
    let (store, runtime, reconciler, status) = fixture();
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("state.json");
    let now = u64::try_from(now_ms() / 1000).unwrap();
    let mut payload = json!({"schema":"interactive-envelope.state/v1", "host":fractal_runtime::seat_admission::local_host().unwrap(),
        "observedAt":now, "envelopeCgroup":"/interactive", "envelopeBytes":100, "usageBytes":101,
        "exceeded":true, "exceededSince":now - 60, "batchPaused":true, "batchPausedSince":now - 60, "mode":"enforce"});
    fs::write(&file, payload.to_string()).unwrap();
    let config = Config { state_file: file.clone(), batch_paused_min_secs: 30 };
    let reconciler = reconciler.with_seat_admission(Some(config.clone()), status.clone());
    reconciler.refresh_seat_admission();
    assert!(status.state().decision.queue);
    assert!(!status.admit("worker", None, None, false, now));
    payload["observedAt"] = json!(now - 121);
    fs::write(&file, payload.to_string()).unwrap();
    // Reattaching forces the same refresh path without racing the filesystem watcher.
    let reconciler = reconciler.with_seat_admission(Some(config.clone()), status.clone());
    reconciler.refresh_seat_admission();
    assert_eq!(status.state().decision.mode, "fail-open");
    assert!(status.admit("worker", None, None, false, now));
    drop(reconciler);
    fs::remove_file(&file).unwrap();
    status.state().decision.queue = true;
    assert!(!status.admit("worker", None, None, false, now));
    let reconciler = Reconciler::new(store, runtime, "node".into(), Arc::new(Notify::new()))
        .with_seat_admission(Some(config), status.clone());
    reconciler.refresh_seat_admission();
    assert_eq!(status.state().decision.mode, "fail-open");
    assert!(status.admit("worker", None, None, false, now));
}

#[test]
fn suspension_resume_without_a_restart_request_queues() {
    let (store, _, reconciler, status) = fixture();
    apply_source(&store, "version 2\nagent \"worker\" { workspace \"/tmp\"; harness \"omp\" {} }", "initial");
    let mut subject = store.desired_subjects().unwrap().remove(0);
    subject.member.as_mut().unwrap().environment.insert(crate::suspension::RESUME_ENV.into(), "session".into());
    assert!(!reconciler.admit_seat_start(&subject, subject.member.as_ref().unwrap(), None).unwrap());
    assert_eq!(status.state().pending.len(), 1);
}

#[test]
fn rollout_replacement_uses_liveness_when_operation_was_requested() {
    use crate::RolloutsStore;
    for observed in ["running", "stopped"] {
        let (store, _, reconciler, status) = fixture();
        let source = "version 2\nagent \"worker\" { workspace \"/tmp\"; harness \"omp\" {}; rollout \"manual\" }";
        let intent = crate::graph::parse_owned_set_intent(source, "node").unwrap();
        let mut options = crate::store::owned_sets::Options {
            set: "workers".into(), expected_set: "absent".into(),
            source: crate::store::owned_sets::Source { repository: "test/workers".into(), r#ref: "refs/heads/main".into(), sha: format!("{:040x}", 1), sequence: 1 },
            rollout: None, adopt: Default::default(), allow_empty: false, confirm_retire: None, expected_subjects: Default::default(),
        };
        let preview = store.owned_set_preview(&intent, &options).unwrap();
        options.confirm_retire = Some(preview.digest); options.expected_subjects = preview.expected_subjects;
        store.apply_owned_set(&intent, &options, "initial", "person/test").unwrap();
        let subject = store.desired_subjects().unwrap().remove(0);
        let mut member = subject.member.as_ref().unwrap().clone();
        let token = store.selected_desired_token(&subject.subject).unwrap().unwrap();
        claim(&store, &subject.subject, "runtime.observed",
            serde_json::to_value(member_fields(&member, "running", Some("old"), true)).unwrap(), vec![]);
        claim(&store, &subject.subject, "runtime.action.succeeded", json!({"action":"start","desired_token":token,"incarnation_id":"old"}), vec![]);
        store.append_claim(&ClaimInput { subject: subject.subject.clone(), kind: "harness.session-file".into(), actor: Some(subject.subject.clone()),
            fields: serde_json::from_value(json!({"harness":"omp","session_id":"native-one","incarnation_id":"old"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
        if observed == "stopped" { claim(&store, &subject.subject, "runtime.observed",
            serde_json::to_value(member_fields(&member, "stopped", Some("old"), false)).unwrap(), vec![]); }
        let request = store.request_rollout(&subject.subject, &token, &member, "old", "person/test",
            &crate::rollout::Policy::when_idle(1_800_000, false), "operation").unwrap();
        claim(&store, &subject.subject, "runtime.observed",
            serde_json::to_value(member_fields(&member, "stopped", Some("old"), false)).unwrap(), vec![]);
        member.environment.insert(crate::rollout::OPERATION_ENV.into(), request.id);
        assert_eq!(reconciler.admit_seat_start(&subject, &member, None).unwrap(), observed == "running");
        if observed == "running" {
            assert_eq!(status.subject(&subject.subject, 0).unwrap()["reason"], "admitted: replacement");
        } else { assert_eq!(status.state().pending.len(), 1); }
    }
}
