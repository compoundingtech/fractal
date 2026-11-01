//! The projector registry's order is part of the storage contract: a change here changes what the
//! fold derives from the same claims. Changing an expected list below needs the same review as a
//! schema change.
use super::projectors::{self, AppendProjector, Hook};

fn names(hooks: &[Hook]) -> Vec<&'static str> {
    hooks.iter().map(|hook| hook.name).collect()
}

fn taking(projectors: &[AppendProjector], kind: &str) -> Vec<&'static str> {
    projectors
        .iter()
        .filter(|projector| projector.takes.takes(kind))
        .map(|projector| projector.name)
        .collect()
}

#[test]
fn open_installs_each_domain_in_order() {
    assert_eq!(
        names(projectors::OPEN),
        [
            "publication-facts",
            "identities",
            "custom",
            "resources",
            "completion-health",
            "glass-heads",
            "directive-notes",
            "agent-messages",
            "arrangements",
            "limits",
        ]
    );
}

#[test]
fn append_projects_each_claim_kind_in_order() {
    for (kind, expected) in [
        ("resource.observed", &["custom", "resources", "resource-preparation", "ordered-membership-flush"][..]),
        ("glass.upserted", &["custom", "resource-preparation", "glass-heads", "ordered-membership-flush"]),
        ("glass.deleted", &["custom", "resource-preparation", "glass-heads", "ordered-membership-flush"]),
        ("agent.retired", &["identities", "custom", "resource-preparation", "retirements", "ordered-membership-flush"]),
        ("agent.renamed", &["identities", "custom", "resource-preparation", "ordered-membership-flush"]),
        ("mission.renamed", &["identities", "custom", "resource-preparation", "ordered-membership-flush"]),
        ("person.directive-note-set", &["custom", "resource-preparation", "directive-notes", "ordered-membership-flush"]),
        ("message.sent", &["custom", "resource-preparation", "agent-messages", "ordered-membership-flush"]),
        ("harness.limits", &["custom", "resource-preparation", "limits", "ordered-membership-flush"]),
        ("arrangement.edited", &["custom", "resource-preparation", "arrangements", "ordered-membership-flush"]),
        ("ordered-membership.edited", &["custom", "resource-preparation", "ordered-membership", "ordered-membership-flush"]),
        ("intent.desired", &["identities", "custom", "resource-preparation", "ordered-membership-flush"]),
        ("work.claimed", &["custom", "resource-preparation", "ordered-membership-flush"]),
        ("message.read", &["custom", "resource-preparation", "ordered-membership-flush"]),
    ] {
        assert_eq!(taking(projectors::APPEND, kind), expected, "append order for {kind}");
    }
    assert_eq!(
        projectors::APPEND.iter().map(|projector| projector.name).collect::<Vec<_>>(),
        [
            "identities",
            "custom",
            "resources",
            "resource-preparation",
            "glass-heads",
            "retirements",
            "directive-notes",
            "agent-messages",
            "limits",
            "arrangements",
            "ordered-membership",
            "ordered-membership-flush",
        ]
    );
}

#[test]
fn replicated_claims_project_arrangements_and_ordered_memberships_only() {
    assert_eq!(taking(projectors::REPLICATED, "arrangement.edited"), ["arrangement-project"]);
    assert_eq!(
        taking(projectors::REPLICATED, "ordered-membership.edited"),
        ["ordered-membership-project"]
    );
    for kind in ["intent.desired", "glass.upserted", "message.sent", "harness.limits"] {
        assert!(taking(projectors::REPLICATED, kind).is_empty(), "{kind}");
    }
}

#[test]
fn flush_catches_up_each_domain_in_order() {
    assert_eq!(
        names(projectors::FLUSH),
        ["custom", "identities", "resources", "completion-health", "glass-heads", "directive-notes", "agent-messages", "limits", "arrangements", "ordered-membership"]
    );
}

#[test]
fn rebuild_and_full_replay_keep_their_stage_order() {
    assert_eq!(
        names(projectors::REBUILD),
        ["planning", "identities", "resources", "completion-health", "slo-breaches", "glass-heads", "directive-notes", "arrangements", "ordered-membership"]
    );
    assert_eq!(
        projectors::GRAPH_REPLAY.iter().map(|stage| stage.phase).collect::<Vec<_>>(),
        [
            "full-replay/planning",
            "full-replay/identities",
            "full-replay/resources",
            "full-replay/completion-health",
            "full-replay/glass-heads",
            "full-replay/directive-notes",
            "full-replay/custom",
            "full-replay/arrangements",
            "full-replay/ordered-membership",
            "full-replay/flush",
        ]
    );
}

#[test]
fn ordered_membership_is_registered_last_in_every_phase_that_projects_it() {
    // Its pair records depend on the arrangement and declared-resource tables the earlier
    // entries keep, so it follows them in append, flush and rebuild order.
    assert_eq!(projectors::APPEND.last().map(|p| p.name), Some("ordered-membership-flush"));
    assert_eq!(
        projectors::APPEND.iter().rev().nth(1).map(|p| p.name),
        Some("ordered-membership")
    );
    assert_eq!(projectors::REPLICATED.last().map(|p| p.name), Some("ordered-membership-project"));
    assert_eq!(projectors::FLUSH.last().map(|hook| hook.name), Some("ordered-membership"));
    assert_eq!(projectors::REBUILD.last().map(|hook| hook.name), Some("ordered-membership"));
    // The full replay's final stage resets publication facts after every domain rebuild.
    let phases = projectors::GRAPH_REPLAY.iter().map(|stage| stage.phase).collect::<Vec<_>>();
    assert_eq!(
        &phases[phases.len() - 2..],
        ["full-replay/ordered-membership", "full-replay/flush"]
    );
}
