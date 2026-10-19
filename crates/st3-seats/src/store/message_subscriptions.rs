//! The part of `store::message_subscriptions` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_attention::store::message_subscriptions::*;
pub use st3_attention::store::message_subscriptions::{validate, validate_actor, validate_publisher, validate_replicated};

#[cfg(test)]
mod tests {
    use super::*;

    pub fn setup(store: &Store) {
        let intent = crate::graph::parse_internal_intent(PERFORMANCE_DECLARATIONS, "node").unwrap();
        store
            .apply_internal(&intent, "fixture-declarations")
            .unwrap();
    }

    #[test]
    fn main_performance_failures_registration_previews_applies_and_is_idempotent() {
        let store = Store::open_memory("node").unwrap();
        setup(&store);
        let intent = crate::graph::parse_intent(PERFORMANCE_REGISTRATION, "node").unwrap();
        assert_eq!(
            intent
                .direct_message_registrations
                .iter()
                .collect::<Vec<_>>(),
            ["subscription/fixture/main-performance-p0"]
        );
        let before = store.desired_subjects().unwrap();
        let observer = store
            .selected_desired_revision("observer/github/compoundingtech/smalltalk")
            .unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(
            store.desired_subjects().unwrap(),
            before,
            "preview is read-only"
        );
        let first = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "register",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap();
        let again = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "register",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap();
        assert_eq!(first.claim_ids, again.claim_ids);
        let next = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(
            !store
                .apply_as(
                    &intent,
                    &next.subject_tokens,
                    "reregister",
                    Some("agent/fleet/fixture-project/speed")
                )
                .unwrap()
                .changed
        );
        assert_eq!(
            store
                .selected_desired_revision("observer/github/compoundingtech/smalltalk")
                .unwrap(),
            observer
        );
        assert_eq!(store.desired_subjects().unwrap().len(), before.len() + 1);
        assert!(store.active_mission_runs().unwrap().is_empty());
        let desired = store
            .desired_subjects()
            .unwrap()
            .into_iter()
            .find(|item| item.subject == "subscription/fixture/main-performance-p0")
            .unwrap();
        let spec = crate::graph::subscription_spec(&desired.desired).unwrap();
        assert_eq!(spec.fields, ["main_performance_failures"]);
        assert_eq!(spec.observer, "observer/github/compoundingtech/smalltalk");
        assert_eq!(spec.to, "agent/fleet/fixture-project/speed");
    }

    #[test]
    fn main_performance_failures_registration_requires_existing_live_declarations_and_actor() {
        let intent = crate::graph::parse_intent(PERFORMANCE_REGISTRATION, "node").unwrap();
        let store = Store::open_memory("node").unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(
            plan.blockers
                .iter()
                .any(|reason| reason.contains("missing-registration-observer"))
        );
        let refused = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "no-observer",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap_err();
        assert_eq!(refused.code, "missing-registration-observer");
        assert!(store.desired_subjects().unwrap().is_empty());
        setup(&store);
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        let refused = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "bad-actor",
                Some("daemon/forged"),
            )
            .unwrap_err();
        assert_eq!(refused.code, "invalid-registration-actor");
        let stop = crate::graph::parse_internal_intent(
            "version 2\nobserver \"github/compoundingtech/smalltalk\" { stop }",
            "node",
        )
        .unwrap();
        store.apply_internal(&stop, "stop-observer").unwrap();
        let refused = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "stopped-observer",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap_err();
        assert_eq!(refused.code, "missing-registration-observer");
        assert!(
            !store
                .desired_subjects()
                .unwrap()
                .iter()
                .any(|item| item.kind == "subscription")
        );
    }

    #[test]
    fn main_performance_failures_registration_rejects_injection_overwrite_and_another_author() {
        let store = Store::open_memory("node").unwrap();
        setup(&store);
        let intent = crate::graph::parse_intent(PERFORMANCE_REGISTRATION, "node").unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        let error = store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "foreign-recipient",
                Some("agent/other"),
            )
            .unwrap_err();
        assert_eq!(error.code, "foreign-registration-recipient");
        assert!(
            !store
                .desired_subjects()
                .unwrap()
                .iter()
                .any(|d| d.kind == "subscription")
        );
        let existing =
            crate::graph::parse_internal_intent(PERFORMANCE_REGISTRATION, "node").unwrap();
        store
            .apply_as(
                &existing,
                &plan.subject_tokens,
                "prior-author",
                Some("person/operator"),
            )
            .unwrap();
        let same = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        let error = store
            .apply_as(
                &intent,
                &same.subject_tokens,
                "different-author",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap_err();
        assert_eq!(error.code, "message-registration-author");
        let different = PERFORMANCE_REGISTRATION.replace(
            "observer/github/compoundingtech/smalltalk",
            "observer/elsewhere",
        );
        let different = crate::graph::parse_intent(&different, "node").unwrap();
        assert!(
            store
                .mission(
                    &different,
                    IntentInput {
                        kdl: String::new(),
                        source_name: None
                    }
                )
                .unwrap()
                .blockers
                .iter()
                .any(|b| b.contains("message-registration-conflict"))
        );
        let stop = crate::graph::parse_internal_intent(
            "version 2\nstop \"agent/fleet/fixture-project/speed\"",
            "node",
        )
        .unwrap();
        store.apply_internal(&stop, "stop-target").unwrap();
        assert!(
            store
                .mission(
                    &intent,
                    IntentInput {
                        kdl: PERFORMANCE_REGISTRATION.into(),
                        source_name: None
                    }
                )
                .unwrap()
                .blockers
                .iter()
                .any(|b| b.contains("missing-registration-recipient"))
        );
    }

    #[test]
    fn main_performance_failures_replicated_registration_is_checked_without_a_client_marker() {
        let store = Store::open_memory("node").unwrap();
        setup(&store);
        let intent = crate::graph::parse_intent(PERFORMANCE_REGISTRATION, "node").unwrap();
        let desired = intent.subjects.values().next().unwrap().clone();
        store
            .connection
            .batched(|transaction| -> Result<(), St3Error> {
                let claim = append_claim_tx(
                    transaction,
                    "peer",
                    &desired.subject,
                    "intent.desired",
                    Some("agent/foreign"),
                    &serde_json::to_value(&desired).unwrap(),
                    &[],
                    None,
                )
                .map_err(claim_append_error)?;
                project_claim_isolated_tx(transaction, "projection:base", &claim, || {
                    select_replicated_desired(transaction, &claim, &desired)
                })?;
                let code: String = transaction
                    .query_row(
                        "SELECT error_code FROM projection_health WHERE aggregate=?1",
                        [format!("projection:base:{}", claim.id)],
                        |row| row.get(0),
                    )
                    .map_err(internal)?;
                assert_eq!(code, "foreign-registration-recipient");
                assert!(
                    current_desired_row_tx(transaction, &desired.subject)
                        .map_err(internal)?
                        .is_none()
                );
                Ok(())
            })
            .unwrap()
            .unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: PERFORMANCE_REGISTRATION.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply_as(
                &intent,
                &plan.subject_tokens,
                "legal-self-registration",
                Some("agent/fleet/fixture-project/speed"),
            )
            .unwrap();
        let claim = store
            .claims_for(&desired.subject, Some("intent.desired"))
            .unwrap()
            .into_iter()
            .find(|claim| claim.actor.as_deref() == Some("agent/fleet/fixture-project/speed"))
            .unwrap();
        store
            .connection
            .batched(|transaction| -> Result<(), St3Error> {
                validate_replicated(transaction, &claim, &desired)?;
                let source = PERFORMANCE_REGISTRATION.replace(
                    "on \"main_performance_failures\"",
                    "on \"main_performance_failures\"; on \"issues\"",
                );
                let broad = crate::graph::parse_internal_intent(&source, "node").unwrap();
                let broad = broad.subjects.values().next().unwrap();
                assert_eq!(
                    validate_replicated(transaction, &claim, broad)
                        .unwrap_err()
                        .code,
                    "invalid-message-registration"
                );
                Ok(())
            })
            .unwrap()
            .unwrap();
    }
}
