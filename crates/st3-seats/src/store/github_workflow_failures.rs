//! The part of `store::github_workflow_failures` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::resource::github_workflows::PerformanceFailure;
pub use st3_attention::store::github_workflow_failures::*;
pub use st3_attention::store::github_workflow_failures::{deliver_tx};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::parse_test_intent as parse_intent;
    use crate::resource::github_workflows::PERFORMANCE_FAILURES_FIELD;
    use st3_attention::store::github_workflow_failures_tests_shared_seats::*;
    use st3_attention::store::github_workflow_failures_tests_shared_seats::{SOURCE, failure, publish};

    fn observe(store: &Store, failures: Value, cursor: &str) -> ResourceObservationOutcome {
        let revision = store
            .selected_desired_revision("observer/repo")
            .unwrap()
            .unwrap();
        let subscriptions = store
            .desired_subjects()
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "subscription")
            .map(|item| {
                (
                    item.subject,
                    crate::graph::subscription_spec(&item.desired).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        store
            .record_resource_observation(
                "observer/repo",
                &revision,
                None,
                "resource/repo",
                Some(cursor),
                &json!({"repository_id": 7, PERFORMANCE_FAILURES_FIELD: failures}),
                100,
                &subscriptions,
                None,
            )
            .unwrap()
    }

    #[test]
    fn main_performance_failures_baseline_and_dedupe_across_restart_and_registration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let store = Store::open(&path, "node").unwrap();
        publish(&store, SOURCE, "register");
        assert!(
            observe(&store, json!([failure(1, 1, "a")]), "baseline")
                .message_subjects
                .is_empty()
        );
        let newly_failed = observe(
            &store,
            json!([failure(1, 1, "a"), failure(2, 1, "b")]),
            "new",
        );
        assert_eq!(newly_failed.message_subjects.len(), 1);
        let message = store
            .claims_for(&newly_failed.message_subjects[0], Some("message.sent"))
            .unwrap()
            .remove(0);
        assert!(
            message.body["fields"]["title"]
                .as_str()
                .unwrap()
                .starts_with("P0:")
        );
        assert_eq!(message.body["fields"]["to"], "agent/node.target");
        let content: Value =
            serde_json::from_str(message.body["fields"]["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["priority"], "P0");
        assert_eq!(content["repository"], "acme/garden");
        assert_eq!(content["run_id"], 2);
        assert_eq!(content["run_attempt"], 1);
        assert_eq!(content["head_sha"], "b".repeat(40));
        assert_eq!(content["workflow_path"], ".github/workflows/perf.yml");
        assert_eq!(
            content["url"],
            "https://github.com/acme/garden/actions/runs/2"
        );
        let duplicate = observe(
            &store,
            json!([failure(2, 1, "b")]),
            "same-failure-changed-list",
        );
        assert!(duplicate.message_subjects.is_empty());
        let mut renamed = failure(2, 1, "b");
        renamed["repository"] = json!("acme/renamed");
        renamed["url"] = json!("https://github.com/acme/renamed/actions/runs/2");
        assert!(
            observe(&store, json!([renamed]), "renamed-same-repository-id")
                .message_subjects
                .is_empty()
        );
        assert_eq!(store.messages(None, true).unwrap().len(), 1);
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        observe(&store, json!([]), "disappeared");
        let replacement = SOURCE.replace(
            "subscription \"performance\"",
            "subscription \"replacement\"",
        );
        publish(
            &store,
            &format!("{replacement}\nsubscription \"performance\" {{ stop }}"),
            "replace-subscription",
        );
        assert!(
            observe(
                &store,
                json!([failure(1, 1, "a"), failure(2, 1, "b")]),
                "reappeared"
            )
            .message_subjects
            .is_empty()
        );
        let changed = observe(
            &store,
            json!([failure(2, 2, "b"), failure(2, 2, "c"), failure(3, 1, "d")]),
            "rerun-and-new-head",
        );
        assert_eq!(changed.message_subjects.len(), 3);
        assert_eq!(store.messages(None, true).unwrap().len(), 4);
    }

    #[test]
    fn main_performance_failures_a_second_recipient_baselines_existing_history_and_an_empty_first_snapshot()
     {
        let store = Store::open_memory("node").unwrap();
        publish(&store, SOURCE, "first-recipient");
        observe(&store, json!([failure(1, 1, "a")]), "first-baseline");
        assert_eq!(
            observe(
                &store,
                json!([failure(1, 1, "a"), failure(2, 1, "b")]),
                "first-new"
            )
            .message_subjects
            .len(),
            1
        );
        publish(
            &store,
            r#"version 2
agent "second" { workspace "."; command "true" }
subscription "second-performance" { observer "observer/repo"; on "main_performance_failures"; to "agent/node.second"; delivery "message" }
"#,
            "second-recipient",
        );
        assert!(
            observe(
                &store,
                json!([failure(1, 1, "a"), failure(2, 1, "b")]),
                "second-baseline"
            )
            .message_subjects
            .is_empty()
        );
        assert_eq!(
            observe(
                &store,
                json!([failure(1, 1, "a"), failure(2, 1, "b"), failure(3, 1, "c")]),
                "both-new"
            )
            .message_subjects
            .len(),
            2
        );
        publish(
            &store,
            r#"version 2
agent "third" { workspace "."; command "true" }
subscription "third-performance" { observer "observer/repo"; on "main_performance_failures"; to "agent/node.third"; delivery "message" }
"#,
            "third-recipient",
        );
        assert!(
            observe(&store, json!([]), "empty-baseline")
                .message_subjects
                .is_empty()
        );
        assert_eq!(
            observe(&store, json!([failure(4, 1, "d")]), "all-new")
                .message_subjects
                .len(),
            3
        );
    }

    #[test]
    fn main_performance_failures_baseline_when_added_to_an_existing_observer() {
        let store = Store::open_memory("node").unwrap();
        publish(&store, SOURCE, "register");
        let revision = store
            .selected_desired_revision("observer/repo")
            .unwrap()
            .unwrap();
        store
            .record_resource_observation(
                "observer/repo",
                &revision,
                None,
                "resource/repo",
                None,
                &json!({"repository_id": 7}),
                100,
                &[],
                None,
            )
            .unwrap();
        assert!(
            observe(&store, json!([failure(1, 1, "a")]), "field-baseline")
                .message_subjects
                .is_empty()
        );
        let mut wrong = failure(99, 1, "a");
        wrong["event"] = json!("pull_request");
        let next = observe(
            &store,
            json!([failure(1, 1, "a"), failure(2, 1, "b"), wrong]),
            "next",
        );
        assert_eq!(next.message_subjects.len(), 1);
    }

    #[test]
    fn main_performance_failures_roll_back_the_receipt_and_message_together() {
        let store = Store::open_memory("node").unwrap();
        publish(&store, SOURCE, "register");
        observe(&store, json!([]), "baseline");
        let revision = store
            .selected_desired_revision("observer/repo")
            .unwrap()
            .unwrap();
        let subscriptions = store
            .desired_subjects()
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "subscription")
            .map(|item| {
                (
                    item.subject,
                    crate::graph::subscription_spec(&item.desired).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let current = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        let failed = store.record_resource_observation(
            "observer/repo",
            &revision,
            None,
            "resource/repo",
            Some("cancelled"),
            &json!({"repository_id": 7, PERFORMANCE_FAILURES_FIELD: [failure(2, 1, "b")]}),
            100,
            &subscriptions,
            Some(&current),
        );
        assert!(failed.is_err());
        assert!(store.messages(None, true).unwrap().is_empty());
        assert_eq!(
            observe(&store, json!([failure(2, 1, "b")]), "retry")
                .message_subjects
                .len(),
            1
        );
        assert_eq!(store.messages(None, true).unwrap().len(), 1);
    }
}
