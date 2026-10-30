#![allow(unused_imports)]
use crate::store::*;
use crate::model::{MessageAttachment, PersonAskRequest, PersonStepResponse};
use crate::person_request::{StructuredRequest, answer_summary};
use crate::store::person_work::*;

pub fn gateway_decision() -> Value {
        json!({
            "version": 1,
            "type": "decision",
            "question": "Change the browser sign-in, then review the gateway again?",
            "why_person": "Only the owner decides how browsers authenticate.",
            "recommendation": {"answer": "revise-auth", "reason": "Native clients already pair this way."},
            "subjects": [{"kind": "pull_request", "label": "#41", "url": "https://example.com/pull/41", "revision": "abc123"}],
            "answers": [
                {"id": "land", "label": "Land the gateway", "outcome": "accept", "consequence": "The gateway merges as it is."},
                {"id": "keep-open", "label": "Keep it open", "outcome": "decline", "consequence": "Nothing merges."},
                {"id": "revise-auth", "label": "Revise auth and review", "outcome": "request_changes", "consequence": "The author changes sign-in and asks again."}
            ]
        })
    }

pub const TEST_FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

pub fn receive_exchange(target: &Store, exchange: &ReplicationExchange) {
        let before = FULL_REPLAYS.with(|count| count.get());
        target
            .receive_replication_exchange(&exchange.peer, TEST_FLEET, exchange)
            .unwrap();
        target.validate_replication_backlog().unwrap();
        assert!(target.project_replication_backlog().unwrap());
        assert_eq!(
            FULL_REPLAYS.with(|count| count.get()),
            before,
            "person work replayed the whole graph"
        );
        let incremental = projection_digest::tables(&target.readers.get()).unwrap();
        target.replay_replication_graph().unwrap();
        assert_eq!(
            incremental,
            projection_digest::tables(&target.readers.get()).unwrap(),
            "person work differs from full replay"
        );
    }
