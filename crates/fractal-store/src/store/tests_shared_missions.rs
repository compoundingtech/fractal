#![allow(unused_imports)]
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
use crate::store::*;
use crate::graph::parse_test_intent as parse_intent;

pub fn listed(store: &Store, sql: &str, params: impl rusqlite::Params) -> Vec<String> {
    store
        .readers
        .get()
        .prepare(/* sqlite-large-value: allow-for-now lv-heda27c60a61a owner=st3_store::store::tests_shared_missions TODO=trace-statement-and-value-representation */ sql)
        .unwrap()
        .query_map(params, |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Put one step of every run on another attempt, as the carried-step lookup by batch did.
pub fn misproject_a_step(store: &Store) {
    let connection = store.connection.lock().unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hc0f084255760 owner=st3_store::store::tests_shared_missions TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET attempt=attempt+1
                 WHERE subject=(SELECT subject FROM step_runs ORDER BY subject LIMIT 1)",
            [],
        )
        .unwrap();
}

pub const CARRY_SOURCE: &str = r#"
version 2

  mission "carry" state="ready" {
    goal "Keep claimed work across a revision."
    step "work" { assigned-to "agent/worker"; goal "Do the claimed work." }
    step "other" { assigned-to "agent/worker"; goal "Use the first goal." }
  }

"#;

pub fn publish_carry(store: &Store, source: &str, key: &str) -> MissionSpec {
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    store.apply(&intent, &planned.subject_tokens, key).unwrap();
    intent.missions["carry"].clone()
}

pub fn carry_request(incarnation: &str, key: &str) -> WorkRequest {
    WorkRequest {
        wait_ms: None,
        actor: Some("agent/node.worker".into()),
        incarnation: Some(incarnation.into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        idempotency_key: key.into(),
    }
}

pub fn request_fault(store: &Store, subject: &str, targets: &[&str]) {
    store
        .request_attention(
            subject,
            &AttentionRequest {
                reviewer: "person/alex".into(),
                title: format!("Fault {subject}"),
                reason: "a person must decide".into(),
                severity: "warning".into(),
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                actor: "daemon/runtime".into(),
                idempotency_key: format!("{subject}:requested"),
            },
        )
        .unwrap();
}
