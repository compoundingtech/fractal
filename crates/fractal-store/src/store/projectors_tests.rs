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
            "glass-heads",
            "directive-notes",
            "agent-messages",
            "arrangements",
            "limits",
            "task-grants",
        ]
    );
}

#[test]
fn append_projects_each_claim_kind_in_order() {
    for (kind, expected) in [
        ("resource.observed", &["custom", "resources", "resource-preparation"][..]),
        ("glass.upserted", &["custom", "resource-preparation", "glass-heads"]),
        ("glass.deleted", &["custom", "resource-preparation", "glass-heads"]),
        ("agent.retired", &["identities", "custom", "resource-preparation", "retirements"]),
        ("agent.renamed", &["identities", "custom", "resource-preparation"]),
        ("mission.renamed", &["identities", "custom", "resource-preparation"]),
        ("person.directive-note-set", &["custom", "resource-preparation", "directive-notes"]),
        ("message.sent", &["custom", "resource-preparation", "agent-messages"]),
        ("harness.limits", &["custom", "resource-preparation", "limits"]),
        ("arrangement.edited", &["custom", "resource-preparation", "arrangements"]),
        ("task-grant.created", &["custom", "resource-preparation", "task-grants"]),
        ("task-grant.revoked", &["custom", "resource-preparation", "task-grants"]),
        ("intent.desired", &["identities", "custom", "resource-preparation"]),
        ("work.claimed", &["custom", "resource-preparation"]),
        ("message.read", &["custom", "resource-preparation"]),
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
            "task-grants",
        ]
    );
}

#[test]
fn replicated_claims_project_registered_families() {
    assert_eq!(taking(projectors::REPLICATED, "arrangement.edited"), ["arrangement-project"]);
    assert_eq!(taking(projectors::REPLICATED, "task-grant.created"), ["task-grants"]);
    assert_eq!(taking(projectors::REPLICATED, "task-grant.revoked"), ["task-grants"]);
    for kind in ["intent.desired", "glass.upserted", "message.sent", "harness.limits"] {
        assert!(taking(projectors::REPLICATED, kind).is_empty(), "{kind}");
    }
}

#[test]
fn flush_catches_up_each_domain_in_order() {
    assert_eq!(
        names(projectors::FLUSH),
        ["custom", "identities", "resources", "glass-heads", "directive-notes", "agent-messages", "limits", "arrangements"]
    );
}

#[test]
fn rebuild_and_full_replay_keep_their_stage_order() {
    assert_eq!(
        names(projectors::REBUILD),
        ["planning", "identities", "resources", "glass-heads", "directive-notes", "arrangements", "task-grants"]
    );
    assert_eq!(
        projectors::GRAPH_REPLAY.iter().map(|stage| stage.phase).collect::<Vec<_>>(),
        [
            "full-replay/planning",
            "full-replay/identities",
            "full-replay/resources",
            "full-replay/glass-heads",
            "full-replay/directive-notes",
            "full-replay/custom",
            "full-replay/arrangements",
            "full-replay/task-grants",
            "full-replay/flush",
        ]
    );
}
