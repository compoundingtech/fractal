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

#[test]
fn permit_and_reciprocal_policy_grants_are_client_only() {
    assert_eq!(grants("fleet"), json!(["st3/fleet/fleet", "st3-client-v0"]));
}

#[tokio::test]
async fn issue_only_reserves_immutable_policy_without_runtime_grants() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let expected = json!({"operation":"issue","owner":"st/fleet/fleet","creation_id":"creation","deadline":600,"policy":{"grants":["st3/fleet/fleet","st3-client-v0"],"name":"joining","direction":"normal"}});
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

#[tokio::test]
async fn pruned_receipts_terminalize_durably_without_blocking_other_fleets() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    for creation in ["pruned-issued", "pruned-confirmation", "healthy"] {
        let mut journal = journal();
        journal.creation = creation.into();
        journal.fleet = creation.into();
        if creation == "pruned-confirmation" {
            journal.bound_node = Some("uncertain-node".into());
            journal.request = Some(request("uncertain-node"));
            journal.confirmation = Some(json!({"operation":"confirm","request_id":"uncertain"}));
        } else if creation == "healthy" {
            journal.request = Some(request("healthy-node"));
        }
        super::super::save(&journal_path(dir.path(), creation), &journal).unwrap();
    }
    let server = tokio::spawn(async move {
        let mut seen = BTreeSet::new();
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let query: Value = serde_json::from_str(&line).unwrap();
            let creation = query["request"]["creation_id"].as_str().unwrap();
            assert_eq!(
                query,
                local_request(json!({
                    "operation":"query","owner":owner(creation),"creation_id":creation
                }))
            );
            assert!(seen.insert(creation.to_owned()));
            let reply = if creation == "healthy" {
                json!({"type":"pairing","reply":{"error":null,"status":{
                    "state":"confirmed","bound_node_id":"healthy-node"
                }}})
            } else {
                assert!(matches!(creation, "pruned-issued" | "pruned-confirmation"));
                json!({"type":"pairing","reply":{"status":null,"error":{
                    "code":"invalid_argument","class":"usage",
                    "message":"unknown or expired creation_id"
                }}})
            };
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            stream.get_mut().write_all(&bytes).await.unwrap();
        }
    });
    let pairing = tick(&reqwest::Client::new(), &socket, dir.path())
        .await
        .unwrap();
    assert_eq!(pairing.blocked, BTreeSet::from(["uncertain-node".into()]));
    assert_eq!(
        pairing.confirmed,
        BTreeSet::from([("healthy".into(), "member-key".into(), "healthy-node".into())])
    );
    server.await.unwrap();
    for creation in ["pruned-issued", "pruned-confirmation", "healthy"] {
        let persisted: Journal = serde_json::from_slice(
            &std::fs::read(journal_path(dir.path(), creation)).unwrap(),
        )
        .unwrap();
        assert!(persisted.terminal);
        assert_eq!(persisted.confirmed, creation == "healthy");
        if creation == "pruned-confirmation" {
            assert_eq!(persisted.bound_node.as_deref(), Some("uncertain-node"));
            assert!(persisted.confirmation.is_some());
            assert!(persisted.request.is_some());
        }
    }
    // Terminal receipts no longer need Fabric, and restart retains both the
    // uncertain identity fence and the independent fleet's confirmed binding.
    let recovered = tick(
        &reqwest::Client::new(),
        &dir.path().join("absent.sock"),
        dir.path(),
    )
    .await
    .unwrap();
    assert_eq!(recovered.blocked, pairing.blocked);
    assert_eq!(recovered.confirmed, pairing.confirmed);
    let mut snapshot = serde_json::from_value::<super::super::LeaveSnapshot>(json!({
        "fleet_id":"healthy","own_member_key":"sponsor-key","persistence_error":null,"members":[],
        "membership":{"anchor":"anchor","legacy_removed":[],"members":[{
            "name":"joining","member_key":"member-key","node_id":"healthy-node",
            "state":"current","mode":"listening","endpoints":[],"start":1
        }]}
    }))
    .unwrap();
    super::super::fence_pairing(&mut snapshot, &recovered);
    assert_eq!(snapshot.membership.members[0].endpoints.len(), 1);
}

#[tokio::test]
async fn missing_receipt_before_deadline_or_unrelated_error_remains_uncertain() {
    for (deadline, code, message) in [
        (u64::MAX, "invalid_argument", "unknown or expired creation_id"),
        (1, "invalid_argument", "invalid creation_id"),
        (1, "runtime_state_unavailable", "unknown or expired creation_id"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("fabric.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let mut journal = journal();
        journal.deadline = deadline;
        journal.bound_node = Some("uncertain-node".into());
        journal.confirmation = Some(json!({"request_id":"uncertain"}));
        let path = journal_path(dir.path(), "creation");
        super::super::save(&path, &journal).unwrap();
        let server = tokio::spawn(fabric(
            listener,
            vec![(
                local_request(json!({
                    "operation":"query","owner":"st/fleet/fleet","creation_id":"creation"
                })),
                json!({"type":"pairing","reply":{"status":null,"error":{
                    "code":code,"message":message
                }}}),
            )],
        ));
        assert!(
            tick(&reqwest::Client::new(), &socket, dir.path())
                .await
                .is_err()
        );
        let persisted: Journal =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(!persisted.terminal);
        assert_eq!(persisted.bound_node, journal.bound_node);
        assert_eq!(persisted.confirmation, journal.confirmation);
        server.await.unwrap();
    }
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

#[test]
fn fresh_confirmed_invite_recovers_same_node_without_unfencing_other_admissions() {
    let snapshot = || {
        serde_json::from_value::<super::super::LeaveSnapshot>(json!({
            "fleet_id":"fleet","own_member_key":"sponsor-key","persistence_error":null,"members":[],
            "membership":{"anchor":"anchor","legacy_removed":[],"members":[{
                "name":"joining","member_key":"member-key","node_id":"node",
                "state":"current","mode":"listening",
                "endpoints":[{"transport":"fabric","node":"node","protocol":"st3/fleet/fleet"}],
                "start":1
            }]}
        }))
        .unwrap()
    };
    let mut pairing = PairingState {
        blocked: BTreeSet::from(["node".into()]),
        confirmed: BTreeSet::new(),
    };
    let mut expired = snapshot();
    super::super::fence_pairing(&mut expired, &pairing);
    assert!(expired.membership.members[0].endpoints.is_empty());

    pairing
        .confirmed
        .insert(("fleet".into(), "member-key".into(), "node".into()));
    let mut recovered = snapshot();
    recovered.membership.members[0].endpoints.clear();
    super::super::fence_pairing(&mut recovered, &pairing);
    assert_eq!(recovered.membership.members[0].endpoints.len(), 1);

    for (fleet, key, node) in [
        ("other-fleet", "member-key", "node"),
        ("fleet", "other-key", "node"),
        ("fleet", "member-key", "other-node"),
    ] {
        pairing.confirmed = BTreeSet::from([(fleet.into(), key.into(), node.into())]);
        let mut different = snapshot();
        super::super::fence_pairing(&mut different, &pairing);
        assert!(different.membership.members[0].endpoints.is_empty());
    }
}

#[test]
fn confirmed_bootstrap_never_overrides_a_published_endpoint_rekey_or_removal() {
    let snapshot = || {
        serde_json::from_value::<super::super::LeaveSnapshot>(json!({
            "fleet_id":"fleet","own_member_key":"sponsor-key","persistence_error":null,"members":[],
            "membership":{"anchor":"anchor","legacy_removed":[],"members":[{
                "name":"joining","member_key":"member-key","node_id":"old-node",
                "state":"current","mode":"listening","endpoints_published":true,
                "endpoints":[],"start":1
            }]}
        }))
        .unwrap()
    };
    let pairing = PairingState {
        blocked: BTreeSet::new(),
        confirmed: BTreeSet::from([("fleet".into(), "member-key".into(), "old-node".into())]),
    };
    let mut removed = snapshot();
    super::super::fence_pairing(&mut removed, &pairing);
    assert!(removed.membership.members[0].endpoints.is_empty());

    let mut rekeyed = snapshot();
    rekeyed.membership.members[0].endpoints.push(json!({
        "transport":"fabric","node":"new-node","protocol":"st3/fleet/fleet"
    }));
    super::super::fence_pairing(&mut rekeyed, &pairing);
    assert_eq!(rekeyed.membership.members[0].endpoints.len(), 1);
    assert_eq!(
        rekeyed.membership.members[0].endpoints[0]["node"],
        "new-node"
    );
}

#[tokio::test]
async fn crash_recovery_observes_confirmed_receipt_after_original_deadline_without_presenting() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let query = json!({
        "type":"pairing_remote","gateway":"sponsor-node",
        "request":{"operation":"status","attempt_id":"attempt"}
    });
    let reply = json!({
        "type":"pairing","reply":{"error":null,"status":{
            "state":"confirmed","bound_node_id":"joining-node","deadline":1
        }}
    });
    let server = tokio::spawn(fabric(listener, vec![(query, reply)]));
    assert!(
        resume_status(&socket, "sponsor-node", "attempt", "joining-node")
            .await
            .unwrap()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn supersession_status_uses_same_daemon_and_pinned_gateway_without_presenting() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let retained = json!({
        "state":"rejected","terminal_reason":"exchange_lost",
        "attempt_id":"old-attempt","bound_node_id":"joining-node","deadline":1
    });
    let server = tokio::spawn(fabric(
        listener,
        vec![
            (
                json!({"type":"status"}),
                json!({"type":"status","node_id":"joining-node"}),
            ),
            (
                json!({"type":"pairing_remote","gateway":"old-gateway",
            "request":{"operation":"status","attempt_id":"old-attempt"}}),
                json!({"type":"pairing","reply":{"error":null,"status":retained.clone()}}),
            ),
        ],
    ));
    let request = StatusRequest {
        gateway: "old-gateway".into(),
        attempt: "old-attempt".into(),
        node_id: "joining-node".into(),
    };
    let observed = query_status(&socket, &request).await.unwrap();
    assert_eq!(
        observed,
        json!({"node_id":"joining-node","status":retained})
    );
    server.await.unwrap();
}

#[tokio::test]
async fn supersession_status_refuses_a_changed_local_daemon_identity() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("fabric.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(fabric(
        listener,
        vec![(
            json!({"type":"status"}),
            json!({"type":"status","node_id":"replacement-node"}),
        )],
    ));
    let request = StatusRequest {
        gateway: "old-gateway".into(),
        attempt: "old-attempt".into(),
        node_id: "original-node".into(),
    };
    assert!(query_status(&socket, &request).await.is_err());
    server.await.unwrap();
}
