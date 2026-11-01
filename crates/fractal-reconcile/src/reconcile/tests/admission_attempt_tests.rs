use super::*;
use serde_json::json;

const SEAT: &str = "agent/node.fixture";

#[test]
fn driver_refusal_supplies_restart_error_without_physical_observation_and_is_incarnation_fenced() {
    for (diagnostic_incarnation, expect_detail) in [("new", true), ("old", false)] {
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
            BTreeMap::from([("action".into(), json!("start"))]),
            vec![request.id.clone()],
            Some(format!("agent-restart-attempt:{}", request.id)),
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
        let error = reconciler
            .reconcile_requested_restart(&desired, desired.member.as_ref().unwrap(), None, None)
            .unwrap_err()
            .to_string();
        if expect_detail {
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
