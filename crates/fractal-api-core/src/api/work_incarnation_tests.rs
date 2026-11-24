//! Each test owns its store and Unix listener; no fleet daemon or seat is touched.
use super::*;

const SEAT: &str = "agent/garden/chooser";

pub struct Fixture {
    _root: tempfile::TempDir,
    state: AppState,
    client: crate::client::Client,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
    step: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(bound: bool) -> Self {
        fn chooser(_: u32, _: Option<&fractal_runtime::PeerProcessIdentity>) -> HarnessAncestry {
            HarnessAncestry::Bound(SEAT.into())
        }
        fn unbound(_: u32, _: Option<&fractal_runtime::PeerProcessIdentity>) -> HarnessAncestry {
            HarnessAncestry::Person
        }
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::state(root.path());
        state
            .store
            .apply_internal(
                &parse_intent(
                    r#"version 2
agent "garden/chooser" { workspace "/tmp"; harness "claude" {} }
agent "garden/other" { workspace "/tmp"; harness "claude" {} }
mission "catalog" state="ready" {
  goal "Choose a format."
  step "choose" { assigned-to "agent/garden/chooser"; goal "Ask for a format." }
}
"#,
                    "node",
                )
                .unwrap(),
                "fixture",
            )
            .unwrap();
        let run = state
            .store
            .create_mission_run(&MissionRunRequest {
                mission: "catalog".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/ada".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "run".into(),
            })
            .unwrap();
        let step = run.steps[0].subject.clone();
        state.store.set_step_state(&step, "ready", None).unwrap();
        state
            .store
            .work_action(
                &step,
                "claim",
                &WorkRequest {
                    wait_ms: None,
                    actor: Some(SEAT.into()),
                    incarnation: Some("chooser:one".into()),
                    summary: None,
                    reason: None,
                    evidence: vec![],
                    idempotency_key: "claim".into(),
                },
            )
            .unwrap();
        let socket = root.path().join("api.sock");
        let app = router(state.clone());
        let listener_socket = socket.clone();
        let store = state.store.clone();
        let server = tokio::spawn(async move {
            serve_unix_with_ancestor(
                &listener_socket,
                None,
                app,
                Some(store),
                if bound { chooser } else { unbound },
            )
            .await
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let fixture = Self {
            _root: root,
            state,
            client: crate::client::Client::unix(&socket),
            server,
            step,
        };
        fixture.runtime("chooser:one");
        fixture
    }

    fn runtime(&self, incarnation: &str) {
        self.state
            .store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "runtime.observed".into(),
                actor: Some("daemon/runtime".into()),
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        self.state
            .store
            .append_claim(&ClaimInput {
                subject: SEAT.into(),
                kind: "harness.observed".into(),
                actor: Some(SEAT.into()),
                fields: BTreeMap::from([
                    ("state".into(), json!("working")),
                    ("driver".into(), json!("claude")),
                    ("incarnation_id".into(), json!(incarnation)),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    fn ask(&self) -> Value {
        json!({"person":"person/ada", "title":"Choose a format", "reason":"The catalog needs a format.",
            "actor":SEAT, "step":self.step, "idempotency_key":"format",
            "request":{"version":1, "type":"choice", "question":"Choose a format.", "why_person":"Only the catalog owner can choose the format.",
                "answers":[{"id":"markdown", "label":"Markdown", "consequence":"Write Markdown."},{"id":"text", "label":"Text", "consequence":"Write plain text."}]}})
    }

    async fn refused(&self, body: Value, code: &str) -> String {
        let error = self
            .client
            .post::<_, Value>("/v1/work/ask", &body)
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains(code), "{message}");
        assert_eq!(
            self.state
                .store
                .step_run(&self.step)
                .unwrap()
                .unwrap()
                .status,
            "claimed"
        );
        message
    }
}

/// A completed shell launch is not ready merely because it spawned. Its first
/// exact-incarnation work claim must be able to establish running authority.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn custom_shell_starting_claim_requires_exact_live_start() {
    custom_shell_claim_cases(true).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn custom_shell_claim_precedes_reconcile_completion() {
    custom_shell_claim_cases(false).await;
}

#[cfg(target_os = "linux")]
async fn custom_shell_claim_cases(applied: bool) {
    use std::process::{Command, Stdio};
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for mismatch in ["none", "dead", "incarnation", "operation", "subject", "pid-token",
        "missing", "unbound", "refused-work", "revoked", "incomplete", "observation",
        "prior-observation", "request-retracted", "request-repaired"] {
        if applied && mismatch.starts_with("request-") { continue; }
        let root = tempfile::tempdir().unwrap();
        let state = super::tests::state(root.path());
        state.store.apply_internal(&parse_intent(r#"version 2
agent "garden/chooser" { workspace "/tmp"; command "sleep 60"; restart "never" }
mission "catalog" state="ready" {
  goal "Choose a format."
  step "choose" { assigned-to "agent/garden/chooser"; goal "Choose." }
}
"#, "node").unwrap(), "shell-fixture").unwrap();
        let run = state.store.create_mission_run(&MissionRunRequest {
            mission: "catalog".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/ada".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "run".into(),
        }).unwrap();
        let step = &run.steps[0].subject;
        state.store.set_step_state(step, "ready", None).unwrap();
        let desired = state.store.desired_subject_with_writer(SEAT).unwrap().unwrap().0;
        let member = desired.member.as_ref().unwrap();
        assert!(member.driver.is_none());
        let selected = state.store.selected_desired_token(SEAT).unwrap().unwrap();
        let mut process = Process(Command::new("sleep").arg("60").stdin(Stdio::null())
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let pid = process.0.id();
        let token = fractal_runtime::process_start_token(pid).unwrap();
        if mismatch == "dead" {
            process.0.kill().unwrap();
            process.0.wait().unwrap();
        }
        let incarnation = "shell:first";
        let operation = "shell-start";
        let before_start = (mismatch == "prior-observation").then(|| {
            state.store.append_claim(&ClaimInput {
                subject: SEAT.into(), kind: "runtime.observed".into(), actor: None,
                fields: serde_json::from_value(json!({
                    "status":"exited", "runtime_id":member.runtime_id,
                    "terminal":member.terminal, "incarnation_id":"predecessor",
                })).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap().id
        });
        let start_request = state.store.append_claim(&ClaimInput {
            subject: SEAT.into(), kind: "runtime.action.requested".into(), actor: None,
            fields: serde_json::from_value(json!({
                "action":"start", "operation":operation, "runtime_id":member.runtime_id,
                "terminal":member.terminal,
            })).unwrap(),
            evidence: vec![selected.clone()], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let body = json!({
            "subject": desired, "member": member, "selected_token": selected,
            "desired_token": selected,
            "start_request":start_request.id, "start_request_receipt":start_request,
        });
        let mut result = json!({
            "operation_id": if mismatch == "operation" { "other-start" } else { operation },
            "subject": if mismatch == "subject" { "agent/garden/other" } else { SEAT },
            "runtime_id": member.runtime_id, "terminal": member.terminal,
            "incarnation": incarnation, "verification": "operation-incarnation",
            "phase": "completed",
            "runtime_pid":pid,
            "runtime_pid_start_token": if mismatch == "pid-token" { token + 1 } else { token }.to_string(),
            "runtime_before_start":before_start,
            "runtime_start_fields":{
                "status":"starting", "runtime_id":member.runtime_id,
                "terminal":member.terminal, "incarnation_id":incarnation,
                "host":member.host, "adopted":false, "shutdown_timeout_ms":member.shutdown_timeout_ms,
                "exit_code":null, "exit_signal":null, "exit_evidence":null,
                "reachability":"reachable", "reason":null
            },
        });
        state.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-hc143d361eb67 owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
            "INSERT INTO local_runtime_starts(id,subject,owner,body,finished,result,applied,dispatched,receipted)
             VALUES(?1,?2,'fixture',?3,1,?4,?5,1,?5)",
            rusqlite::params![operation, SEAT, body.to_string(), result.to_string(), applied],
        ).unwrap();
        if applied {
            let succeeded = state.store.append_claim(&ClaimInput {
                subject: SEAT.into(), kind: "runtime.action.succeeded".into(), actor: None,
                fields: serde_json::from_value(json!({
                    "action":"start", "operation":operation, "runtime_id":member.runtime_id,
                    "incarnation_id":incarnation, "runtime_pid":pid,
                    "runtime_pid_start_token": if mismatch == "pid-token" { token + 1 } else { token }.to_string(),
                    "desired_token": selected,
                })).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            let starting = state.store.append_claim(&ClaimInput {
                subject: SEAT.into(), kind: "runtime.observed".into(), actor: None,
                fields: serde_json::from_value(json!({
                    "status":"starting", "runtime_id":member.runtime_id,
                    "terminal":member.terminal, "incarnation_id":incarnation,
                })).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
            result["result_receipts"] = json!([succeeded.id, starting.id]);
            result["runtime_observation"] = json!({"receipt":starting});
            // Also keep coverage for completed starts written by older workers.
            result.as_object_mut().unwrap().remove("runtime_pid");
            result.as_object_mut().unwrap().remove("runtime_pid_start_token");
            state.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h2519c27cb02c owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE local_runtime_starts SET result=?2 WHERE id=?1",
                rusqlite::params![operation, result.to_string()],
            ).unwrap();
        }
        if mismatch == "revoked" {
            state.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-hc3b8ab661222 owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE local_runtime_starts SET revoked_by='stop' WHERE id=?1", [operation],
            ).unwrap();
        }
        if mismatch == "incomplete" {
            state.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h2b140334f07c owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE local_runtime_starts SET finished=0 WHERE id=?1", [operation],
            ).unwrap();
        }
        if mismatch == "refused-work" {
            state.store.set_step_state(step, "completed", None).unwrap();
        }
        if mismatch == "observation" {
            state.store.append_claim(&ClaimInput {
                subject: SEAT.into(), kind: "runtime.observed".into(), actor: None,
                fields: serde_json::from_value(json!({
                    "status":"absent", "runtime_id":member.runtime_id,
                    "terminal":member.terminal, "incarnation_id":"other",
                })).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        if mismatch == "request-retracted" {
            state.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h21aa596bea94 owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
                "DELETE FROM local_observations WHERE subject=?1 AND kind='runtime.action.requested'",
                [SEAT],
            ).unwrap();
        }
        if mismatch == "request-repaired" {
            state.store.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-hd2266ef36f4b owner=fractal_api_core::api::work_incarnation_tests TODO=bound-or-reference-values-and-reclamation */
                "UPDATE local_observations SET body=json_set(body,'$.fields.reason','repaired')
                 WHERE subject=?1 AND kind='runtime.action.requested'", [SEAT],
            ).unwrap();
        }
        let request = WorkRequest {
            actor: Some(SEAT.into()),
            incarnation: (mismatch != "missing")
                .then(|| if mismatch == "incarnation" { "shell:old" } else { incarnation }.into()),
            wait_ms: None, summary: None, reason: None, evidence: vec![],
            idempotency_key: "first-claim".into(),
        };
        let response = super::work::post_work_action(
            State(state.clone()), AxumPath(("claim".into(), step.clone())),
            Query(super::work::WorkActionQuery { view: None }),
            (mismatch != "unbound").then(|| Extension(BoundAgent(SEAT.into()))), Json(request),
        ).await;
        let status = state.store.latest_claim(SEAT, Some("runtime.observed")).unwrap()
            .map_or(Value::Null, |claim| claim.body["fields"]["status"].clone());
        if matches!(mismatch, "none" | "prior-observation") {
            assert!(response.is_ok(), "exact live shell claim refused: {response:?}");
            assert_eq!(status, "running");
            assert_eq!(state.store.step_run(step).unwrap().unwrap().status, "claimed");
            if !applied {
                let flags: (bool, bool) = state.store.connection.write().query_row(
                    "SELECT applied,receipted FROM local_runtime_starts WHERE id=?1",
                    [operation], |row| Ok((row.get(0)?, row.get(1)?)),
                ).unwrap();
                assert_eq!(flags, (false, false), "claim must not depend on planner application");
                assert!(state.store.latest_observation(SEAT, "runtime.action.succeeded").unwrap().is_none());
            }
        } else {
            assert!(response.is_err(), "{mismatch} unexpectedly admitted");
            let expected = if mismatch == "observation" { json!("absent") }
                else if applied { json!("starting") } else { Value::Null };
            assert_eq!(status, expected, "{mismatch} published running");
            assert_eq!(state.store.step_run(step).unwrap().unwrap().status,
                if mismatch == "refused-work" { "completed" } else { "ready" });
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_seat_asks_without_an_incarnation() {
    let fixture = Fixture::new(true).await;
    let ask: StepRunView = fixture
        .client
        .post("/v1/work/ask", &fixture.ask())
        .await
        .unwrap();
    assert_eq!(ask.assigned_to.as_deref(), Some("person/ada"));
    assert_eq!(
        fixture
            .state
            .store
            .step_run(&fixture.step)
            .unwrap()
            .unwrap()
            .status,
        "waiting-person"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn explicit_stale_ask_names_expected_and_given_incarnations() {
    let fixture = Fixture::new(true).await;
    let mut body = fixture.ask();
    body["incarnation"] = json!("chooser:stale");
    let message = fixture.refused(body, "stale-work-ask").await;
    assert!(message.contains("chooser:one"), "{message}");
    assert!(message.contains("chooser:stale"), "{message}");
    assert!(message.contains(&fixture.step), "{message}");
    assert!(message.contains("omit --step"), "{message}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn restarted_seat_cannot_ask_with_its_old_incarnation() {
    let fixture = Fixture::new(true).await;
    fixture.runtime("chooser:two");
    // The previous claim has not yet been recovered: the runtime fence must still refuse it.
    let mut body = fixture.ask();
    body["incarnation"] = json!("chooser:one");
    let message = fixture.refused(body, "stale-work-ask").await;
    assert!(message.contains("chooser:two"), "{message}");
    assert!(message.contains("chooser:one"), "{message}");
    assert!(message.contains(&fixture.step), "{message}");
    assert!(message.contains("omit --step"), "{message}");
}

#[tokio::test]
async fn unbound_request_cannot_fill_the_claim_incarnation() {
    let fixture = Fixture::new(false).await;
    fixture.refused(fixture.ask(), "stale-work-ask").await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn another_seat_cannot_borrow_the_bound_incarnation() {
    let fixture = Fixture::new(true).await;
    let mut body = fixture.ask();
    body["actor"] = json!("agent/garden/other");
    fixture.refused(body, "foreign-agent-actor").await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn bound_work_actions_and_extensions_also_fill_missing_incarnations() {
    let fixture = Fixture::new(true).await;
    let progress: StepRunView = fixture.client.post(
        &format!("/v1/work/progress/{}", fixture.step),
        &json!({"actor":SEAT, "summary":"Preparing the question.", "idempotency_key":"progress"}),
    ).await.unwrap();
    assert_eq!(progress.claim_incarnation.as_deref(), Some("chooser:one"));
    let extended: StepRunView = fixture.client.post(
        &format!("/v1/work/extend/{}", fixture.step),
        &json!({"actor":SEAT, "by_ms":60_000, "reason":"More time to prepare.", "idempotency_key":"extend"}),
    ).await.unwrap();
    assert_eq!(extended.claim_incarnation.as_deref(), Some("chooser:one"));
}

#[tokio::test]
async fn an_unbound_caller_can_still_supply_an_explicit_claim_fence() {
    let fixture = Fixture::new(false).await;
    let mut body = fixture.ask();
    body["incarnation"] = json!("chooser:one");
    let ask: StepRunView = fixture.client.post("/v1/work/ask", &body).await.unwrap();
    assert_eq!(ask.assigned_to.as_deref(), Some("person/ada"));
}

#[tokio::test]
async fn submitted_incarnation_binds_the_receipt_and_survives_replay() {
    let fixture = Fixture::new(false).await;
    let document = fixture
        .state
        .store
        .put_document("doc/garden/result", b"Garden result", &None, "result")
        .unwrap();
    let evidence = vec![format!("{}@{}", document.name, document.hash)];
    for (incarnation, code) in [
        (None, "missing-work-incarnation"),
        (Some("chooser:two"), "wrong-work-incarnation"),
    ] {
        if let Some(incarnation) = incarnation {
            fixture.runtime(incarnation);
        }
        let body = json!({"actor":SEAT, "incarnation":incarnation, "evidence":evidence,
            "idempotency_key":format!("refused-{code}")});
        let error = fixture
            .client
            .post::<_, Value>(&format!("/v1/work/complete/{}", fixture.step), &body)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains(code));
        assert!(
            fixture
                .state
                .store
                .latest_claim(&fixture.step, Some("work.submitted"))
                .unwrap()
                .is_none()
        );
    }
    fixture.runtime("chooser:one");
    // A submission with no summary still carries its immutable author identity.
    let submitted: StepRunView = fixture.client.post(
        &format!("/v1/work/complete/{}", fixture.step),
        &json!({"actor":SEAT, "incarnation":"chooser:one", "evidence":evidence, "idempotency_key":"submit"}),
    ).await.unwrap();
    assert_eq!(
        submitted.submitted_incarnation.as_deref(),
        Some("chooser:one")
    );
    assert!(submitted.claimant.is_none());
    assert!(submitted.claim_incarnation.is_none());
    let receipt = fixture
        .state
        .store
        .latest_claim(&fixture.step, Some("work.submitted"))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.subject, fixture.step);
    assert_eq!(receipt.actor.as_deref(), Some(SEAT));
    assert_eq!(receipt.body["fields"]["attempt"], submitted.attempt);
    assert_eq!(
        receipt.body["fields"]["submitted_incarnation"],
        "chooser:one"
    );
    assert!(receipt.body["fields"]["claimant"].is_null());
    assert!(receipt.body["fields"]["claim_incarnation"].is_null());
    let matches = |claim: &ClaimRecord,
                   subject: &str,
                   attempt: u32,
                   actor: &str,
                   incarnation,
                   refs: &[String]| {
        crate::model::work_submission_matches(claim, subject, attempt, actor, incarnation, refs)
    };
    assert!(matches(
        &receipt,
        &fixture.step,
        submitted.attempt,
        SEAT,
        Some("chooser:one"),
        &evidence
    ));
    for incarnation in [None, Some(""), Some("chooser:two")] {
        assert!(!matches(
            &receipt,
            &fixture.step,
            submitted.attempt,
            SEAT,
            incarnation,
            &evidence
        ));
    }
    assert!(!matches(
        &receipt,
        "step-run/other/choose",
        submitted.attempt,
        SEAT,
        Some("chooser:one"),
        &evidence
    ));
    assert!(!matches(
        &receipt,
        &fixture.step,
        submitted.attempt + 1,
        SEAT,
        Some("chooser:one"),
        &evidence
    ));
    assert!(!matches(
        &receipt,
        &fixture.step,
        submitted.attempt,
        "agent/garden/other",
        Some("chooser:one"),
        &evidence
    ));
    assert!(!matches(
        &receipt,
        &fixture.step,
        submitted.attempt,
        SEAT,
        Some("chooser:one"),
        &[]
    ));
    fixture.state.store.replay_replication_graph().unwrap();
    let replayed = fixture
        .state
        .store
        .step_run(&fixture.step)
        .unwrap()
        .unwrap();
    assert_eq!(
        replayed.submitted_incarnation,
        submitted.submitted_incarnation
    );
    let mut other_attempt = replayed.clone();
    other_attempt.attempt += 1;
    fixture
        .state
        .store
        .enrich_work_response(&mut other_attempt)
        .unwrap();
    assert!(other_attempt.submitted_incarnation.is_none());
    let read: StepRunView = fixture
        .client
        .get(&format!("/v1/work-items/{}", fixture.step))
        .await
        .unwrap();
    assert_eq!(read.submitted_incarnation, submitted.submitted_incarnation);
    let claim_read: ClaimRecord = fixture.client.get(
        &format!("/v1/claims/by-id/{}", urlencoding::encode(&receipt.id)),
    ).await.unwrap();
    assert_eq!(claim_read.body, receipt.body);
    assert!(crate::model::work_submission_matches(&claim_read, &read.subject, read.attempt, SEAT, read.submitted_incarnation.as_deref(), &evidence));
    let mut unknown = receipt.clone();
    unknown.body["fields"]["submitted_incarnation"] = Value::Null;
    assert!(!crate::model::work_submission_matches(&unknown, &read.subject, read.attempt, SEAT, Some("chooser:one"), &evidence));

}
