/home/schickling/.megarepo/github.com/compoundingtech/smalltalk/refs/heads/fleet-invite-pairing/crates/st-fabric-adapter/src/pairing_tests.rs:

use super::*;
use tokio::net::UnixListener;

fn request(node: &str) -> JoinRequest {
    JoinRequest {
        protocol: "st3-join-v1".into(),
        invite: "invite".into(),
        name: "joining".into(),
        mode: "listening".into(),
        member_key: "member-key".into(),
        ephemeral: "ephemeral".into(),
        writer_head: None,
        build: "test".into(),
        migrate: false,
        node_id: Some(node.into()),
        proof: "proof".into(),
        signature: "signature".into(),
    }
}
fn journal() -> Journal {
    Journal {
        fleet: "fleet".into(),
        creation: "creation".into(),
        deadline: 600,
        attempt: Some("attempt".into()),
        response: None,
        request: None,
        confirmation: None,
        terminal: false,
        confirmed: false,
        bound_node: None,
    }
}
fn status() -> Value {
    json!({"state":"pending","attempt_id":"attempt","bound_node_id":"node","attempt_revision":2})
}
fn exchange(sequence: u64, payload: &[u8]) -> Value {
    json!({"type":"pairing","reply":{"error":null,"status":status(),"exchange":{"message":{"sequence":sequence,"digest":digest(payload),"payload":payload}}}})
}
async fn fabric(listener: UnixListener, script: Vec<(Value, Value)>) {
    for (expected, reply) in script {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
        let mut bytes = serde_json::to_vec(&reply).unwrap();
        bytes.push(b'\n');
        stream.get_mut().write_all(&bytes).await.unwrap();
    }
}
fn local_request(request: Value) -> Value {
    json!({"type":"pairing","request":request})
}
fn receive(ack: Option<u64>) -> Value {
    let mut request =
        json!({"operation":"pair_receive","owner":"st/fleet/fleet","attempt_id":"attempt"});
    if let Some(ack) = ack {
        request["ack"] = json!(ack);
    }
    local_request(request)
}

#[tokio::test]
async fn issue_only_reserves_immutable_policy_without_runtime_grants() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let expected = json!({"operation":"issue","owner":"st/fleet/fleet","creation_id":"creation","deadline":600,"policy":{"grants":["st3/fleet/fleet","st3/pty/fleet","st3-client-v0"],"name":"joining","direction":"normal"}});
    let reply = json!({"type":"pairing","reply":{"error":null,"status":{"state":"issued"},"secret":"one-shot"}});
    let server = tokio::spawn(fabric(
        listener,
        vec![(local_request(expected.clone()), reply)],
    ));
    let result = local(
        &socket,
        issue_request("fleet", "creation", 600, Some("joining")),
    )
    .await
    .unwrap();
    assert_eq!(result["status"]["state"], "issued");
    server.await.unwrap();
}

#[tokio::test]
async fn wrong_signed_identity_never_reaches_admission_or_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let bytes = serde_json::to_vec(&request("impostor")).unwrap();
    let server = tokio::spawn(fabric(listener, vec![(receive(None), exchange(1, &bytes))]));
    let mut journal = journal();
    let error = process(
        &reqwest::Client::new(),
        &socket,
        &dir.path().join("journal.json"),
        &mut journal,
        &status(),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("differs from Fabric bound identity")
    );
    assert!(journal.confirmation.is_none());
    assert!(journal.response.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn sealed_answer_must_be_acknowledged_before_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut journal = journal();
    journal.request = Some(request("node"));
    journal.response = Some(b"sealed-answer".to_vec());
    let send = local_request(
        json!({"operation":"pair_send","owner":"st/fleet/fleet","attempt_id":"attempt","sequence":1,"digest":digest(b"sealed-answer"),"payload":b"sealed-answer".to_vec()}),
    );
    let empty = json!({"type":"pairing","reply":{"error":null,"status":status(),"exchange":{"message":null}}});
    let server = tokio::spawn(fabric(
        listener,
        vec![(send, empty.clone()), (receive(Some(1)), empty)],
    ));
    process(
        &reqwest::Client::new(),
        &socket,
        &dir.path().join("journal.json"),
        &mut journal,
        &status(),
    )
    .await
    .unwrap();
    assert!(journal.confirmation.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn acknowledged_happy_path_confirms_exact_atomic_selectors() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let st_socket = dir.path().join("st.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let st_listener = UnixListener::bind(&st_socket).unwrap();
    let answer = JoinResponse {
        protocol: "st3-join-v1".into(),
        sponsor: "sponsor".into(),
        sponsor_key: "sponsor-key".into(),
        ephemeral: "ephemeral".into(),
        nonce: "nonce".into(),
        sealed: "sealed-answer".into(),
        signature: "signature".into(),
    };
    let payload = serde_json::to_vec(&answer).unwrap();
    let answer_body = payload.clone();
    let st = tokio::spawn(async move {
        for index in 0..3 {
            let (stream, _) = st_listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            if index == 0 {
                assert!(line.starts_with("POST /v1/internal/fleet/pairing-admission "));
            } else {
                assert!(line.starts_with("GET /v1/internal/fleet/leave-state "));
            }
            let mut content_length = 0;
            loop {
                line.clear();
                stream.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(length) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = length.trim().parse::<usize>().unwrap();
                }
            }
            if content_length != 0 {
                let mut body = vec![0; content_length];
                stream.read_exact(&mut body).await.unwrap();
                let admission: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(admission["node_id"], "node");
                assert_eq!(admission["request"]["node_id"], "node");
            }
            let body = if index == 0 {
                answer_body.clone()
            } else {
                serde_json::to_vec(&json!({"fleet_id":"fleet","own_member_key":"sponsor-key","persistence_error":null,"members":[],"membership":{"anchor":"anchor","legacy_removed":[],"members":[{"name":"joining","member_key":"member-key","node_id":"node","state":"current","mode":"listening","endpoints":[],"start":1}]}})).unwrap()
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.get_mut().write_all(header.as_bytes()).await.unwrap();
            stream.get_mut().write_all(&body).await.unwrap();
        }
    });
    let mut journal = journal();
    let send = local_request(
        json!({"operation":"pair_send","owner":"st/fleet/fleet","attempt_id":"attempt","sequence":1,"digest":digest(&payload),"payload":payload}),
    );
    let ack = serde_json::to_vec(&json!({"answer_digest":digest(&payload)})).unwrap();
    let confirmation = json!({"operation":"confirm","owner":"st/fleet/fleet","attempt_id":"attempt","expected_node_id":"node","expected_attempt_revision":2,"expected_policy_revision":17,"request_id":"confirm-attempt-17"});
    let reply = json!({"type":"pairing","reply":{"error":null,"status":status()}});
    let server = tokio::spawn(fabric(
        listener,
        vec![
            (
                receive(None),
                exchange(1, &serde_json::to_vec(&request("node")).unwrap()),
            ),
            (send, reply.clone()),
            (receive(Some(1)), exchange(2, &ack)),
            (
                json!({"type":"runtime_peers","request":{"operation":"list"}}),
                json!({"type":"runtime_peers","reply":{"error":null,"snapshot":{"policy_revision":17,"fault":null}}}),
            ),
            (
                local_request(confirmation.clone()),
                json!({"type":"pairing","reply":{"error":null,"status":{"state":"confirmed"}}}),
            ),
        ],
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .unix_socket(st_socket)
        .build()
        .unwrap();
    let path = dir.path().join("journal.json");
    process(&client, &socket, &path, &mut journal, &status())
        .await
        .unwrap();
    assert_eq!(journal.confirmation, Some(confirmation));
    let persisted: Journal = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(persisted.confirmation, journal.confirmation);
    server.await.unwrap();
    st.await.unwrap();
}

#[tokio::test]
async fn uncertain_confirmation_observes_receipt_without_reinstalling_policy() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut journal = journal();
    journal.confirmation = Some(json!({"operation":"confirm","request_id":"confirm-attempt"}));
    journal.request = Some(request("node"));
    let path = journal_path(dir.path(), "creation");
    super::super::save(&path, &journal).unwrap();
    let query = local_request(
        json!({"operation":"query","owner":"st/fleet/fleet","creation_id":"creation"}),
    );
    let server = tokio::spawn(fabric(
        listener,
        vec![(
            query,
            json!({"type":"pairing","reply":{"error":null,"status":{"state":"confirmed","bound_node_id":"node"}}}),
        )],
    ));
    let blocked = tick(&reqwest::Client::new(), &socket, dir.path())
        .await
        .unwrap();
    assert!(blocked.blocked.is_empty());
    assert!(
        blocked
            .confirmed
            .contains(&("fleet".into(), "member-key".into(), "node".into()))
    );
    let persisted: Journal = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(persisted.terminal && persisted.confirmed);
    server.await.unwrap();
}

#[tokio::test]
async fn rejected_admitted_identity_stays_fenced_from_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal();
    journal.terminal = true;
    journal.bound_node = Some("node".into());
    super::super::save(&journal_path(dir.path(), "creation"), &journal).unwrap();
    let blocked = tick(
        &reqwest::Client::new(),
        &dir.path().join("absent.sock"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_eq!(blocked.blocked, BTreeSet::from(["node".into()]));
}

#[test]
fn admission_binding_requires_exact_signed_node_even_before_endpoints_exist() {
    let mut snapshot: super::super::LeaveSnapshot = serde_json::from_value(json!({
        "fleet_id":"fleet","own_member_key":"sponsor-key","persistence_error":null,"members":[],
        "membership":{"anchor":"anchor","legacy_removed":[],"members":[{
            "name":"joining","member_key":"member-key","node_id":"node",
            "state":"current","mode":"listening","endpoints":[],"start":1
        }]}
    }))
    .unwrap();
    assert!(bound(&snapshot, "joining", "member-key", "node", "fleet").is_ok());
    assert!(bound(&snapshot, "joining", "member-key", "impostor", "fleet").is_err());
    snapshot.membership.members[0].node_id = None;
    assert!(bound(&snapshot, "joining", "member-key", "node", "fleet").is_err());
}

#[tokio::test]
async fn conclusive_revision_conflict_allows_replanning_but_not_uncertain_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut journal = journal();
    let confirmation = json!({"operation":"confirm","request_id":"confirm-attempt-17"});
    journal.confirmation = Some(confirmation.clone());
    let server = tokio::spawn(fabric(
        listener,
        vec![(
            local_request(confirmation.clone()),
            json!({"type":"pairing","reply":{"error":{"code":"policy_revision_conflict"}}}),
        )],
    ));
    let path = dir.path().join("journal.json");
    assert!(
        finish_confirmation(&socket, &path, &mut journal, confirmation.clone())
            .await
            .is_err()
    );
    assert!(journal.confirmation.is_none());
    server.await.unwrap();
    journal.confirmation = Some(confirmation.clone());
    assert!(
        finish_confirmation(
            &dir.path().join("absent.sock"),
            &path,
            &mut journal,
            confirmation
        )
        .await
        .is_err()
    );
    assert!(journal.confirmation.is_some());
}

#[tokio::test]
async fn concurrent_invite_lock_fences_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let _lock = pairing_lock(dir.path(), false).unwrap();
    assert!(
        tick(
            &reqwest::Client::new(),
            &dir.path().join("absent.sock"),
            dir.path()
        )
        .await
        .is_err()
    );
}
