use super::*;

fn publish_as(store: &Store, source: &str, key: &str, actor: Option<&str>) {
    let intent = parse_intent(source, "node").unwrap();
    let plan = store.mission(&intent, crate::model::IntentInput {
        kdl: source.into(), source_name: None,
    }).unwrap();
    store.apply_as(&intent, &plan.subject_tokens, key, actor).unwrap();
}

fn fixture(owned: bool) -> (tempfile::TempDir, Arc<Store>, Arc<FakeRuntime>, DesiredSubject) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let source = format!("version 2\nagent \"launcher\" {{ workspace {:?}; harness \"claude\" {{}} }}\n", root.path().display().to_string());
    publish_as(&store, &source, "launcher", owned.then_some("person/avery"));
    let launcher = store.desired_subjects_named(&["agent/node.launcher".into()]).unwrap().remove(0);
    let member = launcher.member.as_ref().unwrap();
    store.append_claim(&ClaimInput {
        subject: launcher.subject.clone(), kind: "runtime.observed".into(), actor: None,
        fields: member_fields(member, "running", Some("launcher-one"), true),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.append_claim(&ClaimInput {
        subject: launcher.subject.clone(), kind: "runtime.action.succeeded".into(), actor: None,
        fields: BTreeMap::from([
            ("action".into(), Value::String("start".into())),
            ("incarnation_id".into(), Value::String("launcher-one".into())),
            ("harness".into(), Value::String("claude".into())),
            ("credential_home".into(), Value::String(root.path().join("credentials").display().to_string())),
        ]), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let source = format!("version 2\nagent \"helper\" {{ workspace {:?}; harness \"claude\" {{}}; tags st3.client.create-key=\"helper-create\" }}\n", root.path().display().to_string());
    let intent = parse_intent(&source, "node").unwrap();
    let helper = intent.subjects.values().next().unwrap().clone();
    store.bind_helper_launcher(&helper, "helper-create", &launcher.subject, "claude").unwrap();
    publish_as(&store, &source, "helper", Some(&launcher.subject));
    let runtime = Arc::new(FakeRuntime::default());
    runtime.ptys.lock().unwrap().push(claude_seat_pty("launcher", "running", "launcher-one"));
    (root, store, runtime, helper)
}

#[test]
fn helper_launcher_inherits_credential_reference_without_impersonation_or_secret_copy() {
    let (root, store, runtime, helper) = fixture(true);
    let credentials = root.path().join("credentials");
    std::fs::create_dir(&credentials).unwrap();
    std::fs::write(credentials.join("secret"), "not-for-argv-or-env").unwrap();
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    assert_eq!(store.agent_person(&helper.subject).unwrap().as_deref(), Some("person/avery"));
    reconciler.ensure_login_fallback_fault(&helper, "helper-login").unwrap();
    assert!(store.operational_failures().unwrap().is_empty());
    reconciler.reconcile_once().unwrap();
    let started = runtime.started_members.lock().unwrap();
    let launch = started.iter().find(|member| member.runtime_id == "node.helper").unwrap();
    assert_eq!(launch.environment["CLAUDE_CONFIG_DIR"], credentials.display().to_string());
    assert_eq!(launch.environment["ST_AGENT"], helper.subject);
    assert!(!serde_json::to_string(launch).unwrap().contains("not-for-argv-or-env"));
    assert_eq!(std::fs::read_dir(&credentials).unwrap().count(), 1);
    let receipts = store.observations_for(&helper.subject, "runtime.action.succeeded").unwrap();
    let receipt = receipts.iter().find(|claim| claim.body["fields"]["action"] == "start").unwrap();
    assert_eq!(receipt.body["fields"]["launcher_binding"]["launcher"], "agent/node.launcher");
    assert_eq!(receipt.body["fields"]["launcher_binding"]["person"], "person/avery");
    assert_eq!(receipt.body["fields"]["credential_home"], credentials.display().to_string());
}

#[test]
fn helper_launcher_unowned_preserves_person_ownership_attention() {
    let (_root, store, runtime, helper) = fixture(false);
    let reconciler = Reconciler::new(store.clone(), runtime, "node".into(), Arc::new(Notify::new()));
    assert_eq!(store.agent_person(&helper.subject).unwrap(), None);
    reconciler.ensure_login_fallback_fault(&helper, "helper-unowned-login").unwrap();
    assert!(!store.operational_failures().unwrap().is_empty());
}

#[test]
fn helper_launcher_end_stops_helper_and_blocks_restart_and_credential_resolution_after_daemon_restart() {
    let (_root, store, runtime, helper) = fixture(true);
    let member = helper.member.as_ref().unwrap();
    runtime.ptys.lock().unwrap().push(claude_seat_pty("helper", "running", "helper-one"));
    let launcher = store.desired_subjects_named(&["agent/node.launcher".into()]).unwrap().remove(0);
    store.append_claim(&ClaimInput {
        subject: launcher.subject.clone(), kind: "runtime.observed".into(), actor: None,
        fields: member_fields(launcher.member.as_ref().unwrap(), "exited", Some("launcher-one"), false),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    *runtime.ptys.lock().unwrap() = vec![claude_seat_pty("helper", "running", "helper-one")];
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    reconciler.reconcile_once().unwrap();
    assert!(runtime.stops.lock().unwrap().contains(&"node.helper".into()));
    assert!(!store.helper_launcher_alive(&helper.subject).unwrap());
    assert!(reconciler.perform_start(&helper, member, "restart").is_err());
    let mut launch = member.clone();
    assert!(reconciler.bind_account(&helper, member, &mut launch).is_err());
    assert!(!launch.environment.contains_key("CLAUDE_CONFIG_DIR"));
    let restarted = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    assert!(restarted.perform_start(&helper, member, "daemon restart").is_err());
    assert!(runtime.starts.lock().unwrap().iter().all(|id| id != "node.helper"));
}
