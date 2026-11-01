use super::*;

fn declare_exec_card(store: &Store, body: &str, key: &str) {
    let source = format!("version 2\nagent \"example/workers\" {{ {body} }}\n");
    let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    let planned = store.mission(&intent, IntentInput { kdl: source, source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
}

fn observe_exec_card(store: &Store, name: &str, status: &str) {
    store.append_claim(&ClaimInput {
        subject: format!("exec/example/workers/{name}"), kind: "runtime.observed".into(),
        actor: None, fields: serde_json::from_value(json!({
            "status":status, "runtime_id":name, "host":"node", "incarnation_id":"one"
        })).unwrap(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn exec_card(store: &Store) -> Value {
    client_agent_resources_cached(store, false, store.index().unwrap()).unwrap()
        .into_iter().find(|card| card["id"] == "agent/example/workers").unwrap()
}

const TWO_TASKS: &str = r#"
    host "node"
    workspace "/work/project"
    exec "first" { id "first"; command "true"; }
    exec "second" { id "second"; command "true"; }
"#;

#[test]
fn exec_only_card_running_children_and_refresh() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = &state.store;
    declare_exec_card(store, TWO_TASKS, "initial");
    // The stale parent absent receipt is deliberately left untouched throughout.
    store.append_claim(&ClaimInput {
        subject: "agent/example/workers".into(), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"status":"absent", "runtime_id":"old-primary",
            "incarnation_id":"old-primary", "restarting_from":"old-primary"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.append_claim(&ClaimInput {
        subject: "agent/example/workers".into(), kind: "harness.observed".into(), actor: None,
        fields: serde_json::from_value(json!({"state":"idle", "driver":"omp",
            "incarnation_id":"old-primary"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert_eq!(exec_card(store)["state"], "waiting");
    observe_exec_card(store, "first", "running");
    observe_exec_card(store, "second", "running");
    let card = exec_card(store);
    assert_eq!(card["state"], "running");
    assert_eq!(card["runtime_ids"], json!(["runtime/first", "runtime/second"]));
    assert_eq!(card["host_id"], "host/node");
    assert_eq!(card["workspace"], "/work/project");
    assert!(card["driver"].is_null());
    assert!(card["harness_state"].is_null());
    assert!(card["incarnation_id"].is_null());
    assert!(card["current_session_id"].is_null());
    assert!(store.desired_subjects_named(&["agent/example/workers".into()]).unwrap()[0].member.is_none());
    observe_exec_card(store, "second", "failed");
    assert_eq!(exec_card(store)["state"], "failed", "a running sibling cannot mask failure");
    observe_exec_card(store, "second", "indeterminate");
    assert_eq!(exec_card(store)["state"], "waiting", "unknown cannot turn green");
    observe_exec_card(store, "second", "exited");
    assert_eq!(exec_card(store)["state"], "running", "a running child keeps the root active");
    observe_exec_card(store, "first", "stopped");
    assert_eq!(exec_card(store)["state"], "stopped");
    let index = store.index().unwrap();
    let cached = exec_card(store);
    let full = client_agent_resources_uncached(store, false, index).unwrap();
    assert_eq!(cached["state"], full[0]["state"]);
    assert_eq!(cached["runtime_ids"], full[0]["runtime_ids"]);
}

#[test]
fn exec_only_card_current_declaration_and_ownership_fences() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = &state.store;
    declare_exec_card(store, TWO_TASKS, "initial");
    observe_exec_card(store, "first", "running");
    observe_exec_card(store, "second", "failed");
    assert_eq!(exec_card(store)["state"], "failed");
    let only_first = r#"host "node"; workspace "/work/project"; exec "first" { id "first"; command "true"; }"#;
    declare_exec_card(store, only_first, "removed-second");
    let card = exec_card(store);
    assert_eq!(card["state"], "running");
    assert_eq!(card["runtime_ids"], json!(["runtime/first"]));
    observe_exec_card(store, "second", "failed");
    assert_eq!(exec_card(store)["state"], "running", "removed children never contribute");
    declare_exec_card(store, &only_first.replace("true", "false"), "changed-launch");
    assert_eq!(exec_card(store)["state"], "waiting", "old receipts cannot prove a new launch");
    observe_exec_card(store, "first", "running");
    assert_eq!(exec_card(store)["state"], "running");
    let mut root_desired = store.desired_subjects_named(&["agent/example/workers".into()]).unwrap();
    root_desired[0].owner_generation = Some("run-generation/other".into());
    let cards = super::super::exec_only::cards(store, &root_desired, store.index().unwrap()).unwrap();
    assert_eq!(cards["agent/example/workers"].state, "waiting", "another generation's child is not ours");
    assert!(cards["agent/example/workers"].runtime_ids.is_empty());
}

#[test]
fn exec_only_card_placement_does_not_guess_from_overridden_children() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = &state.store;
    // Nested authored tasks inherit placement today; exercise divergent normalized
    // members too, without pretending that task-local host syntax is supported.
    let source = format!("version 2\nagent \"example/workers\" {{ {} }}", TWO_TASKS.replace("host \"node\"", ""));
    let mut intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    for (task, host) in [("first", "east"), ("second", "west")] {
        let child = intent.subjects.get_mut(&format!("exec/example/workers/{task}")).unwrap();
        child.member.as_mut().unwrap().host = host.into();
    }
    let planned = store.mission(&intent, IntentInput { kdl: source, source_name: None }).unwrap();
    store.apply(&intent, &planned.subject_tokens, "split-placement").unwrap();
    let card = exec_card(store);
    assert!(card["host_id"].is_null());
    assert_eq!(card["workspace"], "/work/project");
    declare_exec_card(store, r#"exec "first" { id "first"; command "true"; }"#, "inherited-placement");
    let card = exec_card(store);
    assert_eq!(card["host_id"], "host/node");
    assert_eq!(card["workspace"], ".");
}

#[test]
fn exec_only_card_harness_root_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let store = &state.store;
    declare_exec_card(store, &format!("{TWO_TASKS}\nharness \"omp\" {{ model \"test-model\"; }}"), "harness");
    observe_exec_card(store, "first", "running");
    observe_exec_card(store, "second", "running");
    let before = exec_card(store);
    assert_ne!(before["state"], "running");
    observe_exec_card(store, "second", "failed");
    let after = exec_card(store);
    assert_eq!(before["state"], after["state"]);
    assert_eq!(before["runtime_ids"], after["runtime_ids"]);
    assert_eq!(before["host_id"], after["host_id"]);
    assert_eq!(before["workspace"], after["workspace"]);
    assert_eq!(before["driver"], after["driver"]);
}
