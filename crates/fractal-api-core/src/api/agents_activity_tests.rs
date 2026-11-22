use super::*;
use crate::api::tests_shared_client_v0_tests::{get_request, state};

const ROUTE: &str = "/v1/internal/agents/activity";

fn declare(state: &AppState, name: &str) {
    let intent = parse_intent(&format!(
        "version 2\nagent \"activity/{name}\" {{ command \"true\"; workspace \"/declared/workspace\"; restart \"never\"; }}\n"
    ), state.store.origin()).unwrap();
    state.store.apply_internal(&intent, &format!("declare-{name}")).unwrap();
}

fn observe(store: &Store, name: &str, fields: Value) -> String {
    store.append_claim(&ClaimInput {
        subject: format!("agent/activity/{name}"), kind: "runtime.observed".into(), actor: None,
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap().id
}

fn running() -> Value {
    json!({"status":"running", "workspace":"/actual/../workspace", "incarnation_id":"one"})
}

fn owned_agent(state: &AppState, name: &str) -> MissionRunView {
    let mut intent = parse_intent(&format!(
        "version 2\nagent \"activity/{name}\" {{ command \"true\"; workspace \"/declared/workspace\"; restart \"never\"; }}\n\
         mission \"activity-owner-{name}\" state=\"ready\" {{ goal \"Own runtime evidence.\"; step \"work\" {{ agentless }} }}\n"
    ), state.store.origin()).unwrap();
    state.store.apply_internal(&intent, &format!("owner-initial-{name}")).unwrap();
    let run = state.store.create_mission_run(&MissionRunRequest {
        mission: format!("activity-owner-{name}"), revision: None,
        workspace: "/declared/workspace".into(), requester: Some("person/operator".into()),
        mode: Some("run".into()), inputs: BTreeMap::new(),
        idempotency_key: format!("activity-run-{name}"),
    }).unwrap();
    let desired = intent.subjects.get_mut(&format!("agent/activity/{name}")).unwrap();
    desired.owner_run = Some(run.subject.clone());
    desired.owner_generation = Some(run.generation.clone());
    state.store.apply_internal(&intent, &format!("owner-bound-{name}")).unwrap();
    run
}

fn state_claim(state: &AppState, subject: &str, kind: &str, fields: Value) {
    state.store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: None,
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn supersede(state: &AppState, run: &MissionRunView) {
    let generation = if run.generation.starts_with("run-generation/") {
        run.generation.clone()
    } else {
        format!("run-generation/{}", run.generation)
    };
    state_claim(state, &generation, "run-generation.state",
        json!({"status":"superseded", "reason":"The generation was replaced."}));
}

async fn activity(state: &AppState, suffix: &str) -> Value {
    let (status, body) = get_request(router(state.clone()), &format!("{ROUTE}{suffix}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn actual_workspace_and_selected_origin_override_declaration() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "actual");
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let worker = Store::open_memory("worker").unwrap();
    worker.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let claim = observe(&worker, "actual", running());
    crate::test_support::replicate(&worker, &state.store);
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([{
        "id":"agent/activity/actual", "layer":"current", "status":"running", "host":"host/worker",
        "workspace":"/actual/../workspace", "incarnation_id":"one",
        "actual_claim":claim, "actual_origin":"worker"
    }]));
    assert_eq!(body["snapshot"]["store_index"], state.store.index().unwrap());
    assert_eq!(body["snapshot"]["host_id"], "host/node");
    assert_eq!(body["snapshot"]["local_host_id"], "host/node");
    assert_eq!(body["incomplete"], json!([]));
    assert_eq!(body["complete"], true);
    assert!(body["next_cursor"].is_null());
    println!("ACTIVITY_SAMPLE={body}");
}

#[tokio::test]
async fn declaration_without_actual_is_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "missing");
    let body = activity(&state, "").await;
    assert_eq!(body["agents"], json!([]));
    assert_eq!(body["incomplete"], json!([{"id":"agent/activity/missing", "layer":"current", "incarnation_id":null, "host":null, "reason":"no-actual"}]));
    assert_eq!(body["complete"], false);
}

#[tokio::test]
async fn min_cut_above_head_is_retryable_cut_not_ready() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let wanted = state.store.index().unwrap() + 1;
    let (status, body) = get_request(router(state), &format!("{ROUTE}?min_cut={wanted}")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "agent-roster-cut-not-ready");
    assert_eq!(body["details"]["min_cut"], wanted);
}

#[tokio::test]
async fn host_filter_uses_actual_origin_and_scopes_completeness() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "local");
    declare(&state, "unobserved");
    declare(&state, "remote");
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let other = Store::open_memory("other").unwrap();
    other.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    observe(&other, "remote", running());
    crate::test_support::replicate(&other, &state.store);
    // A host field inside the observation is not its selected origin authority.
    let mut fields = running();
    fields["host"] = json!("other");
    observe(&state.store, "local", fields);
    let body = activity(&state, "?host=host%2Fnode").await;
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);
    assert_eq!(body["agents"][0]["id"], "agent/activity/local");
    assert_eq!(body["agents"][0]["host"], "host/node");
    let unknown = json!([{"id":"agent/activity/unobserved", "layer":"current", "incarnation_id":null, "host":null, "reason":"no-actual"}]);
    assert_eq!(body["incomplete"], unknown);
    assert_eq!(body["complete"], false);
    let remote = activity(&state, "?host=host%2Fother").await;
    assert_eq!(remote["agents"].as_array().unwrap().len(), 1);
    assert_eq!(remote["agents"][0]["id"], "agent/activity/remote");
    assert_eq!(remote["incomplete"], unknown);
    assert_eq!(remote["complete"], false);
    let empty = activity(&state, "?host=host%2Fabsent").await;
    assert_eq!(empty["agents"], json!([]));
    assert_eq!(empty["incomplete"], unknown);
    assert_eq!(empty["complete"], false);
}

#[tokio::test]
async fn pagination_keeps_one_cut_and_completes_only_on_last_page() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for name in ["a", "b", "c"] {
        declare(&state, name);
        observe(&state.store, name, running());
    }
    let first = activity(&state, "?limit=1").await;
    assert_eq!(first["complete"], false);
    assert_eq!(first["agents"][0]["id"], "agent/activity/a");
    let mut moved = running();
    moved["workspace"] = json!("/new/workspace");
    observe(&state.store, "b", moved);
    declare(&state, "later");
    let mut page = first.clone();
    for (name, complete) in [("b", false), ("c", true)] {
        let cursor = page["next_cursor"].as_str().unwrap();
        page = activity(&state, &format!("?cursor={}", urlencoding::encode(cursor))).await;
        assert_eq!(page["snapshot"], first["snapshot"]);
        assert_eq!(page["agents"][0]["id"], format!("agent/activity/{name}"));
        assert_eq!(page["agents"][0]["workspace"], "/actual/../workspace");
        assert_eq!(page["complete"], complete);
    }
    assert!(page["next_cursor"].is_null());
}

#[tokio::test]
async fn pagination_remembers_incomplete_rows_on_earlier_pages() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "a");
    declare(&state, "b");
    observe(&state.store, "b", running());
    let first = activity(&state, "?limit=1&host=host%2Fnode").await;
    assert_eq!(first["incomplete"][0]["reason"], "no-actual");
    let cursor = first["next_cursor"].as_str().unwrap();
    let last = activity(&state, &format!("?host=host%2Fnode&cursor={}", urlencoding::encode(cursor))).await;
    assert_eq!(last["incomplete"], json!([]));
    assert_eq!(last["agents"].as_array().unwrap().len(), 1);
    assert!(last["next_cursor"].is_null());
    assert_eq!(last["complete"], false);
    let (status, _) = get_request(router(state), &format!(
        "{ROUTE}?host=host%2Fother&cursor={}", urlencoding::encode(cursor)
    )).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn unknown_status_is_explicit_and_retains_raw_value() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "future");
    observe(&state.store, "future", running());
    let mut fields = running();
    fields["status"] = json!("future-status");
    observe(&state.store, "future", fields);
    let body = activity(&state, "").await;
    assert_eq!(body["agents"][0]["status"], "unknown");
    assert_eq!(body["agents"][0]["status_raw"], "future-status");
    assert_eq!(body["complete"], true);
}

#[tokio::test]
async fn missing_workspace_never_uses_declared_workspace() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "workspace");
    observe(&state.store, "workspace", json!({"status":"running", "incarnation_id":"one"}));
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"], json!([]));
        assert_eq!(body["incomplete"][0]["reason"], "missing-workspace");
        assert_eq!(body["complete"], false);
    }
    let other_host = activity(&state, "?host=host%2Fother").await;
    assert_eq!(other_host["agents"], json!([]));
    assert_eq!(other_host["incomplete"], json!([]));
    assert_eq!(other_host["complete"], true);
}

#[tokio::test]
async fn expired_cursor_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for name in ["a", "b"] {
        declare(&state, name);
        observe(&state.store, name, running());
    }
    let first = activity(&state, "?limit=1").await;
    let mut cursor = decode_client_cursor(first["next_cursor"].as_str().unwrap()).unwrap();
    cursor.expires_at_unix_ms = 0;
    let cursor = encode_client_cursor(&cursor).unwrap();
    let (status, body) = get_request(router(state), &format!(
        "{ROUTE}?cursor={}", urlencoding::encode(&cursor)
    )).await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(body["code"], "page-cursor-expired");
}

#[tokio::test]
async fn concurrent_actual_authorities_remain_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "concurrent");
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let left = Store::open_memory("left").unwrap();
    let right = Store::open_memory("right").unwrap();
    for source in [&left, &right] {
        source.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
        observe(source, "concurrent", running());
        crate::test_support::replicate(source, &state.store);
    }
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"], json!([]));
        assert_eq!(body["incomplete"], json!([{
            "id":"agent/activity/concurrent", "layer":"current", "incarnation_id":"one", "host":null, "reason":"indeterminate-authority"
        }]));
        assert_eq!(body["complete"], false);
    }
}

#[tokio::test]
async fn invalid_page_limits_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    for limit in [0, 201] {
        let (status, _) = get_request(router(state.clone()), &format!("{ROUTE}?limit={limit}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn terminal_owner_never_hides_a_nonterminal_runtime() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let run = owned_agent(&state, "terminal-owner");
    state.store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
    for status in ["running", "starting", "waiting", "suspended", "down", "future-status"] {
        let mut fields = running();
        fields["status"] = json!(status);
        observe(&state.store, "terminal-owner", fields);
        for suffix in ["", "?host=host%2Fnode"] {
            let body = activity(&state, suffix).await;
            assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
            assert_eq!(body["agents"][0]["id"], "agent/activity/terminal-owner");
            assert_eq!(body["agents"][0]["layer"], "historical");
            assert_eq!(body["agents"][0]["workspace"], "/actual/../workspace");
            assert_eq!(body["complete"], true);
        }
    }
}

#[tokio::test]
async fn superseded_generation_never_hides_a_running_runtime() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let run = owned_agent(&state, "superseded");
    observe(&state.store, "superseded", running());
    supersede(&state, &run);
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
        assert_eq!(body["agents"][0]["id"], "agent/activity/superseded");
        assert_eq!(body["agents"][0]["layer"], "historical");
        assert_eq!(body["agents"][0]["status"], "running");
        assert_eq!(body["complete"], true);
    }
}

#[tokio::test]
async fn historical_agents_without_actual_remain_incomplete() {
    for superseded in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let run = owned_agent(&state, "unresolved-owner");
        if superseded {
            supersede(&state, &run);
        } else {
            state.store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
        }
        for suffix in ["", "?host=host%2Fnode"] {
            let body = activity(&state, suffix).await;
            assert_eq!(body["agents"], json!([]));
            assert_eq!(body["incomplete"], json!([{
                "id":"agent/activity/unresolved-owner", "layer":"historical", "incarnation_id":null, "host":null, "reason":"no-actual"
            }]));
            assert_eq!(body["complete"], false);
        }
    }
}

#[tokio::test]
async fn historical_terminal_runtime_ends_are_omitted() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let run = owned_agent(&state, "ended");
    observe(&state.store, "ended", running());
    state.store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
    for status in ["stopped", "exited", "vanished", "absent", "failed"] {
        let mut fields = running();
        fields["status"] = json!(status);
        observe(&state.store, "ended", fields);
        for suffix in ["", "?host=host%2Fnode"] {
            let body = activity(&state, suffix).await;
            assert_eq!(body["agents"], json!([]), "{body}");
            assert_eq!(body["incomplete"], json!([]));
            assert_eq!(body["complete"], true);
        }
    }
}

#[tokio::test]
async fn unrelated_fold_status_does_not_change_incarnation_runtime_status() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let run = owned_agent(&state, "live-incarnation");
    observe(&state.store, "live-incarnation", running());
    state.store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
    state_claim(&state, "agent/activity/live-incarnation", "workspace.observed",
        json!({"host":"node", "workspace":"/next-desired-checkout"}));
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
        assert_eq!(body["agents"][0]["layer"], "historical");
        assert_eq!(body["agents"][0]["status"], "running");
        assert_eq!(body["complete"], true);
    }
}

#[tokio::test]
async fn delayed_named_old_end_never_ends_the_new_incarnation() {
    for historical in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let run = owned_agent(&state, "delayed-end");
        if historical {
            state.store.set_mission_run_state(&run.id, "completed", "terminal", None).unwrap();
        }
        observe(&state.store, "delayed-end",
            json!({"status":"running", "incarnation_id":"old", "workspace":"/old"}));
        observe(&state.store, "delayed-end",
            json!({"status":"running", "incarnation_id":"new", "workspace":"/live"}));
        observe(&state.store, "delayed-end",
            json!({"status":"stopped", "incarnation_id":"old"}));
        for suffix in ["", "?host=host%2Fnode"] {
            let body = activity(&state, suffix).await;
            assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
            assert_eq!(body["agents"][0]["incarnation_id"], "new");
            assert_eq!(body["agents"][0]["workspace"], "/live");
            assert_eq!(body["agents"][0]["status"], "running");
            assert_eq!(body["agents"][0]["layer"], if historical { "historical" } else { "current" });
            assert_eq!(body["incomplete"], json!([]));
            assert_eq!(body["complete"], true);
        }
    }
}

async fn assert_former_host_end(unnamed: bool) {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let intent = parse_intent(
        "version 2\nagent \"activity/moved\" { host \"cedar\"; command \"true\"; workspace \"/declared\"; restart \"never\"; }\n",
        state.store.origin(),
    ).unwrap();
    state.store.apply_internal(&intent, "moved-declaration").unwrap();
    state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    let former = Store::open_memory("birch").unwrap();
    let current = Store::open_memory("cedar").unwrap();
    for source in [&former, &current] {
        source.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
    }
    former.set_write_clock_at(1_900_000_000_001).unwrap();
    observe(&former, "moved", json!({"status":"running", "incarnation_id":"old", "workspace":"/old"}));
    crate::test_support::replicate(&former, &state.store);
    current.set_write_clock_at(1_900_000_000_002).unwrap();
    observe(&current, "moved", json!({"status":"running", "incarnation_id":"new", "workspace":"/live"}));
    crate::test_support::replicate(&current, &state.store);
    former.set_write_clock_at(1_900_000_000_003).unwrap();
    let mut end = json!({"status":"stopped", "workspace":"/old"});
    if !unnamed {
        end["incarnation_id"] = json!("old");
    }
    observe(&former, "moved", end);
    crate::test_support::replicate(&former, &state.store);
    for suffix in ["", "?host=host%2Fcedar"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
        assert_eq!(body["agents"][0]["incarnation_id"], "new");
        assert_eq!(body["agents"][0]["host"], "host/cedar");
        assert_eq!(body["agents"][0]["actual_origin"], "cedar");
        assert_eq!(body["agents"][0]["workspace"], "/live");
        assert_eq!(body["agents"][0]["status"], "running");
        assert_eq!(body["incomplete"], json!([]));
        assert_eq!(body["complete"], true);
    }
    let former_host = activity(&state, "?host=host%2Fbirch").await;
    assert_eq!(former_host["agents"], json!([]));
    assert_eq!(former_host["complete"], true);
}

#[tokio::test]
async fn unnamed_old_host_end_does_not_end_the_new_hosts_incarnation() {
    assert_former_host_end(true).await;
}

#[tokio::test]
async fn moved_host_workspace_never_comes_from_the_former_hosts_end() {
    assert_former_host_end(false).await;
}

#[tokio::test]
async fn overlapping_incarnations_are_retained_and_explain_supersession() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "overlap");
    observe(&state.store, "overlap", json!({"status":"running", "incarnation_id":"one", "workspace":"/old"}));
    observe(&state.store, "overlap", json!({"status":"running", "incarnation_id":"two", "workspace":"/live"}));
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        let rows = body["agents"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{body}");
        assert_eq!(rows[0]["id"], rows[1]["id"]);
        assert_eq!(rows[0]["incarnation_id"], "one");
        assert_eq!(rows[0]["workspace"], "/old");
        assert_eq!(rows[0]["superseded_by"], "two");
        assert_eq!(rows[1]["incarnation_id"], "two");
        assert_eq!(rows[1]["workspace"], "/live");
        assert!(rows[1].get("superseded_by").is_none());
        assert_eq!(body["complete"], true);
    }
    let first = activity(&state, "?limit=1&host=host%2Fnode").await;
    assert_eq!(first["complete"], false);
    assert_eq!(first["agents"][0]["incarnation_id"], "one");
    let cursor = first["next_cursor"].as_str().unwrap();
    let last = activity(&state, &format!("?host=host%2Fnode&cursor={}", urlencoding::encode(cursor))).await;
    assert_eq!(last["agents"][0]["incarnation_id"], "two");
    assert_eq!(last["snapshot"], first["snapshot"]);
    assert_eq!(last["complete"], true);
}

#[tokio::test]
async fn unattributed_workspace_never_fills_a_running_incarnations_missing_path() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "unattributed");
    observe(&state.store, "unattributed", json!({"status":"running", "incarnation_id":"one"}));
    state_claim(&state, "agent/activity/unattributed", "workspace.observed",
        json!({"host":"node", "workspace":"/unattributed"}));
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"], json!([]));
        let reasons = body["incomplete"].as_array().unwrap().iter()
            .filter_map(|row| row["reason"].as_str()).collect::<BTreeSet<_>>();
        assert_eq!(reasons, BTreeSet::from(["missing-workspace"]));
        assert_eq!(body["complete"], false);
    }
}

#[tokio::test]
async fn nonterminal_evidence_without_a_running_witness_remains_unresolved() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "unwitnessed");
    observe(&state.store, "unwitnessed",
        json!({"status":"starting", "incarnation_id":"one", "workspace":"/starting"}));
    for suffix in ["", "?host=host%2Fnode", "?host=host%2Fother"] {
        let body = activity(&state, suffix).await;
        assert_eq!(body["agents"], json!([]));
        assert_eq!(body["incomplete"][0]["reason"], "unresolved-liveness");
        assert_eq!(body["incomplete"][0]["incarnation_id"], "one");
        assert!(body["incomplete"][0]["host"].is_null());
        assert_eq!(body["complete"], false);
    }
}

#[tokio::test]
async fn changing_workspace_retains_every_path_until_the_incarnation_ends() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "paths");
    for workspace in ["/first", "/second", "/first"] {
        observe(&state.store, "paths",
            json!({"status":"running", "incarnation_id":"same", "workspace":workspace}));
    }
    let body = activity(&state, "").await;
    let paths = body["agents"].as_array().unwrap().iter()
        .map(|row| row["workspace"].as_str().unwrap()).collect::<BTreeSet<_>>();
    assert_eq!(paths, BTreeSet::from(["/first", "/second"]));
    assert_eq!(body["complete"], true);
    observe(&state.store, "paths", json!({"status":"stopped", "incarnation_id":"same"}));
    let ended = activity(&state, "").await;
    assert_eq!(ended["agents"], json!([]));
    assert_eq!(ended["complete"], true);
}

#[tokio::test]
async fn same_identifier_resurrection_restores_all_previously_witnessed_paths() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    declare(&state, "resurrected");
    observe(&state.store, "resurrected",
        json!({"status":"running", "incarnation_id":"same", "workspace":"/old"}));
    observe(&state.store, "resurrected", json!({"status":"stopped", "incarnation_id":"same"}));
    observe(&state.store, "resurrected",
        json!({"status":"running", "incarnation_id":"same", "workspace":"/new"}));
    for suffix in ["", "?host=host%2Fnode"] {
        let body = activity(&state, suffix).await;
        let rows = body["agents"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{body}");
        assert!(rows.iter().all(|row| row["incarnation_id"] == "same"));
        let paths = rows.iter().map(|row| row["workspace"].as_str().unwrap()).collect::<BTreeSet<_>>();
        assert_eq!(paths, BTreeSet::from(["/new", "/old"]));
        assert_eq!(body["complete"], true);
    }
}

#[tokio::test]
async fn real_runtime_producer_obeys_the_signed_reader_floor() {
    use fractal_reconcile::reconcile::{Reconciler, tests::FakeRuntime};
    for supported in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
        state.store.bind_fleet(crate::test_support::PAUSE_FLEET).unwrap();
        state.store.pin_fleet_anchor(key.public()).unwrap();
        state.store.set_member_key(Some(key.clone())).unwrap();
        state.store.admit_fleet_anchor(crate::test_support::PAUSE_FLEET, key.public(), "listening").unwrap();
        state_claim(&state, "daemon/node", "daemon.started",
            json!({"status":"running", "features":{"runtime_workspace":u8::from(supported)}}));
        state.store.replication_snapshot().unwrap();
        assert_eq!(state.store.runtime_workspace_publication_ready().unwrap(), supported);

        let source = format!(
            "version 2\nagent \"activity/producer\" {{ workspace {:?}; command \"true\"; restart \"never\"; }}\n",
            root.path().to_str().unwrap(),
        );
        let intent = parse_intent(&source, state.store.origin()).unwrap();
        state.store.apply_internal(&intent, "producer-declaration").unwrap();
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(state.store.clone(), runtime, state.node.clone(), state.notify.clone());
        reconciler.reconcile_once().unwrap();
        let observed = state.store.latest_observation("agent/activity/producer", "runtime.observed").unwrap().unwrap();
        assert_eq!(observed.body["fields"]["status"], "running");
        let body = activity(&state, "").await;
        if supported {
            assert_eq!(observed.body["fields"]["workspace"], root.path().to_str().unwrap());
            assert_eq!(body["agents"].as_array().unwrap().len(), 1, "{body}");
            assert_eq!(body["agents"][0]["workspace"], root.path().to_str().unwrap());
            assert_eq!(body["incomplete"], json!([]));
            assert_eq!(body["complete"], true);
            println!("ACTIVITY_PRODUCER_SAMPLE={body}");
        } else {
            assert!(observed.body["fields"].get("workspace").is_none());
            assert_eq!(body["agents"], json!([]));
            assert!(body["incomplete"].as_array().unwrap().iter()
                .any(|row| row["reason"] == "missing-workspace"), "{body}");
            assert_eq!(body["complete"], false);
        }
    }
}
