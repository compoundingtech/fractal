//! Each test owns its store and Unix listener; no fleet daemon or seat is touched.
use super::*;
use crate::RuntimeStartsStore;

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
        Self::with_runtime(bound, true).await
    }

    async fn with_runtime(bound: bool, running: bool) -> Self {
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
        if running {
            fixture.runtime("chooser:one");
        }
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

    fn admit_launch(&self, operation: &str) {
        let selected = self.state.store.selected_desired_token(SEAT).unwrap();
        let desired = self.state.store.desired_subjects_named(&[SEAT.into()])
            .unwrap().remove(0);
        let mut start = crate::store::runtime_starts::RuntimeStartOperation {
            id: operation.into(),
            subject: SEAT.into(),
            owner: "api-incarnation-test".into(),
            body: json!({"subject":desired, "selected_token":selected,
                "desired_token":"api-incarnation-test",
                "member":{"runtime_id":SEAT, "terminal":true}}),
            revoked_by: None,
            finished: false,
            result: None,
            applied: false,
            cleanup_verified: false,
            dispatched: false,
            receipted: false,
        };
        self.state.store.admit_runtime_start(&mut start, 2).unwrap();
        assert_eq!(
            self.state.store.runtime_start_for(SEAT).unwrap().unwrap().id,
            operation
        );
    }

    fn register_launch(&self, admitted: &str, observed_operation: &str) -> bool {
        self.state.store.finish_runtime_start(admitted, &json!({
            "incarnation":"chooser:one", "verification":"operation-incarnation"
        })).unwrap();
        let input = |kind: &str, fields: BTreeMap<String, Value>| ClaimInput {
            subject: SEAT.into(),
            kind: kind.into(),
            actor: None,
            fields,
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        };
        let mut succeeded = input("runtime.action.succeeded", BTreeMap::from([
            ("action".into(), json!("start")),
            ("operation".into(), json!(observed_operation)),
            ("runtime_id".into(), json!(SEAT)),
            ("terminal".into(), json!(true)),
            ("desired_token".into(), json!("api-incarnation-test")),
            ("incarnation_id".into(), json!("chooser:one")),
        ]));
        succeeded.idempotency_key = Some(format!("start-succeeded:{admitted}"));
        let mut observed = input("runtime.observed", BTreeMap::from([
            ("status".into(), json!("running")),
            ("runtime_id".into(), json!(SEAT)),
            ("terminal".into(), json!(true)),
            ("incarnation_id".into(), json!("chooser:one")),
        ]));
        observed.idempotency_key = Some(format!("start-observed:{admitted}"));
        self.state.store.apply_runtime_start_result(admitted, &[succeeded, observed]).unwrap()
    }

    fn completion(&self, actor: &str, incarnation: &str, key: &str) -> Value {
        let document = self.state.store
            .put_document("doc/garden/fence-result", b"Garden result", &None, "fence-result")
            .unwrap();
        json!({"actor":actor, "incarnation":incarnation,
            "evidence":[format!("{}@{}", document.name, document.hash)],
            "idempotency_key":key})
    }

    async fn completion_refused(&self, body: Value, code: &str) {
        let snapshot = || {
            let mut view = serde_json::to_value(
                self.state.store.step_run(&self.step).unwrap().unwrap()
            ).unwrap();
            // Read-time clocks advance without a write; all durable fields must stay exact.
            let fields = view.as_object_mut().unwrap();
            fields.remove("execution_elapsed_ms");
            fields.remove("ready_age_ms");
            view
        };
        let before = snapshot();
        let error = self.client.post::<_, Value>(
            &format!("/v1/work/complete/{}", self.step), &body
        ).await.unwrap_err();
        assert_eq!(crate::client::api_error_code(&error), Some(code), "{error:#}");
        assert_eq!(
            snapshot(),
            before,
            "a refused launch must not mutate the work claim"
        );
        assert!(self.state.store.latest_claim(&self.step, Some("work.submitted"))
            .unwrap().is_none());
        assert!(self.state.store.cached_idempotency_response::<StepRunView>(
            body["idempotency_key"].as_str().unwrap()
        ).unwrap().is_none());
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

#[tokio::test]
async fn unknown_incarnation_cannot_submit_work() {
    let fixture = Fixture::with_runtime(false, false).await;
    fixture.completion_refused(
        fixture.completion(SEAT, "chooser:one", "unknown-launch"),
        "inactive-work-incarnation",
    ).await;
}

#[tokio::test]
async fn admitted_launch_without_running_registration_cannot_submit_work() {
    let fixture = Fixture::with_runtime(false, false).await;
    fixture.admit_launch("admitted-only");
    fixture.completion_refused(
        fixture.completion(SEAT, "chooser:one", "admitted-launch"),
        "inactive-work-incarnation",
    ).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn foreign_actor_cannot_submit_with_the_bound_launch() {
    let fixture = Fixture::new(true).await;
    fixture.completion_refused(
        fixture.completion("agent/garden/other", "chooser:one", "foreign-launch"),
        "foreign-agent-actor",
    ).await;
}

#[tokio::test]
async fn superseded_incarnation_cannot_submit_work() {
    let fixture = Fixture::new(false).await;
    fixture.runtime("chooser:two");
    fixture.completion_refused(
        fixture.completion(SEAT, "chooser:one", "superseded-launch"),
        "inactive-work-incarnation",
    ).await;
}

#[tokio::test]
async fn ended_incarnation_cannot_submit_work() {
    let fixture = Fixture::new(false).await;
    fixture.state.store.append_claim(&ClaimInput {
        subject: SEAT.into(),
        kind: "runtime.observed".into(),
        actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("exited")),
            ("incarnation_id".into(), json!("chooser:one")),
        ]),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    fixture.completion_refused(
        fixture.completion(SEAT, "chooser:one", "ended-launch"),
        "inactive-work-incarnation",
    ).await;
}

#[tokio::test]
async fn wrong_operation_cannot_register_a_launch_or_submit_work() {
    let fixture = Fixture::with_runtime(false, false).await;
    fixture.admit_launch("expected-operation");
    // The caller supplies the claim's correct incarnation. Registration, not that
    // supplied ID, must prevent a different operation from acquiring running authority.
    assert!(!fixture.register_launch("expected-operation", "other-operation"));
    for kind in ["runtime.action.succeeded", "runtime.observed"] {
        assert!(fixture.state.store.observations_for(SEAT, kind).unwrap().is_empty());
    }
    fixture.completion_refused(
        fixture.completion(SEAT, "chooser:one", "wrong-operation-launch"),
        "inactive-work-incarnation",
    ).await;
}

#[tokio::test]
async fn proven_current_launch_can_submit_work_with_an_exact_receipt() {
    let fixture = Fixture::with_runtime(false, false).await;
    fixture.admit_launch("current-operation");
    assert!(fixture.register_launch("current-operation", "current-operation"));
    let running = fixture.state.store.latest_claim(SEAT, Some("runtime.observed"))
        .unwrap().unwrap();
    assert_eq!(running.body["fields"]["status"], "running");
    assert_eq!(running.body["fields"]["incarnation_id"], "chooser:one");
    let start = fixture.state.store.observations_for(SEAT, "runtime.action.succeeded")
        .unwrap().into_iter().find(|claim| claim.body["fields"]["operation"] == "current-operation").unwrap();
    assert_eq!(start.body["fields"]["operation"], "current-operation");
    let body = fixture.completion(SEAT, "chooser:one", "current-launch");
    let submitted: StepRunView = fixture.client.post(
        &format!("/v1/work/complete/{}", fixture.step), &body
    ).await.unwrap();
    assert_eq!(submitted.submitted_incarnation.as_deref(), Some("chooser:one"));
    assert!(submitted.claimant.is_none());
    assert!(submitted.claim_incarnation.is_none());
    let receipt = fixture.state.store.latest_claim(&fixture.step, Some("work.submitted"))
        .unwrap().unwrap();
    let evidence: Vec<String> = serde_json::from_value(body["evidence"].clone()).unwrap();
    assert!(crate::model::work_submission_matches(
        &receipt, &fixture.step, submitted.attempt, SEAT, Some("chooser:one"), &evidence
    ));
    assert_eq!(receipt.actor.as_deref(), Some(SEAT));
    assert_eq!(receipt.body["fields"]["submitted_incarnation"], "chooser:one");
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
