use super::super::MemberView;
use super::*;
use tokio::net::UnixListener;

const FLEET: &str = "test-fleet";
const OWNER: &str = "st/fleet/test-fleet";

fn member(name: &str, key: &str, node: &str, state: &str) -> MemberView {
    MemberView {
        name: name.into(),
        member_key: key.into(),
        state: state.into(),
        mode: "listening".into(),
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
        desired(&view, "own-key", FLEET).unwrap(),
        BTreeSet::from(["peer-node".into()])
    );
    view.members[0].state = "ended".into();
    assert!(desired(&view, "own-key", FLEET).unwrap().is_empty());
    view.anchor = None;
    assert!(desired(&view, "own-key", FLEET).is_err());
}

#[test]
fn endpoint_protocol_cannot_inject_non_st_grants() {
    let mut view = view();
    view.members[1].endpoints[0]["protocol"] = json!("exec");
    assert!(
        desired(&view, "own-key", FLEET)
            .unwrap_err()
            .to_string()
            .contains("nonstandard")
    );
    assert_eq!(
        grants(FLEET),
        BTreeSet::from([
            "st3/fleet/test-fleet".into(),
            "st3/pty/test-fleet".into(),
            "st3-client-v0".into()
        ])
    );
}

#[tokio::test]
async fn adds_peer_and_replaces_exact_grants_in_one_atomic_batch() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![Some(list(vec![], 7)), Some(mutation(vec![], "complete"))],
    );
    let status = reconcile(&socket, FLEET, "own-key", &view()).await.unwrap();
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
            {"op":"set_grants", "node_id":"peer-node", "grants":["st3-client-v0", "st3/fleet/test-fleet", "st3/pty/test-fleet"]}
        ])
    );
}

#[tokio::test]
async fn rekey_removes_old_identity_and_adds_new_in_same_transaction() {
    let root = tempfile::tempdir().unwrap();
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![peer("old-node", OWNER, &["exec", "echo"])], 7)),
            Some(mutation(vec![], "complete")),
        ],
    );
    reconcile(&socket, FLEET, "own-key", &view()).await.unwrap();
    let requests = server.await.unwrap();
    let operations = requests[1]["request"]["operations"].as_array().unwrap();
    assert_eq!(
        operations[0],
        json!({"op":"remove_peer", "node_id":"old-node"})
    );
    assert_eq!(operations[1]["node_id"], "peer-node");
    assert_eq!(operations.len(), 3);
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
        ],
    );
    reconcile(&socket, FLEET, "own-key", &view()).await.unwrap();
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
            "st3/pty/test-fleet",
        ],
    );
    let (socket, server) = serve(root.path(), vec![Some(list(vec![exact], 7))]);
    assert_eq!(
        reconcile(&socket, FLEET, "own-key", &view())
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
        ],
    );
    reconcile(&socket, FLEET, "own-key", &view()).await.unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests[1], requests[2]);
}

#[tokio::test]
async fn runtime_disabled_is_clear_and_does_not_write_policy_or_fallback_files() {
    let root = tempfile::tempdir().unwrap();
    let static_path = root.path().join("peers.toml");
    std::fs::write(&static_path, "static policy").unwrap();
    let (socket, server) = serve(root.path(), vec![Some(refusal("runtime_peers_disabled"))]);
    let error = reconcile(&socket, FLEET, "own-key", &view())
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
        assert!(reconcile(&socket, FLEET, "own-key", &view()).await.is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn removal_reports_residual_static_access_even_after_previous_owner_removal() {
    let mut view = view();
    view.members[1].state = "ended".into();
    let root = tempfile::tempdir().unwrap();
    let retained = peer("peer-node", "static", &["st3/fleet/test-fleet"]);
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![retained.clone()], 7)),
            Some(list(vec![retained], 7)),
        ],
    );
    let status = reconcile(&socket, FLEET, "own-key", &view).await.unwrap();
    assert_eq!(status.state, "residual_access");
    assert!(status.message.unwrap().contains("/config/peers.toml"));
    server.await.unwrap();
}

#[tokio::test]
async fn repeated_withdrawal_cannot_hide_other_owner_access() {
    let root = tempfile::tempdir().unwrap();
    let retained = peer("peer-node", "other-tool", &["st3/pty/test-fleet"]);
    let (socket, server) = serve(
        root.path(),
        vec![
            Some(list(vec![retained.clone()], 7)),
            Some(list(vec![retained], 7)),
        ],
    );
    assert_eq!(
        withdraw(&socket, FLEET).await.unwrap().state,
        "residual_access"
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
        reconcile(&socket, FLEET, "own-key", &view())
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
        reconcile(&socket, FLEET, "own-key", &view)
            .await
            .unwrap_err()
            .to_string()
            .contains("limit 128")
    );
    assert_eq!(server.await.unwrap().len(), 1);
}
