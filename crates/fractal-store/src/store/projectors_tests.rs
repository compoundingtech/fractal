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
            "credit-projection-retirement",
            "usage-source-fence",
            "run-eta",
            "gate-runners",
            "wake-lookup",
            "objective-times",
            "escalation-clocks",
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
        ("message.sent", &["custom", "resource-preparation", "agent-messages", "wake-demand", "ordered-membership-flush"]),
        ("harness.limits", &["custom", "resource-preparation", "limits", "ordered-membership-flush"]),
        ("usage.limits.observed", &["custom", "resource-preparation", "limits", "ordered-membership-flush"]),
        ("arrangement.edited", &["custom", "resource-preparation", "arrangements", "ordered-membership-flush"]),
        ("read-position.advanced", &["custom", "resource-preparation", "read-positions", "ordered-membership-flush"]),
        ("person.attention-seen", &["custom", "resource-preparation", "attention-controls", "ordered-membership-flush"]),
        ("person.attention-snoozed", &["custom", "resource-preparation", "attention-controls", "ordered-membership-flush"]),
        ("ordered-membership.edited", &["custom", "resource-preparation", "ordered-membership", "ordered-membership-flush"]),
        ("intent.desired", &["identities", "custom", "resource-preparation", "ordered-membership-flush"]),
        ("work.claimed", &["custom", "resource-preparation", "ordered-membership-flush"]),
        ("message.read", &["custom", "resource-preparation", "ordered-membership-flush"]),
    ] {
        let mut expected_with_times = vec!["escalation-clocks", "objective-times"];
        expected_with_times.extend_from_slice(expected);
        assert_eq!(taking(projectors::APPEND, kind), expected_with_times, "append order for {kind}");
    }
    assert_eq!(
        projectors::APPEND.iter().map(|projector| projector.name).collect::<Vec<_>>(),
        [
            "escalation-clocks",
            "objective-times",
            "identities",
            "custom",
            "resources",
            "resource-preparation",
            "glass-heads",
            "retirements",
            "directive-notes",
            "agent-messages",
            "wake-demand",
            "limits",
            "arrangements",
            "read-positions",
            "attention-controls",
            "ordered-membership",
            "ordered-membership-flush",
        ]
    );
}

#[test]
fn replicated_claims_project_arrangements_read_positions_and_ordered_memberships_only() {
    assert!(taking(projectors::REPLICATED, "usage.limits.observed").is_empty());
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
        ["objective-times", "escalation-clocks", "custom", "identities", "resources", "completion-health", "glass-heads", "directive-notes", "agent-messages", "wake-demand", "limits", "arrangements", "run-eta", "ordered-membership"]
    );
}

#[test]
fn rebuild_and_full_replay_keep_their_stage_order() {
    assert_eq!(
        names(projectors::REBUILD),
        ["planning", "identities", "resources", "completion-health", "slo-breaches", "glass-heads", "directive-notes", "arrangements", "read-positions", "attention-controls", "run-eta", "gate-runners", "wake-lookup", "escalation-clocks", "ordered-membership"]
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
            "full-replay/run-eta",
            "full-replay/wake-lookup",
            "full-replay/escalation-clocks",
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

#[test]
fn reopening_retires_disposable_credit_tables_and_trigger_without_changing_claims() {
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("retired-credit-projection.db");
    let store=super::Store::open(&path,"node").unwrap();
    store.connection.batched(|transaction| -> anyhow::Result<()> {
        // Historical projection fixture, not new authoritative state.
        transaction.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h1e32e6d704d1 owner=fractal_store::store::projectors_tests TODO=chunk-or-reference-variable-columns */
            "CREATE TABLE local_usage_credit_heads (
                provider TEXT NOT NULL,account TEXT NOT NULL,credit_id TEXT NOT NULL,actor TEXT NOT NULL,
                measured_seconds INTEGER NOT NULL,measured_nanos INTEGER NOT NULL,claim_id TEXT NOT NULL,credit TEXT NOT NULL,
                PRIMARY KEY(provider,account,credit_id,actor)) WITHOUT ROWID;
             CREATE TABLE local_usage_credit_pending (claim_id TEXT PRIMARY KEY) WITHOUT ROWID;
             CREATE TABLE local_usage_credit_progress (id INTEGER PRIMARY KEY CHECK(id=1),through_index INTEGER NOT NULL,target_index INTEGER NOT NULL);
             CREATE TRIGGER usage_credit_claim_insert AFTER INSERT ON claims WHEN NEW.kind='usage.limits.observed' BEGIN
                INSERT OR IGNORE INTO local_usage_credit_pending VALUES(NEW.id);
             END;"
        )?;
        Ok(())
    }).unwrap().unwrap();
    let actor="agent/sample/usage-producer";
    store.set_usage_limits_writers(vec![actor.into()]).unwrap();
    let claim=store.append_claim(&crate::model::ClaimInput{
        subject:actor.into(),kind:"usage.limits.observed".into(),actor:Some(actor.into()),
        fields:std::collections::BTreeMap::from([("readings".into(),serde_json::json!([])),("credit_readings".into(),serde_json::json!([]))]),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    let digest=store.replication_inventory().unwrap().digest;
    drop(store);
    let store=super::Store::open(&path,"node").unwrap();
    assert_eq!(store.replication_inventory().unwrap().digest,digest);
    assert_eq!(store.latest_claim(actor,Some("usage.limits.observed")).unwrap().unwrap().id,claim.id);
    let remaining:u64=store.readers.get().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('local_usage_credit_heads','local_usage_credit_pending','local_usage_credit_progress','usage_credit_claim_insert')",
        [],|row|row.get(0),
    ).unwrap();
    assert_eq!(remaining,0);
}

#[test]
fn native_usage_source_fence_is_independent_of_optional_views_and_survives_reopen() {
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("source-fence.db");
    let store=super::Store::open(&path,"node").unwrap();
    let identity = |store: &super::Store| store.readers.get().query_row(
        "SELECT identity.value,CAST(epoch.value AS INTEGER)
         FROM meta identity JOIN meta epoch ON epoch.key='local_usage_source_v1_epoch'
         WHERE identity.key='local_usage_source_v1_identity'",
        [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?)),
    ).unwrap();
    let initial=identity(&store);
    assert_eq!(initial.1,0);
    let actor="agent/sample/usage-producer";
    store.set_usage_limits_writers(vec![actor.into()]).unwrap();
    let claim=store.append_claim(&crate::model::ClaimInput{
        subject:actor.into(),kind:"usage.limits.observed".into(),actor:Some(actor.into()),
        fields:std::collections::BTreeMap::from([
            ("readings".into(),serde_json::json!([])),
            ("credit_readings".into(),serde_json::json!([])),
        ]),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap();
    assert_eq!(identity(&store),initial,"ordinary appends preserve incremental cursors");
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h3abbf8ee686b owner=fractal_store::store::projectors_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=accepted_at_unix_ms WHERE id=?1",[&claim.id]).unwrap();
    assert_eq!(identity(&store),(initial.0.clone(),1));
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-ha50a715460a4 owner=fractal_store::store::projectors_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1",[&claim.id]).unwrap();
    assert_eq!(identity(&store),(initial.0.clone(),2));
    drop(store);
    let reopened=super::Store::open(&path,"node").unwrap();
    assert_eq!(identity(&reopened),(initial.0.clone(),2));
    let replacement=super::Store::open_memory("node").unwrap();
    assert_ne!(identity(&replacement).0,initial.0);
}
