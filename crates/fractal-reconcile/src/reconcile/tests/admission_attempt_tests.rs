use super::*;
use serde_json::json;

const SEAT: &str = "agent/node.fixture";

#[test]
fn restart_diagnostic_distinguishes_verified_start_then_exit_from_interrupted_attempt() {
    for (diagnostic_incarnation, receipt_operation, expect_detail, expect_started) in [
        ("new", None, true, false),
        ("old", None, false, false),
        ("old", Some("attempt-operation"), false, true),
        ("old", Some("unrelated-operation"), false, false),
    ] {
        let store = Arc::new(Store::open_memory("node").unwrap());
        apply_source(
            &store,
            "version 2\nagent \"fixture\" { workspace \"/tmp\"; restart \"never\"; harness \"omp\" {} }",
            "admission-attempt",
        );
        let desired = store.desired_subjects().unwrap().remove(0);
        let runtime = Arc::new(FakeRuntime::default());
        let reconciler = Reconciler::new(
            store.clone(),
            runtime.clone(),
            "node".into(),
            Arc::new(Notify::new()),
        );
        let append = |kind: &str,
                      actor: Option<&str>,
                      fields: BTreeMap<String, Value>,
                      evidence: Vec<String>,
                      key: Option<String>| {
            store
                .append_claim(&ClaimInput {
                    subject: SEAT.into(),
                    kind: kind.into(),
                    actor: actor.map(str::to_owned),
                    fields,
                    evidence,
                    expected_subject: None,
                    idempotency_key: key,
                })
                .unwrap()
        };
        let request = append(
            "runtime.action.requested",
            Some("person/operator"),
            BTreeMap::from([
                ("action".into(), json!("restart")),
                ("incarnation_id".into(), json!("old")),
            ]),
            vec![store.selected_desired_token(SEAT).unwrap().unwrap()],
            None,
        );
        let attempt = append(
            "runtime.action.requested",
            request.actor.as_deref(),
            BTreeMap::from([
                ("action".into(), json!("start")),
                ("operation".into(), json!("attempt-operation")),
            ]),
            vec![request.id.clone()],
            Some(format!("agent-restart-attempt:{}", request.id)),
        );
        if let Some(operation) = receipt_operation {
            append(
                "runtime.action.succeeded",
                None,
                BTreeMap::from([
                    ("action".into(), json!("start")),
                    ("operation".into(), json!(operation)),
                    ("operation_status".into(), json!("verified")),
                    ("incarnation_id".into(), json!("new")),
                ]),
                vec![attempt.id.clone()],
                None,
            );
            append(
                "harness.observed",
                Some(SEAT),
                BTreeMap::from([
                    ("state".into(), json!("ended")),
                    ("driver".into(), json!("omp")),
                    ("incarnation_id".into(), json!("new")),
                    ("reason".into(), json!("driver-stopped")),
                    ("exit".into(), json!("exit 143")),
                ]),
                vec![],
                None,
            );
        }
        append(
            "harness.diagnostic",
            Some(SEAT),
            BTreeMap::from([
                ("code".into(), json!("harness-admission-failed")),
                ("severity".into(), json!("error")),
                ("status".into(), json!("failed")),
                ("incarnation_id".into(), json!(diagnostic_incarnation)),
                (
                    "reason".into(),
                    json!("phase=version outcome=timeout; fixture refusal"),
                ),
            ]),
            vec![],
            None,
        );
        append(
            "runtime.observed",
            Some(SEAT),
            BTreeMap::from([
                ("runtime_id".into(), json!("node.fixture")),
                ("status".into(), json!("exited")),
                ("exit_code".into(), json!(1)),
                ("incarnation_id".into(), json!("new")),
            ]),
            vec![],
            None,
        );
        assert_eq!(
            reconciler
                .restart_admission_error(SEAT, &attempt, "old")
                .unwrap()
                .is_some(),
            expect_detail
        );
        let error = reconciler
            .reconcile_requested_restart(&desired, desired.member.as_ref().unwrap(), None, None)
            .unwrap_err()
            .to_string();
        if expect_started {
            assert!(error.contains("the replacement started and then exited: driver-stopped"), "{error}");
            assert!(!error.contains("interrupted before"), "{error}");
        } else if expect_detail {
            assert!(error.contains("phase=version outcome=timeout"), "{error}");
            assert!(!error.contains("interrupted before"), "{error}");
        } else {
            assert!(error.contains("interrupted before"), "{error}");
            assert!(!error.contains("fixture refusal"), "{error}");
        }
        assert!(runtime.starts.lock().unwrap().is_empty());
        assert!(runtime.stops.lock().unwrap().is_empty());
        // Reconciliation may repeat or the daemon may restart. The retained failure is stable.
        let again = reconciler
            .reconcile_requested_restart(&desired, desired.member.as_ref().unwrap(), None, None)
            .unwrap_err()
            .to_string();
        assert_eq!(again, error);
        assert_eq!(
            store
                .claims_for(SEAT, Some("runtime.action.failed"))
                .unwrap()
                .len(),
            1
        );
    }
}
