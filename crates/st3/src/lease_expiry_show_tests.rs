use super::*;
use st3::model::{IntentInput, MissionRunRequest, WorkRequest};
use std::sync::Arc;

struct Clock;
impl Clock {
    fn at(store: &Store, at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        store.set_write_clock_at(at).unwrap();
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

async fn unrenewed_expired_work_show(
    store: &Arc<Store>,
    root: &std::path::Path,
    subject: &str,
    socket_name: &str,
) -> String {
    let state = st3::api::AppState {
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0).0,
        node: "node".into(),
        state_dir: root.into(),
        pty_root: root.join("pty"),
        pty_binary: "unused-pty".into(),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
    };
    let socket = root.join(socket_name);
    let server_socket = socket.clone();
    let (listening, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        st3::api::serve_unix_with_ready(&server_socket, st3::api::router(state), || {
            let _ = listening.send(());
        })
        .await
        .unwrap();
    });
    ready.await.unwrap();
    let client = generated_client(&Endpoint::Unix(socket), None).unwrap();
    let response = client.work_get(subject).await.unwrap();
    let ClientResource::Work(work) = response.value else {
        panic!("expected work")
    };
    let shown = render_client_work_detail(&work);
    server.abort();
    // Release the router's store reference before the caller reopens the file.
    let _ = server.await;
    assert_eq!(work.state, "ready", "{shown}");
    assert_eq!(work.attempt, 1, "{shown}");
    assert!(work.claimant.is_none(), "{shown}");
    assert!(work.claim_incarnation.is_none(), "{shown}");
    assert!(
        shown.contains("\nReason: the worker lease expired"),
        "{shown}"
    );
    assert!(shown.contains("2023-11-14T22:23:20Z"), "expiry: {shown}");
    assert!(
        shown.contains("no renewal was accepted for this claim"),
        "do not invent a renewal timestamp: {shown}"
    );
    assert!(
        shown.contains("claim accepted at 2023-11-14T22:13:20Z"),
        "{shown}"
    );
    shown
}

#[tokio::test(flavor = "current_thread")]
async fn lease_expiry_without_renewal_is_visible_before_repair_and_across_restarts() {
    const START: u128 = 1_700_000_000_000;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.sqlite3");
    let store = Store::open(&database, "node").unwrap();
    let _clock = Clock::at(&store, START);
    let source = r#"version 2
agent "worker" { workspace "/tmp"; command "true"; restart "never" }
resource "clock" { kind "custom.st3.document-source" }
mission "lease" state="ready" {
  goal "Show an unrenewed lease loss."
  step "work" { assigned-to "agent/node.worker" }
}
"#;
    let intent = st3::graph::parse_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &plan.subject_tokens, "unrenewed-publish")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "lease".into(),
            revision: None,
            workspace: root.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: Default::default(),
            idempotency_key: "unrenewed-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        actor: Some("agent/node.worker".into()),
        incarnation: Some("incarnation-A".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        wait_ms: None,
        idempotency_key: key.into(),
    };
    let claim = store
        .work_action(&subject, "claim", &request("unrenewed-claim"))
        .unwrap();
    assert_eq!(claim.attempt, 1);
    // The acknowledgement is enriched on the writer thread's real clock. Pin
    // the original lease from the actual accepted claim, before any repair.
    let claimed = store
        .latest_claim(&subject, Some("work.claimed"))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.body["fields"]["claim_incarnation"], "incarnation-A");
    assert_eq!(
        claimed.body["fields"]["claim_expires_at_unix_ms"],
        json!(START + 600_000)
    );
    let _expired = Clock::at(&store, START + 600_001);
    // The real client snapshot is pinned to accepted events, not reader wall time.
    // Advance that cut with an unrelated observation, leaving the work unrenewed.
    store
        .append_claim(&ClaimInput {
            subject: "resource/clock".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: serde_json::from_value(json!({"state":"ready"})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("unrenewed-expiry-observation".into()),
        })
        .unwrap();
    let store = Arc::new(store);
    let before = unrenewed_expired_work_show(&store, root.path(), &subject, "before.sock").await;
    assert_eq!(
        store
            .work_action(&subject, "complete", &request("unrenewed-late-complete"))
            .unwrap_err()
            .code,
        "work-not-claimed"
    );
    assert!(
        store
            .messages(Some("agent/node.worker"), true)
            .unwrap()
            .is_empty()
    );
    drop(store);

    let store = Arc::new(Store::open(&database, "node").unwrap());
    let _restarted = Clock::at(&store, START + 600_001);
    let restarted =
        unrenewed_expired_work_show(&store, root.path(), &subject, "restarted.sock").await;
    assert_eq!(
        restarted, before,
        "restart preserves effective expiry provenance before repair"
    );
    assert!(
        store
            .set_step_state(&subject, "ready", Some("the worker lease expired"))
            .unwrap()
    );
    assert!(
        !store
            .set_step_state(&subject, "ready", Some("the worker lease expired"))
            .unwrap()
    );
    let notices = store.messages(Some("agent/node.worker"), true).unwrap();
    assert_eq!(notices.len(), 1);
    let notice = notices[0].clone();
    assert_eq!(notice.title.as_deref(), Some("Work claim expired"));
    assert!(notice.content.contains("Claim incarnation: incarnation-A"));
    assert!(notice.content.contains("2023-11-14T22:23:20Z"));
    assert!(
        notice
            .content
            .contains("no renewal was accepted for this claim")
    );
    assert!(
        notice
            .content
            .contains("claim accepted at 2023-11-14T22:13:20Z")
    );
    drop(store);

    let store = Arc::new(Store::open(&database, "node").unwrap());
    let _repaired_restart = Clock::at(&store, START + 600_001);
    unrenewed_expired_work_show(&store, root.path(), &subject, "repaired.sock").await;
    assert!(
        !store
            .set_step_state(&subject, "ready", Some("the worker lease expired"))
            .unwrap()
    );
    let notices = store.messages(Some("agent/node.worker"), true).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].subject, notice.subject,
        "the durable notice survives restart without duplication"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn expired_work_keeps_its_reason_through_the_real_show_path_after_restart() {
    // Fixed accepted event times make the displayed expiry and renewal exact.
    const START: u128 = 1_700_000_000_000;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.sqlite3");
    let store = Store::open(&database, "node").unwrap();
    let _clock = Clock::at(&store, START);
    let source = r#"version 2
agent "worker" { workspace "/tmp"; command "true"; restart "never" }
resource "clock" { kind "custom.st3.document-source" }
mission "lease" state="ready" {
  goal "Make lease loss visible."
  step "work" { assigned-to "agent/node.worker" }
}
"#;
    let intent = st3::graph::parse_intent(source, "node").unwrap();
    let plan = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &plan.subject_tokens, "lease-show-publish")
        .unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "lease".into(),
            revision: None,
            workspace: root.path().display().to_string(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: Default::default(),
            idempotency_key: "lease-show-run".into(),
        })
        .unwrap();
    let subject = run.steps[0].subject.clone();
    store.set_step_state(&subject, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        wait_ms: None,
        idempotency_key: key.into(),
    };
    let claimed = store
        .work_action(&subject, "claim", &request("lease-show-claim"))
        .unwrap();
    assert_eq!(claimed.attempt, 1);
    let _renew_clock = Clock::at(&store, START + 60_000);
    let accepted_before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let renewed = store
        .work_action(&subject, "renew", &request("lease-show-renew"))
        .unwrap();
    let accepted_after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    assert_eq!(renewed.claim_expires_at_unix_ms, Some(START + 660_000));
    drop(store);
    let store = Arc::new(Store::open(&database, "node").unwrap());
    let _expired_clock = Clock::at(&store, START + 660_001);
    // Client snapshots use acceptance time. A separate daemon observation advances
    // that cut past expiry without changing the work or its renewal evidence.
    store
        .append_claim(&ClaimInput {
            subject: "resource/clock".into(),
            kind: "resource.observed".into(),
            actor: None,
            fields: serde_json::from_value(json!({"state":"ready"})).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("lease-show-after-restart-observation".into()),
        })
        .unwrap();
    assert_eq!(store.step_run(&subject).unwrap().unwrap().status, "ready");
    let state = st3::api::AppState {
        store: store.clone(),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0).0,
        node: "node".into(),
        state_dir: root.path().into(),
        pty_root: root.path().join("pty"),
        pty_binary: "unused-pty".into(),
        fleet_id: None,
        configured_peers: Vec::new(),
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
    };
    let socket = root.path().join("st3.sock");
    let server_socket = socket.clone();
    let (listening, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        st3::api::serve_unix_with_ready(&server_socket, st3::api::router(state), || {
            let _ = listening.send(());
        })
        .await
        .unwrap();
    });
    ready.await.unwrap();
    // These are exactly work show's generated client request and text renderer.
    let client = generated_client(&Endpoint::Unix(socket), None).unwrap();
    let response = client.work_get(&subject).await.unwrap();
    let ClientResource::Work(work) = response.value else {
        panic!("expected work")
    };
    let shown = render_client_work_detail(&work);
    eprintln!("SHOW AFTER RESTART:\n{shown}");
    assert_eq!(work.state, "ready");
    assert_eq!(
        work.attempt, 1,
        "lease loss does not consume a retry attempt"
    );
    assert!(work.claimant.is_none());
    assert!(
        shown.contains("\nReason: the worker lease expired"),
        "{shown}"
    );
    // Read the persisted acceptance independently after the missing-Reason assertion,
    // so the before-fix regression still fails at the original display defect.
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (accepted, request_stamp): (String, String) = connection.query_row(
        "SELECT accepted_at_unix_ms, accepted_request_at_unix_ms FROM local_work_lease_renewals WHERE subject=?1",
        [&subject], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let accepted = accepted.parse::<u128>().unwrap();
    assert!(
        (accepted_before..=accepted_after).contains(&accepted),
        "acceptance is inside the writer call, not its old request timestamp"
    );
    assert_eq!(request_stamp, (START + 60_000).to_string());
    let renewal_time = chrono::DateTime::from_timestamp_millis(i64::try_from(accepted).unwrap())
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
    assert!(
        shown.contains("2023-11-14T22:24:20Z"),
        "expiry is visible: {shown}"
    );
    assert!(
        shown.contains(&renewal_time),
        "last accepted renewal is visible: {shown}"
    );
    // The reconciler uses this durable transition. The notification and reason
    // commit together, and subsequent passes must neither resend nor erase them.
    assert!(
        store
            .set_step_state(&subject, "ready", Some("the worker lease expired"))
            .unwrap()
    );
    assert!(
        !store
            .set_step_state(&subject, "ready", Some("the worker lease expired"))
            .unwrap()
    );
    let mail = store.messages(Some("agent/node.worker"), true).unwrap();
    let notices: Vec<_> = mail
        .iter()
        .filter(|message| message.title.as_deref() == Some("Work claim expired"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("2023-11-14T22:24:20Z"));
    assert!(notices[0].content.contains(&renewal_time));
    assert!(notices[0].content.contains("attempt 1 unchanged"));
    let response = client.work_get(&subject).await.unwrap();
    let ClientResource::Work(work) = response.value else {
        panic!("expected work")
    };
    let after = render_client_work_detail(&work);
    assert_eq!(work.attempt, 1);
    assert!(
        after.contains("Reason: the worker lease expired"),
        "{after}"
    );
    assert!(after.contains(&renewal_time), "{after}");
    server.abort();
}
