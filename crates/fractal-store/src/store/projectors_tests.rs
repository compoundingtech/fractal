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
            "read-positions",
            "attention-controls",
            "limits",
            "usage-credits",
            "run-eta",
            "gate-runners",
            "wake-lookup",
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
        ("usage.limits.observed", &["custom", "resource-preparation", "limits", "usage-credits", "ordered-membership-flush"]),
        ("arrangement.edited", &["custom", "resource-preparation", "arrangements", "ordered-membership-flush"]),
        ("read-position.advanced", &["custom", "resource-preparation", "read-positions", "ordered-membership-flush"]),
        ("person.attention-seen", &["custom", "resource-preparation", "attention-controls", "ordered-membership-flush"]),
        ("person.attention-snoozed", &["custom", "resource-preparation", "attention-controls", "ordered-membership-flush"]),
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
            "usage-credits",
            "arrangements",
            "read-positions",
            "attention-controls",
            "ordered-membership",
            "ordered-membership-flush",
        ]
    );
}

#[test]
fn replicated_claims_project_credits_arrangements_read_positions_and_ordered_memberships_only() {
    assert_eq!(taking(projectors::REPLICATED, "usage.limits.observed"), ["usage-credits"]);
    assert_eq!(taking(projectors::REPLICATED, "arrangement.edited"), ["arrangement-project"]);
    assert_eq!(taking(projectors::REPLICATED, "read-position.advanced"), ["read-position-project"]);
    for kind in ["person.attention-seen", "person.attention-snoozed"] {
        assert_eq!(taking(projectors::REPLICATED, kind), ["attention-control-project"]);
    }
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
        ["custom", "identities", "resources", "completion-health", "glass-heads", "directive-notes", "agent-messages", "limits", "usage-credits", "arrangements", "run-eta", "ordered-membership"]
    );
}

#[test]
fn rebuild_and_full_replay_keep_their_stage_order() {
    assert_eq!(
        names(projectors::REBUILD),
        ["planning", "identities", "resources", "completion-health", "slo-breaches", "glass-heads", "directive-notes", "arrangements", "read-positions", "attention-controls", "usage-credits", "run-eta", "gate-runners", "wake-lookup", "ordered-membership"]
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
            "full-replay/read-positions",
            "full-replay/attention-controls",
            "full-replay/usage-credits",
            "full-replay/run-eta",
            "full-replay/wake-lookup",
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

/// A replicated projection pass charges each flush hook, and the lease-renewal reapply after them,
/// to its own stage of the writer lend, so a long hold names the domain that caught up in it. The
/// staged lends are kept process-wide; CI runs each test in its own process (nextest).
#[test]
fn a_projection_pass_stages_each_flush_hook_by_name() {
    use crate::store::glasses_tests_shared_delivery::{input, sync};
    let source = crate::store::Store::open_memory("example-source").unwrap();
    let target = crate::store::Store::open_memory("example-target").unwrap();
    source.append_claim(&input(1, "example")).unwrap();
    sync(&source, &target);
    let staged = smallclaims::windows::holds::recent_staged();
    let stages = staged
        .iter()
        .rev()
        .map(|(stages, _)| stages.iter().map(|(stage, _)| *stage).collect::<Vec<_>>())
        .find(|stages| stages.contains(&"incremental"))
        .unwrap_or_else(|| panic!("no staged projection lend: {staged:?}"));
    for stage in names(projectors::FLUSH).into_iter().chain(["lease-renewals", "commit"]) {
        assert!(stages.contains(&stage), "{stage} in {stages:?}");
    }
}
