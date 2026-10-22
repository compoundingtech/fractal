#![allow(unused_imports)]
use super::*;


#[test]
fn unchanged_values_skip_history_but_a_named_attempt_keeps_its_idempotent_receipt() {
    let store = Store::open_memory("resource-fence-attempt").unwrap();
    let source = r#"version 2
resource "fixture" { kind "vcs.pull-request" }
observer "fixture" {
    resource "resource/fixture"
    provider "github.pull-request"
    locator "invented/fixture#1"
    field "state"
}
"#;
    let intent = crate::graph::parse_test_intent(source, "resource-fence-attempt").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(planned.blockers.is_empty());
    store
        .apply(&intent, &planned.subject_tokens, "fixture-observer")
        .unwrap();
    let observer = "observer/fixture";
    let revision = store.selected_desired_revision(observer).unwrap().unwrap();
    let observe = |attempt, next_check| {
        store
            .record_resource_observation(
                observer,
                &revision,
                attempt,
                "resource/fixture",
                Some("fixture-cursor"),
                &json!({"state": "open"}),
                next_check,
                &[],
                None,
            )
            .unwrap()
    };
    let count = || {
        store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claims", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    };
    assert!(observe(None, 100).baseline);
    let initial = count();
    let unchanged = observe(None, 200);
    assert!(unchanged.changed_fields.is_empty());
    assert!(unchanged.observation_claim.is_none());
    assert!(unchanged.message_subjects.is_empty());
    assert_eq!(count(), initial);
    let values = store
        .claims_for("resource/fixture", Some("resource.observed"))
        .unwrap()
        .len();
    let attempted = observe(Some("fixture-attempt"), 300);
    assert!(attempted.observation_claim.is_none());
    assert!(attempted.message_subjects.is_empty());
    assert_eq!(
        store
            .claims_for("resource/fixture", Some("resource.observed"))
            .unwrap()
            .len(),
        values
    );
    let receipts = store
        .claims_for(observer, Some("observer.observed"))
        .unwrap();
    assert_eq!(
        receipts
            .iter()
            .filter(|claim| claim.body["fields"]["attempt"] == "fixture-attempt")
            .count(),
        1
    );
    let after_attempt = count();
    assert!(after_attempt > initial);
    observe(Some("fixture-attempt"), 300);
    assert_eq!(count(), after_attempt);
}
