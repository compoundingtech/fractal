//! The part of `store::card_fold::health_capture` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::{BTreeMap, BTreeSet};
use anyhow::Result;
use rusqlite::{Connection, Statement, StatementStatus, params};
use serde_json::Value;
pub use st3_store::store::card_fold::health_capture::*;
pub use st3_store::store::card_fold::health_capture::{CandidateOwner, CardCapture, PendingCandidates, Request, SourceRow, capture};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use st3_store::store::card_fold::health_capture_tests_shared_attention_tests::{fixture, insert};

    #[test]
    fn private_card_extension_retains_raw_candidates_without_fallback_or_admission() {
        use crate::model::CurrentHarnessView;
        let connection = fixture();
        insert(
            &connection,
            "desired-a",
            "agent/a",
            "intent.desired",
            2,
            102,
            json!({}),
        );
        insert(
            &connection,
            "state-a",
            "agent/a",
            "harness.observed",
            3,
            103,
            json!({"incarnation_id":"incarnation-a", "evidence_incarnation":"provider-a",
                "ownership_sequence":1, "driver":"omp", "transport":"omp-channel",
                "observed_since_ms":100}),
        );
        connection
            .execute("UPDATE claims SET actor='agent/a' WHERE id='state-a'", [])
            .unwrap();
        insert(
            &connection,
            "output-a",
            "agent/a",
            "harness.output.observed",
            4,
            104,
            json!({}),
        );
        let harness = CurrentHarnessView {
            state: "idle".into(),
            driver: Some("omp".into()),
            incarnation_id: "incarnation-a".into(),
            transport: Some("omp-channel".into()),
            reason: None,
            blocked_on: None,
            ask: None,
            input_buffer: None,
            exit: None,
            claim: "state-a".into(),
            observed_at_unix_ms: 103,
            since_unix_ms: 100,
        };
        let mut reads = super::super::AgentCardReads {
            index: 10,
            harness: std::collections::HashMap::from([("agent/a".into(), Some(harness))]),
            activity: Default::default(),
            suspensions: Default::default(),
            rollouts: Default::default(),
            health: BTreeMap::new(),
        };
        connection.execute_batch("BEGIN").unwrap();
        reads
            .prepare_health_chunk(
                &connection,
                "namespace-a",
                1_000,
                &[("agent/a", Some("runtime-a"), Some("desired-a"))],
            )
            .unwrap();
        let captured = reads.take_health("agent/a").unwrap();
        assert_eq!(captured.namespace, "namespace-a");
        assert_eq!(captured.subject, "agent/a");
        assert_eq!(captured.index, 10);
        assert_eq!(captured.desired.unwrap().id, "desired-a");
        assert_eq!(captured.harness_candidate.unwrap().id, "state-a");
        assert_eq!(captured.output_candidate.unwrap().id, "output-a");
        let owner = captured.owner_candidate.unwrap();
        assert_eq!(
            (owner.source.as_str(), owner.origin.as_str()),
            ("state-a", "host/a")
        );
        assert_eq!(
            (
                owner.runtime_incarnation.as_str(),
                owner.provider_incarnation.as_str()
            ),
            ("incarnation-a", "provider-a")
        );
        assert_eq!(
            (
                owner.ownership_sequence,
                owner.driver.as_str(),
                owner.component.as_str(),
                owner.reported_since_ms
            ),
            (1, "omp", "omp-channel", Some(100))
        );
        // These are raw candidate values, not an admitted current provider or
        // supported output capability. There is no positive-health conversion.
        assert!(reads.take_health("agent/a").is_none());
    }
}
