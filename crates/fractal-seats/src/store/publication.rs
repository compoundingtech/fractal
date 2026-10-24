//! The part of `store::publication` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::MemberSpec;
use crate::publication::{Interruption, interruption};
pub use fractal_missions::store::publication::*;
pub use fractal_missions::store::publication::{Check, CommitChecks, Guard, check, launched_member};

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::publication_tests_shared::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::publication_tests_shared::{declaration, observe, publish};
    use super::*;
    use crate::graph::parse_intent;

    fn preview_change(original: &str, proposed: &str) -> MissionResponse {
        let store = Store::open_memory("node").unwrap();
        publish(&store, &parse_intent(original, "node").unwrap(), "original");
        store.mission(
            &parse_intent(proposed, "node").unwrap(),
            IntentInput { kdl: String::new(), source_name: None },
        ).unwrap()
    }

    #[test]
    fn apply_preview_metadata_only_with_harness_is_publish_only() {
        let original = "version 2\nagent \"example-preview/worker\" { workspace \"/tmp\"; harness \"claude\" {}; description \"before\"; }";
        let preview = preview_change(original, &original.replace("before", "after"));
        assert_eq!(preview.changes.len(), 1);
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        assert_eq!(preview.predicted_actions.len(), 1);
        assert_eq!(preview.predicted_actions[0].subject, "agent/example-preview/worker");
        assert_eq!(preview.predicted_actions[0].action, "publish-only");
    }

    #[test]
    fn apply_preview_metadata_only_without_harness_is_publish_only() {
        let original = "version 2\nagent \"example-preview/worker\" { workspace \"/tmp\"; description \"before\"; exec \"one\" { command \"true\"; }; exec \"two\" { command \"true\"; }; exec \"three\" { command \"true\"; }; }";
        let preview = preview_change(original, &original.replace("before", "after"));
        assert_eq!(preview.changes.len(), 1);
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        assert_eq!(preview.predicted_actions.len(), 1);
        assert_eq!(preview.predicted_actions[0].subject, "agent/example-preview/worker");
        assert_eq!(preview.predicted_actions[0].action, "publish-only");
    }

    #[test]
    fn apply_preview_launch_change_keeps_observe_or_start() {
        let original = "version 2\nagent \"example-preview/worker\" { workspace \"/tmp\"; command \"true\"; }";
        let preview = preview_change(original, &original.replace("true", "false"));
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.predicted_actions.len(), 1);
        assert_eq!(preview.predicted_actions[0].action, "observe-or-start");
    }

    #[test]
    fn apply_preview_grouping_only_is_publish_only() {
        for launch in ["harness \"claude\" {}", "exec \"one\" { command \"true\"; }"] {
            let original = format!("version 2\nagent \"example-preview/worker\" {{ workspace \"/tmp\"; {launch}; }}");
            let proposed = original.replace("workspace", "under \"agent/example-parent\"; workspace");
            let preview = preview_change(&original, &proposed);
            assert_eq!(preview.changes.len(), 1);
            assert_eq!(preview.predicted_actions.len(), 1);
            assert_eq!(preview.predicted_actions[0].action, "publish-only");
        }
    }

    #[test]
    fn apply_preview_operational_changes_are_not_metadata_only() {
        let original = "version 2\nagent \"example-preview/worker\" { workspace \"/tmp\"; command \"true\"; restart \"always\"; }";
        for proposed in [
            original.replace("/tmp", "/workspace"),
            original.replace("always", "never"),
            original.replace("command", "env { EXAMPLE \"changed\"; }; command"),
        ] {
            let preview = preview_change(original, &proposed);
            assert_eq!(preview.predicted_actions.len(), 1);
            assert_eq!(preview.predicted_actions[0].action, "observe-or-start");
        }
    }

    #[test]
    fn publication_check_uses_launch_not_selected_declaration() {
        let store = Store::open_memory("node").unwrap();
        let original = declaration("true");
        let applied = publish(&store, &original, "original");
        let token = &applied.subject_tokens["agent/example/worker"][0];
        observe(&store, "one", Some(token));
        let drift = declaration("false");
        publish(&store, &drift, "drift");
        let preview = store
            .mission(
                &drift,
                IntentInput {
                    kdl: String::new(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(preview.changes.is_empty());
        assert_eq!(preview.predicted_actions[0].action, "restart");
        let connection = store.readers.get();
        let check = check(
            &connection,
            &drift.subjects["agent/example/worker"],
            u64::MAX,
            "node",
        )
        .unwrap();
        assert_eq!(check.interruption, Interruption::Restart(vec!["launch"]));
        assert_eq!(check.incarnation.as_deref(), Some("one"));
        assert_eq!(
            check.launched.unwrap(),
            original.subjects["agent/example/worker"]
                .member
                .clone()
                .unwrap()
        );
    }

    #[test]
    fn publication_guard_reads_the_replacement_incarnation_before_writing() {
        let store = Store::open_memory("node").unwrap();
        let original = declaration("true");
        let applied = publish(&store, &original, "original");
        observe(
            &store,
            "one",
            Some(&applied.subject_tokens["agent/example/worker"][0]),
        );
        let proposed = declaration("true");
        let preview = store
            .mission(
                &proposed,
                IntentInput {
                    kdl: String::new(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(preview.predicted_actions.is_empty());
        // Matching old launch evidence is not evidence for a new runtime.
        observe(&store, "two", None);
        let before = store.index().unwrap();
        let error = store
            .apply_as_checked(
                &proposed,
                &preview.subject_tokens,
                "guard",
                None,
                &|_, check| {
                    assert_eq!(check.incarnation.as_deref(), Some("two"));
                    if check.interruption == Interruption::Unknown {
                        Err(St3Error::new("conflict", "unknown launch"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(store.index().unwrap(), before);
    }

    #[test]
    fn publication_guard_refuses_launch_drift_in_a_partial_bundle() {
        let store = Store::open_memory("node").unwrap();
        let original = declaration("true");
        let applied = publish(&store, &original, "original");
        let original_token = applied.subject_tokens["agent/example/worker"][0].clone();
        let changed = declaration("false");
        let changed_apply = publish(&store, &changed, "changed");
        observe(
            &store,
            "two",
            Some(&changed_apply.subject_tokens["agent/example/worker"][0]),
        );
        let mut bundle = changed.clone();
        let addition = parse_intent("version 2\nhost \"example-host\" {}", "node").unwrap();
        bundle.subjects.extend(addition.subjects);
        let preview = store
            .mission(
                &bundle,
                IntentInput {
                    kdl: String::new(),
                    source_name: None,
                },
            )
            .unwrap();
        observe(&store, "one", Some(&original_token));
        let before = store.index().unwrap();
        let refused = store
            .apply_as_checked(
                &bundle,
                &preview.subject_tokens,
                "guard-bundle",
                None,
                &|_, check| {
                    if check.interruption != Interruption::None {
                        Err(St3Error::new("conflict", "interrupting publication"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
        assert_eq!(refused.code, "conflict");
        assert_eq!(store.index().unwrap(), before);
        assert!(
            store
                .desired_subject_with_writer("host/example-host")
                .unwrap()
                .is_none()
        );
    }
}
