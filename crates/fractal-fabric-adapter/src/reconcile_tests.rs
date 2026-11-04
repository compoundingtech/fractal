use super::*;
use tokio::net::UnixListener;

const FLEET: &str = "test-fleet";
const OWNER: &str = "st/fleet/test-fleet";

#[tokio::test]
async fn renamed_owner_revocation_and_residuals_never_grant_pty() {
    let root = tempfile::tempdir().unwrap();
    let renamed = "fractal/fleet/test-fleet";
    let removed = vec![peer("gone", OWNER, &["st3-client-v0"]), peer("gone", renamed, &["fractal-client-v0"])];
    let (socket, server) = serve(root.path(), vec![
            Some(list(removed, 7)),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("gone", renamed, &["fractal-client-v0"])], 8)),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("gone", "static", &["fractal-client-v0"])], 9)),
    ]);
    assert_eq!(withdraw(root.path(), &socket, FLEET).await.unwrap().state, "residual_access");
    let requests = server.await.unwrap();
    for (index, owner) in [(1, OWNER), (3, renamed)] {
        assert_eq!(requests[index]["request"]["owner"], owner);
        assert_eq!(
            requests[index]["request"]["operations"],
            json!([{"op":"remove_peer","node_id":"gone"}])
        );
    }
}

#[test]
fn renamed_owner_reduction_and_recovered_batches_are_reader_only() {
    let renamed = "fractal/fleet/test-fleet";
    let services = ["fractal/fleet/test-fleet", "fractal-client-v0", "fractal/pty/test-fleet"];
    let records = vec![peer("peer-node", renamed, &services), peer("old-node", renamed, &["st3-client-v0"])];
    let snapshot: Snapshot = serde_json::from_value(list(records, 7)["reply"]["snapshot"].clone()).unwrap();
    let desired = DesiredPolicy::from([(
        "peer-node".into(),
        BTreeSet::from(["st3/fleet/test-fleet".into()]),
    )]);
    let retained = retained_policy(&snapshot, renamed, &desired);
    let operations = plan(&snapshot, renamed, &retained, &Expiries::new());
    assert_eq!(
        operations[0],
        json!({"op":"remove_peer","node_id":"old-node"})
    );
    assert_eq!(operations[1], json!({"op":"remove_peer","node_id":"peer-node"}));
    let mut pending = uncertain_add();
    pending.request["owner"] = json!(renamed);
    pending.request["operations"][1]["grants"] = json!(["fractal/fleet/test-fleet"]);
    assert!(!pending_matches_desired(&pending, &desired, &Expiries::new()));
    pending.request["operations"][1]["grants"] = json!([format!("{RENAMED_PTY_PREFIX}{FLEET}")]);
    assert!(!pending_matches_desired(&pending, &desired, &Expiries::new()));
}

#[tokio::test]
async fn renamed_pending_addition_is_observed_never_sent() {
    let root = tempfile::tempdir().unwrap();
    let mut pending = uncertain_add();
    pending.request["owner"] = json!(format!("{RENAMED_OWNER_PREFIX}{FLEET}"));
    let (socket, server) = serve(root.path(), vec![Some(mutation(vec![], "complete"))]);
    complete_pending(&socket, &pending, pending_matches_desired(&pending,
        &DesiredPolicy::from([("peer-node".into(), grants(FLEET))]), &Expiries::new())).await.unwrap();
    assert_eq!(server.await.unwrap()[0]["request"]["operation"], "cancellation");
}

fn member(name: &str, key: &str, node: &str, state: &str) -> MemberView {
    MemberView {
        name: name.into(),
        member_key: key.into(),
        node_id: Some(node.into()),
        state: state.into(),
        mode: "listening".into(),
        endpoints_published: true,
        endpoints: vec![
            json!({"transport":"fabric", "node":node, "protocol":"st3/fleet/test-fleet"}),
        ],
        start: 1,
        end: None,
        ended: None,
        removed_by: None,
        removal_reason: None,
    }
}
fn view() -> FleetView {
    FleetView {
        anchor: Some("own-key".into()),
        members: vec![
            member("own", "own-key", "own-node", "current"),
            member("peer", "peer-key", "peer-node", "current"),
        ],
        legacy_removed: vec![],
    }
}
fn peer(node: &str, owner: &str, services: &[&str]) -> Value {
    let source = if owner == "static" {
        json!({"kind":"static", "path":"/config/peers.toml"})
    } else {
        json!({"kind":"runtime", "owner":owner})
    };
    let services = services
        .iter()
        .map(|service| ((*service).into(), json!([source.clone()])))
        .collect::<serde_json::Map<String, Value>>();
    json!({"node_id":node, "trust_sources":[source], "services":services})
}
fn list(peers: Vec<Value>, revision: u64) -> Value {
    json!({"type":"runtime_peers", "reply":{"snapshot":{"instance":"instance-a", "policy_revision":revision, "peers":peers, "next_cursor":null, "fault":null}, "mutation":null, "error":null}})
}
fn mutation(peers: Vec<Value>, cancellation: &str) -> Value {
    json!({"type":"runtime_peers", "reply":{"snapshot":null, "mutation":{"current_revision":8, "affected":peers, "cancellation":{"state":cancellation}}, "error":null}})
}
fn refusal(code: &str) -> Value {
    json!({"type":"runtime_peers", "reply":{"snapshot":null, "mutation":null, "error":{"code":code, "class":"refusal", "message":"disabled"}}})
}

// Unlike command shims used for older Fabric, this exercises the actual local wire boundary.
// None drops a reply after reading a request, simulating an uncertain committed mutation.
fn serve(
    root: &Path,
    replies: Vec<Option<Value>>,
) -> (PathBuf, tokio::task::JoinHandle<Vec<Value>>) {
    let socket = root.join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            requests.push(serde_json::from_str(&line).unwrap());
            if let Some(reply) = reply {
                let mut bytes = serde_json::to_vec(&reply).unwrap();
                bytes.push(b'\n');
                stream.write_all(&bytes).await.unwrap();
            }
        }
        requests
    });
    (socket, task)
}

#[test]
fn signed_current_members_only_and_local_retirement_withdraws_everything() {
    let mut view = view();
    view.members
        .push(member("ended", "ended-key", "ended-node", "ended"));
    view.members.push(member(
        "conflicted",
        "conflicted-key",
        "conflicted-node",
        "conflicted",
    ));
    assert_eq!(
        desired(&view, "own-key", FLEET, &BTreeSet::new()).unwrap(),
        BTreeSet::from(["peer-node".into()])
    );
    view.members[0].state = "ended".into();
    assert!(
        desired(&view, "own-key", FLEET, &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
    view.anchor = None;
    assert!(desired(&view, "own-key", FLEET, &BTreeSet::new()).is_err());
}

#[test]
fn endpoint_protocol_accepts_both_names_only_for_this_fleet() {
    for (protocol, accepted) in [
        ("st3/fleet/test-fleet", true),
        ("fractal/fleet/test-fleet", true),
        ("st3/fleet/other-fleet", false),
        ("fractal/fleet/other-fleet", false),
        ("other/fleet/test-fleet", false),
        ("fractal/pty/test-fleet", false),
    ] {
        let mut view = view();
        view.members[1].endpoints[0]["protocol"] = json!(protocol);
        let result = desired(&view, "own-key", FLEET, &BTreeSet::new());
        assert_eq!(result.is_ok(), accepted, "{protocol}");
        if accepted {
            assert_eq!(result.unwrap(), BTreeSet::from(["peer-node".into()]));
        }
    }
}

#[test]
fn endpoint_protocol_cannot_inject_non_st_grants() {
    let mut view = view();
    view.members[1].endpoints[0]["protocol"] = json!("exec");
    assert!(
        desired(&view, "own-key", FLEET, &BTreeSet::new())
            .unwrap_err()
            .to_string()
            .contains("nonstandard")
    );
    assert_eq!(
        grants(FLEET),
        BTreeSet::from([
            "st3/fleet/test-fleet".into(),
            "st3-client-v0".into()
        ])
    );
}

#[tokio::test]
async fn adds_peer_and_replaces_exact_grants_in_one_atomic_batch() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![], 7)),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("peer-node", OWNER, &["st3-client-v0", "st3/fleet/test-fleet"])], 8)),
        ],
    );
    let status = reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &view(),
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    assert_eq!(status.state, "applied");
    assert_eq!(status.policy_revision, Some(8));
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["type"], "runtime_peers");
    let batch = &requests[1]["request"];
    assert_eq!(batch["operation"], "batch");
    assert_eq!(batch["owner"], OWNER);
    assert_eq!(batch["expected_revision"], 7);
    assert_eq!(
        batch["operations"],
        json!([
            {"op":"add_peer", "node_id":"peer-node"},
            {"op":"set_grants", "node_id":"peer-node", "grants":["st3-client-v0", "st3/fleet/test-fleet"]}
        ])
    );
}

#[tokio::test]
async fn rekey_removes_old_identity_and_adds_new_in_same_transaction() {
    let root = tempfile::tempdir().unwrap();
    let services = ["st3-client-v0", "st3/fleet/test-fleet"];
    let mut membership = view();
    let member_key = membership.members[1].member_key.clone();
    membership.members[1].endpoints[0]["node"] = json!("new-node");
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![peer("old-node", OWNER, &services)], 7)),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("new-node", OWNER, &services)], 8)),
        ],
    );
    reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &membership,
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(membership.members[1].member_key, member_key);
    // Only list -> one mutation -> list crosses the control boundary. There is
    // no separate remove/add request that could expose an intermediate policy.
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["request"]["operation"], "list");
    assert_eq!(requests[2]["request"]["operation"], "list");
    let batch = &requests[1]["request"];
    assert_eq!(batch["operation"], "batch");
    assert_eq!(batch["owner"], OWNER);
    assert_eq!(batch["expected_revision"], 7);
    assert_eq!(
        batch["operations"],
        json!([
            {"op":"remove_peer", "node_id":"old-node"},
            {"op":"add_peer", "node_id":"new-node"},
            {"op":"set_grants", "node_id":"new-node", "grants":services}
        ])
    );
}

#[tokio::test]
async fn existing_owner_grants_are_replaced_without_touching_other_owners() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(
                vec![
                    peer("peer-node", OWNER, &["exec"]),
                    peer("unrelated", "other-tool", &["shell"]),
                ],
                7,
            )),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("peer-node", OWNER, &["st3-client-v0", "st3/fleet/test-fleet"])], 8)),
        ],
    );
    reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &view(),
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    let operations = requests[1]["request"]["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["op"], "set_grants");
    assert!(
        !operations[0]["grants"]
            .as_array()
            .unwrap()
            .contains(&json!("exec"))
    );
}

#[tokio::test]
async fn already_exact_policy_needs_no_mutation() {
    let root = tempfile::tempdir().unwrap();
    let exact = peer(
        "peer-node",
        OWNER,
        &[
            "st3-client-v0",
            "st3/fleet/test-fleet",
        ],
    );
    let (socket, server) = serve(root.path(), vec![Some(list(vec![exact], 7))]);
    assert_eq!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .unwrap()
        .policy_revision,
        Some(7)
    );
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn lost_reply_retries_identical_request_id_revision_and_contents() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![], 7)),
            None,
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("peer-node", OWNER, &["st3-client-v0", "st3/fleet/test-fleet"])], 8)),
        ],
    );
    reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &view(),
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests[1], requests[2]);
}

#[tokio::test]
async fn runtime_disabled_is_clear_and_does_not_write_policy_or_fallback_files() {
    let root = tempfile::tempdir().unwrap();
    let static_path = root.path().join("peers.toml");
    std::fs::write(&static_path, "static policy").unwrap();
    let (socket, server) = serve(root.path(), vec![Some(refusal("runtime_peers_disabled"))]);
    let error = reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &view(),
        &BTreeSet::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error_status(&error).state, "runtime_peers_disabled");
    assert!(error.to_string().contains("--runtime-peers"));
    assert_eq!(
        std::fs::read_to_string(static_path).unwrap(),
        "static policy"
    );
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn stale_revision_and_cancellation_timeout_never_report_applied() {
    for reply in [
        refusal("policy_revision_conflict"),
        mutation(vec![], "timed_out"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (socket, server) = serve(root.path(), vec![Some(list(vec![], 7)), Some(reply)]);
        assert!(
            reconcile(
                root.path(),
                &socket,
                FLEET,
                "own-key",
                &view(),
                &BTreeSet::new()
            )
            .await
            .is_err()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn removal_reports_residual_static_access_even_after_previous_owner_removal() {
    let mut view = view();
    view.members[1].state = "ended".into();
    let root = tempfile::tempdir().unwrap();
    let retained = peer("peer-node", "static", &["st3/fleet/test-fleet"]);
    let (socket, server) = serve(root.path(), vec![Some(list(vec![retained.clone()], 7))]);
    let status = reconcile(
        root.path(),
        &socket,
        FLEET,
        "own-key",
        &view,
        &BTreeSet::new(),
    )
    .await
    .unwrap();
    assert_eq!(status.state, "residual_access");
    assert!(status.message.unwrap().contains("/config/peers.toml"));
    server.await.unwrap();
}

#[tokio::test]
async fn repeated_withdrawal_cannot_hide_other_owner_access() {
    let root = tempfile::tempdir().unwrap();
    let retained = peer("peer-node", "other-tool", &["st3-client-v0"]);
    let (socket, server) = serve(root.path(), vec![Some(list(vec![retained.clone()], 7))]);
    assert_eq!(
        withdraw(root.path(), &socket, FLEET).await.unwrap().state,
        "residual_access"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn an_operator_terminal_grant_is_neither_managed_nor_reported_as_residual() {
    let root = tempfile::tempdir().unwrap();
    // Client-only access: the adapter never grants the terminal protocol, so an operator's own
    // `st3/pty/<fleet-id>` grant on another owner is outside this owner's policy. Withdrawal
    // leaves it alone and does not call it residual access, and sends no mutation.
    let retained = peer("peer-node", "static", &["st3/pty/test-fleet"]);
    let (socket, server) = serve(root.path(), vec![Some(list(vec![retained.clone()], 7))]);
    assert_eq!(
        withdraw(root.path(), &socket, FLEET).await.unwrap().state,
        "applied"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn pagination_never_combines_policy_generations() {
    let root = tempfile::tempdir().unwrap();
    let mut first = list(vec![], 7);
    first["reply"]["snapshot"]["next_cursor"] =
        json!({"instance":"instance-a", "revision":7, "after_node":"a"});
    let (socket, server) = serve(root.path(), vec![Some(first), Some(list(vec![], 8))]);
    assert!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("pagination")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn too_large_atomic_change_is_refused_without_partial_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut view = view();
    for i in 0..64 {
        view.members.push(member(
            &format!("host-{i}"),
            &format!("key-{i}"),
            &format!("node-{i}"),
            "current",
        ));
    }
    let (socket, server) = serve(root.path(), vec![Some(list(vec![], 7))]);
    assert!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view,
            &BTreeSet::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("limit 128")
    );
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn restart_resolves_lost_commit_before_accepting_an_empty_diff() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(root.path(), vec![Some(list(vec![], 7)), None, None]);
    assert!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .is_err()
    );
    let original = server.await.unwrap();
    let journal = load_journal(root.path()).await.unwrap();
    assert_eq!(
        journal.pending.as_ref().unwrap().request,
        original[1]["request"]
    );
    std::fs::remove_file(&socket).unwrap();
    let exact = peer(
        "peer-node",
        OWNER,
        &[
            "st3-client-v0",
            "st3/fleet/test-fleet",
        ],
    );
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(mutation(vec![], "complete")),
            Some(list(vec![exact], 8)),
        ],
    );
    assert_eq!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .unwrap()
        .state,
        "applied"
    );
    let resumed = server.await.unwrap();
    assert_eq!(resumed[0], original[1]);
    assert_eq!(resumed[1]["request"]["operation"], "list");
    assert!(load_journal(root.path()).await.unwrap().pending.is_none());
}

#[tokio::test]
async fn timed_out_cancellation_survives_restart_and_never_becomes_noop_success() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![Some(list(vec![], 7)), Some(mutation(vec![], "timed_out"))],
    );
    assert!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .is_err()
    );
    let original = server.await.unwrap();
    std::fs::remove_file(&socket).unwrap();
    let (socket, server) = serve(root.path(), vec![Some(mutation(vec![], "timed_out"))]);
    assert!(
        withdraw(root.path(), &socket, FLEET)
            .await
            .unwrap_err()
            .to_string()
            .contains("retained transaction")
    );
    let resumed = server.await.unwrap();
    assert_eq!(resumed[0]["request"]["operation"], "cancellation");
    assert_eq!(
        resumed[0]["request"]["request_id"],
        original[1]["request"]["request_id"]
    );
    assert!(load_journal(root.path()).await.unwrap().pending.is_some());
}

#[tokio::test]
async fn rekey_residual_warning_survives_later_polling_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let retained = peer("old-node", "static", &["st3/fleet/test-fleet"]);
    let exact = peer(
        "peer-node",
        OWNER,
        &[
            "st3-client-v0",
            "st3/fleet/test-fleet",
        ],
    );
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(
                vec![peer("old-node", OWNER, &["st3/fleet/test-fleet"])],
                7,
            )),
            Some(mutation(vec![retained.clone(), exact.clone()], "complete")),
            Some(list(vec![retained.clone(), exact.clone()], 8)),
            Some(list(vec![retained, exact], 8)),
        ],
    );
    for _ in 0..2 {
        let status = reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new(),
        )
        .await
        .unwrap();
        assert_eq!(status.state, "residual_access");
        assert!(status.message.unwrap().contains("old-node"));
    }
    assert_eq!(server.await.unwrap().len(), 4);
    assert!(
        load_journal(root.path())
            .await
            .unwrap()
            .retired
            .contains("old-node")
    );
}

#[tokio::test]
async fn real_fault_class_revision_conflict_allows_fresh_reconciliation() {
    let root = tempfile::tempdir().unwrap();
    let mut conflict = refusal("policy_revision_conflict");
    conflict["reply"]["error"]["class"] = json!("fault");
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![], 7)),
            Some(conflict),
            Some(list(vec![], 8)),
            Some(mutation(vec![], "complete")),
            Some(list(vec![peer("peer-node", OWNER, &["st3-client-v0", "st3/fleet/test-fleet"])], 9)),
        ],
    );
    assert!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .is_err()
    );
    assert!(load_journal(root.path()).await.unwrap().pending.is_none());
    assert_eq!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view(),
            &BTreeSet::new()
        )
        .await
        .unwrap()
        .state,
        "applied"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests[1]["request"]["expected_revision"], 7);
    assert_eq!(requests[2]["request"]["operation"], "list");
    assert_eq!(requests[3]["request"]["expected_revision"], 8);
}

fn missing_receipt() -> Value {
    let mut reply = refusal("invalid_argument");
    reply["reply"]["error"]["class"] = json!("usage");
    reply["reply"]["error"]["message"] = json!("unknown or expired runtime request_id");
    reply
}

fn uncertain_add() -> Pending {
    Pending {
        request: json!({
            "operation":"batch", "owner":OWNER, "request_id":"uncertain-add", "expected_revision":7,
            "operations":[
                {"op":"add_peer","node_id":"peer-node"},
                {"op":"set_grants","node_id":"peer-node","grants":grants(FLEET)},
            ],
        }),
        created_unix_ms: now_ms(),
    }
}

#[tokio::test]
async fn removed_member_never_replays_an_unreceived_addition() {
    let root = tempfile::tempdir().unwrap();
    save_journal(
        root.path(),
        &Journal {
            pending: Some(uncertain_add()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut view = view();
    view.members[1].state = "ended".into();
    view.members[1].ended = Some("removed".into());
    let (socket, server) = serve(
        root.path(),
        vec![Some(missing_receipt()), Some(list(vec![], 7))],
    );
    assert_eq!(
        reconcile(
            root.path(),
            &socket,
            FLEET,
            "own-key",
            &view,
            &BTreeSet::new()
        )
        .await
        .unwrap()
        .state,
        "applied"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]["request"],
        json!({"operation":"cancellation","request_id":"uncertain-add"})
    );
    assert_eq!(requests[1]["request"]["operation"], "list");
    assert!(load_journal(root.path()).await.unwrap().pending.is_none());
}

#[tokio::test]
async fn leaving_node_queries_obsolete_addition_without_granting_it() {
    let root = tempfile::tempdir().unwrap();
    save_journal(
        root.path(),
        &Journal {
            pending: Some(uncertain_add()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![Some(missing_receipt()), Some(list(vec![], 7))],
    );
    assert_eq!(
        withdraw(root.path(), &socket, FLEET).await.unwrap().state,
        "applied"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["request"]["operation"], "cancellation");
    assert!(
        requests
            .iter()
            .all(|request| request["request"]["operation"] != "batch")
    );
}

#[tokio::test]
async fn committed_obsolete_addition_is_queried_then_revoked_never_replayed() {
    let root = tempfile::tempdir().unwrap();
    save_journal(
        root.path(),
        &Journal {
            pending: Some(uncertain_add()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(mutation(vec![], "complete")),
            Some(list(
                vec![peer("peer-node", OWNER, &["st3/fleet/test-fleet"])],
                8,
            )),
            Some(mutation(vec![], "complete")),
            Some(list(vec![], 8)),
        ],
    );
    assert_eq!(
        withdraw(root.path(), &socket, FLEET).await.unwrap().state,
        "applied"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["request"]["operation"], "cancellation");
    assert_eq!(
        requests[2]["request"]["operations"],
        json!([{"op":"remove_peer","node_id":"peer-node"}])
    );
}

#[test]
fn retained_leave_carriers_require_exact_signed_left_membership() {
    let mut view = view();
    let retained = BTreeSet::from(["own-key".into(), "peer-key".into()]);
    view.members[0].state = "ended".into();
    view.members[0].ended = Some("left".into());
    assert_eq!(
        desired(&view, "own-key", FLEET, &retained).unwrap(),
        BTreeSet::from(["peer-node".into()])
    );
    view.members[1].state = "ended".into();
    view.members[1].ended = Some("left".into());
    assert_eq!(
        desired(&view, "own-key", FLEET, &retained).unwrap(),
        BTreeSet::from(["peer-node".into()])
    );
    view.members[1].ended = Some("removed".into());
    assert!(
        desired(&view, "own-key", FLEET, &retained)
            .unwrap()
            .is_empty()
    );
    view.members[0].ended = Some("removed".into());
    assert!(
        desired(&view, "own-key", FLEET, &retained)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn local_and_remote_leave_keep_only_replication_carrier_until_receipt() {
    for leaving_index in [0, 1] {
        let root = tempfile::tempdir().unwrap();
        let mut view = view();
        view.members[leaving_index].state = "ended".into();
        view.members[leaving_index].ended = Some("left".into());
        let retained = BTreeSet::from([view.members[leaving_index].member_key.clone()]);
        let exact = peer(
            "peer-node",
            OWNER,
            &[
                "st3-client-v0",
                "st3/fleet/test-fleet",
            ],
        );
        let carrier = peer("peer-node", OWNER, &["st3/fleet/test-fleet"]);
        let (socket, server) = serve(
            root.path(),
            vec![
                Some(list(vec![exact], 7)),
                Some(mutation(vec![], "complete")),
                Some(list(vec![carrier.clone()], 8)),
                Some(list(vec![carrier], 8)),
                Some(mutation(vec![], "complete")),
                Some(list(vec![], 8)),
            ],
        );
        assert_eq!(
            reconcile(root.path(), &socket, FLEET, "own-key", &view, &retained)
                .await
                .unwrap()
                .state,
            "leaving"
        );
        assert_eq!(
            reconcile(
                root.path(),
                &socket,
                FLEET,
                "own-key",
                &view,
                &BTreeSet::new()
            )
            .await
            .unwrap()
            .state,
            "applied"
        );
        let requests = server.await.unwrap();
        assert_eq!(
            requests[1]["request"]["operations"],
            json!([{"op":"set_grants","node_id":"peer-node","grants":["st3/fleet/test-fleet"]}])
        );
        assert_eq!(
            requests[4]["request"]["operations"],
            json!([{"op":"remove_peer","node_id":"peer-node"}])
        );
    }
}

#[tokio::test]
async fn leave_drain_never_replays_full_service_grants() {
    let root = tempfile::tempdir().unwrap();
    save_journal(
        root.path(),
        &Journal {
            pending: Some(uncertain_add()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut view = view();
    view.members[1].state = "ended".into();
    view.members[1].ended = Some("left".into());
    let retained = BTreeSet::from(["peer-key".into()]);
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(missing_receipt()),
            Some(list(vec![], 7)),
            Some(mutation(vec![], "complete")),
            Some(list(
                vec![peer("peer-node", OWNER, &["st3/fleet/test-fleet"])],
                8,
            )),
        ],
    );
    assert_eq!(
        reconcile(root.path(), &socket, FLEET, "own-key", &view, &retained)
            .await
            .unwrap()
            .state,
        "leaving"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["request"]["operation"], "cancellation");
    assert_eq!(
        requests[2]["request"]["operations"],
        json!([
            {"op":"add_peer","node_id":"peer-node"},
            {"op":"set_grants","node_id":"peer-node","grants":["st3/fleet/test-fleet"]},
        ])
    );
}

fn rotation_view(state: &str, deadline: u64) -> FleetView {
    let mut view = view();
    view.members[1].endpoints[0]["node"] = json!(if state == "committed" { "next-node" } else { "peer-node" });
    view.members[1].endpoints.push(json!({
        "transport":"fabric-rekey", "old_node":"peer-node", "next_node":"next-node",
        "deadline":deadline, "state":state,
    }));
    view
}

fn expiring_peer(node: &str, deadline: u64) -> Value {
    let mut value = peer(node, OWNER, &["st3-client-v0", "st3/fleet/test-fleet"]);
    value["trust_sources"][0]["expires_at"] = json!(deadline);
    for sources in value["services"].as_object_mut().unwrap().values_mut() {
        sources[0]["expires_at"] = json!(deadline);
    }
    value
}

#[tokio::test]
async fn prepared_committed_and_aborted_reconcile_exact_immutable_expiry() {
    let deadline = (now_ms() / 1000) as u64 + 300;
    let permanent = |node| peer(node, OWNER, &["st3-client-v0", "st3/fleet/test-fleet"]);
    let staged = vec![permanent("peer-node"), expiring_peer("next-node", deadline)];
    let committed = vec![expiring_peer("peer-node", deadline), permanent("next-node")];
    for (state, before, after, expected) in [
        ("prepared", vec![permanent("peer-node")], staged.clone(), json!([
            {"op":"add_peer","node_id":"next-node"},
            {"op":"set_grants","node_id":"next-node","grants":["st3-client-v0","st3/fleet/test-fleet"]},
            {"op":"set_expiry","node_id":"next-node","expires_at":deadline}
        ])),
        ("committed", staged, committed.clone(), json!([
            {"op":"remove_peer","node_id":"next-node"},
            {"op":"add_peer","node_id":"next-node"},
            {"op":"set_grants","node_id":"next-node","grants":["st3-client-v0","st3/fleet/test-fleet"]},
            {"op":"set_expiry","node_id":"peer-node","expires_at":deadline}
        ])),
        ("aborted", committed, vec![permanent("peer-node")], json!([
            {"op":"remove_peer","node_id":"next-node"},
            {"op":"remove_peer","node_id":"peer-node"},
            {"op":"add_peer","node_id":"peer-node"},
            {"op":"set_grants","node_id":"peer-node","grants":["st3-client-v0","st3/fleet/test-fleet"]}
        ])),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut replies = vec![
            Some(list(before, 7)), Some(mutation(vec![], "complete")), Some(list(after, 8)),
        ];
        if state != "aborted" {
            replies.insert(0, Some(json!({"type":"reachability_status","version":"0.9.4+19e7f89"})));
        }
        let (socket, server) = serve(root.path(), replies);
        let status = reconcile(root.path(), &socket, FLEET, "own-key", &rotation_view(state, deadline), &BTreeSet::new()).await.unwrap();
        assert_eq!(status.state, "applied");
        assert_eq!(server.await.unwrap()[1 + usize::from(state != "aborted")]["request"]["operations"], expected);
    }
}

#[test]
fn expired_transitions_never_renew_staging_and_never_expire_prepared_old() {
    for state in ["prepared", "committed", "aborted"] {
        let view = rotation_view(state, 1000);
        let mut policy = desired(&view, "own-key", FLEET, &BTreeSet::new()).unwrap()
            .into_iter().map(|node| (node, grants(FLEET))).collect();
        let (expiries, receipts) = rotations(&view, "own-key", &mut policy, 1000).unwrap();
        assert!(expiries.is_empty());
        assert!(receipts.is_empty());
        assert_eq!(policy.keys().cloned().collect::<Vec<_>>(),
            vec![if state == "committed" { "next-node" } else { "peer-node" }.to_owned()]);
    }
}

#[tokio::test]
async fn successful_mutation_without_verified_install_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(root.path(), vec![
        Some(list(vec![], 7)), Some(mutation(vec![], "complete")), Some(list(vec![], 8)),
    ]);
    assert!(reconcile(root.path(), &socket, FLEET, "own-key", &view(), &BTreeSet::new())
        .await.unwrap_err().to_string().contains("final snapshot"));
    server.await.unwrap();
}

#[tokio::test]
async fn elapsed_preparation_removes_next_without_renewal_or_old_expiry() {
    let root = tempfile::tempdir().unwrap();
    let deadline = (now_ms() / 1000) as u64 - 1;
    let old = peer("peer-node", OWNER, &["st3-client-v0", "st3/fleet/test-fleet"]);
    let (socket, server) = serve(root.path(), vec![
        Some(list(vec![old.clone(), expiring_peer("next-node", deadline)], 7)),
        Some(mutation(vec![], "complete")),
        Some(list(vec![old], 8)),
    ]);
    reconcile(root.path(), &socket, FLEET, "own-key", &rotation_view("prepared", deadline), &BTreeSet::new())
        .await.unwrap();
    assert_eq!(server.await.unwrap()[1]["request"]["operations"],
        json!([{"op":"remove_peer","node_id":"next-node"}]));
}

#[test]
fn phase_changes_and_elapsed_preparation_cannot_replay_staging_transaction() {
    let deadline = 1300;
    let pending = Pending {
        created_unix_ms: now_ms(),
        request: json!({"operations":[
            {"op":"add_peer","node_id":"next-node"},
            {"op":"set_grants","node_id":"next-node","grants":["st3-client-v0","st3/fleet/test-fleet"]},
            {"op":"set_expiry","node_id":"next-node","expires_at":deadline}
        ]}),
    };
    for (state, now, expected) in [
        ("prepared", 1000, true),
        ("prepared", 1300, false),
        ("committed", 1000, false),
        ("aborted", 1000, false),
    ] {
        let view = rotation_view(state, deadline);
        let mut policy = desired(&view, "own-key", FLEET, &BTreeSet::new()).unwrap()
            .into_iter().map(|node| (node, grants(FLEET))).collect();
        let (expiries, _) = rotations(&view, "own-key", &mut policy, now).unwrap();
        assert_eq!(pending_matches_desired(&pending, &policy, &expiries), expected);
    }
}

#[test]
fn own_metadata_and_noncurrent_members_never_authorize_overlap_or_receipts() {
    let mut view = rotation_view("prepared", 1300);
    let rotation = view.members[1].endpoints.pop().unwrap();
    view.members[0].endpoints.push(rotation);
    let mut policy = DesiredPolicy::from([("peer-node".into(), grants(FLEET))]);
    let (expiries, receipts) = rotations(&view, "own-key", &mut policy, 1000).unwrap();
    assert!(expiries.is_empty());
    assert!(receipts.is_empty());
    let mut view = rotation_view("prepared", 1300);
    view.members[1].state = "ended".into();
    let (expiries, receipts) = rotations(&view, "own-key", &mut policy, 1000).unwrap();
    assert!(expiries.is_empty());
    assert!(receipts.is_empty());
}


fn serve_capability(root: &Path, capability: &'static str, replies: Vec<Value>)
    -> (PathBuf, tokio::task::JoinHandle<Vec<Value>>) {
    let socket = root.join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for mut reply in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            if request["request"]["operations"].as_array().is_some_and(|ops|
                ops.iter().any(|op| op["op"] == "set_expiry")) {
                reply = refusal("unsupported_operation");
            }
            let probing = request["type"] == "reachability_status";
            requests.push(request);
            if probing && capability != "old" {
                match capability {
                    "timeout" => tokio::time::sleep(CONTROL_TIMEOUT + Duration::from_millis(20)).await,
                    "malformed" => stream.write_all(b"not-json\n").await.unwrap(),
                    "truncated" => stream.write_all(b"{\"type\":\"reachability_status\"").await.unwrap(),
                    "closed" => (),
                    _ => unreachable!(),
                }
                continue;
            }
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).await.unwrap();
        }
        requests
    });
    (socket, server)
}

#[tokio::test]
async fn fabric_without_expiry_blocks_rekey_but_applies_unrelated_changes() {
    for capability in ["old", "timeout", "malformed", "truncated", "closed"] {
        let root = tempfile::tempdir().unwrap();
        let permanent = |node| peer(node, OWNER, &["st3-client-v0", "st3/fleet/test-fleet"]);
        let replies = vec![
            json!({"type":"reachability_status","version":"0.9.3+old"}),
            list(vec![permanent("peer-node"), permanent("gone")], 7),
            mutation(vec![], "complete"),
            list(vec![permanent("peer-node"), permanent("new-node")], 8),
        ];
        let (socket, server) = serve_capability(root.path(), capability, replies);
        let mut view = rotation_view("prepared", (now_ms() / 1000) as u64 + 300);
        view.members.push(member("new", "new-key", "new-node", "current"));
        let status = reconcile(root.path(), &socket, FLEET, "own-key", &view, &BTreeSet::new()).await.unwrap();
        assert_eq!(status.state, "rekey_blocked");
        assert!(status.message.unwrap().contains(if capability == "old" {
            "fabric_without_expiry"
        } else { "fabric_capability_unknown" }));
        let requests = server.await.unwrap();
        let operations = requests[2]["request"]["operations"].as_array().unwrap();
        assert!(operations.iter().any(|op| op["op"] == "remove_peer" && op["node_id"] == "gone"));
        assert!(operations.iter().any(|op| op["op"] == "add_peer" && op["node_id"] == "new-node"));
        assert!(operations.iter().all(|op| op["op"] != "set_expiry" && op["node_id"] != "next-node"));
    }
}

#[tokio::test]
async fn committed_cutover_survives_unknown_capability_without_any_policy_write() {
    let deadline = (now_ms() / 1000) as u64 + 300;
    for capability in ["timeout", "malformed"] {
        for owner in [OWNER.to_owned(), format!("{RENAMED_OWNER_PREFIX}{FLEET}")] {
            let root = tempfile::tempdir().unwrap();
            let services = if owner == OWNER { grants(FLEET) } else {
                BTreeSet::from([format!("{RENAMED_OWNER_PREFIX}{FLEET}"), "fractal-client-v0".into()])
            };
            let services = services.iter().map(String::as_str).collect::<Vec<_>>();
            let mut old = peer("peer-node", &owner, &services);
            old["trust_sources"][0]["expires_at"] = json!(deadline);
            let installed = vec![old, peer("next-node", &owner, &services)];
            let (socket, server) = serve_capability(root.path(), capability, vec![
                json!({"type":"reachability_status","version":"0.9.4"}), list(installed, 7),
            ]);
            let status = reconcile(root.path(), &socket, FLEET, "own-key",
                &rotation_view("committed", deadline), &BTreeSet::new()).await.unwrap();
            assert_eq!(status.state, "rekey_blocked");
            assert!(status.message.unwrap().contains("fabric_capability_unknown"));
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0]["type"], "reachability_status");
            assert_eq!(requests[1]["request"]["operation"], "list",
                "no expiry write, promotion, revocation or new grant may alter the installed cutover");
        }
    }
}

#[tokio::test]
async fn blocked_committed_cutover_never_replays_a_pending_replacement() {
    let root = tempfile::tempdir().unwrap();
    let policy = DesiredPolicy::from([("next-node".into(), grants(FLEET))]);
    let request = json!({"operation":"batch","owner":OWNER,"request_id":"replace-next",
        "operations":[{"op":"remove_peer","node_id":"next-node"},
            {"op":"add_peer","node_id":"next-node"},
            {"op":"set_grants","node_id":"next-node","grants":grants(FLEET)}]});
    let mut journal = Journal { pending: Some(Pending {
        request, created_unix_ms: now_ms(),
    }), ..Journal::default() };
    let (socket, server) = serve(root.path(), vec![Some(mutation(vec![], "complete"))]);
    resolve_pending(root.path(), &socket, &mut journal, &policy, &Expiries::new(),
        &BTreeSet::from(["next-node".into(), "peer-node".into()])).await.unwrap();
    assert_eq!(server.await.unwrap()[0]["request"]["operation"], "cancellation");
    assert!(journal.pending.is_none());
}
