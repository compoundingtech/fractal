#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_missions::{CARRY_SOURCE, carry_request, listed, misproject_a_step, publish_carry, request_fault};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{TAKEOVER_SOURCE, TREE_SHAPE_RUNS, TreeShapes, full_canonical_claim_id_oracle, graph_digest_of, insert_raw_claims, local_observation_for_test};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{local_timeline_observation_for_test, observe_runtime, projection_replayed, publish_mission, publish_takeover, query_plan, replicate_fully, roster_cache_store, simple};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared_seats::{exchange_of, harness_state, takeover_step, timeline_observation};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::tests_shared::{TEST_FLEET, exchange_from, receive_and_project};
use crate::AttentionSnapshotStore;
use super::*;
use crate::graph::parse_test_intent as parse_intent;
use proptest::prelude::*;


/// A call Claude's auto mode classifier denied waits on the seat's person: an alert until they
/// answer it, and the answer (allow once, allow its kind, or dismiss) is what the seat's
/// `PreToolUse` hook reads; the seat is told, so it runs the call again or leaves it.
#[test]
fn a_classifier_denial_is_an_alert_whose_answer_the_seat_hears() {
    let store = Store::open_memory("node").unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source = format!(
        "version 2\nagent \"worker\" {{ workspace {:?}; harness \"claude\" {{}} }}\n",
        workspace.path().display().to_string()
    );
    let intent = parse_intent(&source, "node").unwrap();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply_as(&intent, &preview.subject_tokens, "denial-seat", Some("person/avery"))
        .unwrap();
    let seat = "agent/node.worker";
    let append = |kind: &str, fields: Value, key: Option<&str>| {
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: key.map(str::to_owned),
            })
            .unwrap()
    };
    let denied = |call: &str| {
        append(
            "harness.diagnostic",
            json!({"severity":"warning", "status":"Bash", "code":"permission-denied",
                "reason":"Irreversible Local Destruction", "driver":"claude",
                "session_id":"session-1", "matched_line": json!({"call_id": call}).to_string()}),
            Some(&format!("claude-permission-denied:{seat}:{call}")),
        )
    };
    let alerts = || {
        store
            .attention_items(Some("person/avery"))
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "harness-prompt")
            .collect::<Vec<_>>()
    };
    append("runtime.observed", json!({"status":"running", "incarnation_id":"one"}), None);
    let first = denied("toolu_1");
    let shown = alerts();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].episode, first.id);
    assert_eq!(shown[0].answers, vec!["approve-once", "approve-kind", "dismiss"]);
    assert!(shown[0].title.ends_with("needs you to allow a Bash call"));
    assert!(shown[0].detail.contains("(Irreversible Local Destruction)"), "{}", shown[0].detail);
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "allow", "person/avery").unwrap_err().code,
        "invalid-answer"
    );
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "approve-once", "person/other").unwrap_err().code,
        "forbidden"
    );
    assert_eq!(store.prompt_answer(&first.id).unwrap().answer, None);
    store.answer_native_prompt(seat, &first.id, "approve-once", "person/avery").unwrap();
    assert_eq!(store.prompt_answer(&first.id).unwrap().answer.as_deref(), Some("approve-once"));
    assert!(alerts().is_empty(), "an answered denial is no longer an alert");
    // The seat hears it, from st, and runs the call again.
    let told = store
        .claims_for(&format!(
            "message/{}",
            &hex::encode(sha2::Sha256::digest(format!("st3-denial-answer:{}", first.id).as_bytes()))[..16]
        ), Some("message.sent"))
        .unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].body["fields"]["to"], seat);
    assert!(told[0].body["fields"]["content"].as_str().unwrap().contains("Run that same call again"));
    assert_eq!(
        store.answer_native_prompt(seat, &first.id, "dismiss", "person/avery").unwrap_err().code,
        "stale-fence",
        "an answer is final"
    );
    // A denial from before the seat restarted waits on nobody.
    let old = denied("toolu_2");
    assert_eq!(alerts().len(), 1);
    append("runtime.observed", json!({"status":"exited", "incarnation_id":"one"}), None);
    append("runtime.observed", json!({"status":"running", "incarnation_id":"two"}), None);
    assert!(alerts().is_empty());
    assert_eq!(
        store.answer_native_prompt(seat, &old.id, "approve-kind", "person/avery").unwrap_err().code,
        "stale-fence"
    );
}
