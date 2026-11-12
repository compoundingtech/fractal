use super::*;
use serde_json::json;

const SEAT: &str = "agent/node.fixture";

#[test]
fn restart_diagnostic_distinguishes_verified_start_then_exit_from_interrupted_attempt() {
    for (diagnostic_incarnation, receipt_operation, ended_incarnation, expect_detail, expect_started) in [
        ("new", None, None, true, false),
        ("old", None, None, false, false),
        ("old", Some("attempt-operation"), Some("new"), false, true),
        ("old", Some("attempt-operation"), None, false, true),
        ("old", Some("attempt-operation"), Some("old"), false, true),
        ("old", Some("unrelated-operation"), Some("new"), false, false),
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
            let receipt = append(
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
            assert!(receipt.id.starts_with("local-observation/"), "{}", receipt.id);
            assert!(
                store.claims_for(SEAT, Some("runtime.action.succeeded")).unwrap().is_empty(),
                "verified start completions live outside the replicated claim log"
            );
            assert_eq!(
                store.observations_for(SEAT, "runtime.action.succeeded").unwrap().len(),
                1
            );
            if let Some(incarnation) = ended_incarnation {
                append(
                    "harness.observed",
                    Some(SEAT),
                    BTreeMap::from([
                        ("state".into(), json!("ended")),
                        ("driver".into(), json!("omp")),
                        ("incarnation_id".into(), json!(incarnation)),
                        ("reason".into(), json!("driver-stopped")),
                        ("exit".into(), json!("exit 143")),
                    ]),
                    vec![],
                    None,
                );
            }
        }
        assert_eq!(
            reconciler.verified_restart_exit(SEAT, &attempt).unwrap().is_some(),
            expect_started,
            "only the exact attempt's verified local start receipt supplies a completed launch"
        );
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
        if expect_started {
            assert!(store.current_harness(SEAT).unwrap().is_none(),
                "the live harness view cannot supply terminal evidence");
        }
        let error = reconciler
            .reconcile_requested_restart(&desired, desired.member.as_ref().unwrap(), None, None)
            .unwrap_err()
            .to_string();
        if expect_started {
            let detail = if ended_incarnation == Some("new") {
                "the replacement started and then exited: driver-stopped"
            } else {
                "the replacement started successfully but is no longer observable"
            };
            assert!(error.contains(detail), "{error}");
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
