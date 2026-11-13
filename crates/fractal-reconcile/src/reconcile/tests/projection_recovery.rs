use super::*;
use serde_json::json;

#[test]
fn projection_recovery_raises_one_operator_failure_until_explicit_replay() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        "version 2\nagent \"worker\" { workspace \"/tmp\"; command \"true\" }",
        "projection-recovery-fixture",
    );
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    assert!(store.index().unwrap() > 0);
    assert!(store.operational_failures().unwrap().is_empty());

    // Exercise the actual first incremental refusal, not a fabricated parked row.
    // Test-only primary-key graph row removal; no variable producer values are bound.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-h8bfe2b208a0a owner=fractal_reconcile::reconcile::tests::projection_recovery TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM projection_health WHERE aggregate='graph'", [],
    ).unwrap();
    assert!(!store.project_replication_backlog().unwrap());
    assert!(store.replication_projection_recovery_required().unwrap());
    let recovery = store.projection_recovery().unwrap().unwrap();
    assert_eq!(recovery.frontier, None);
    assert_eq!(recovery.reason_code, "missing-health");
    let current_target: u64 = store.readers.get().query_row(
        "SELECT value FROM meta WHERE key='replication_admitted_index'", [],
        |row| row.get::<_, String>(0),
    ).unwrap().parse().unwrap();
    assert_eq!(recovery.current_target, current_target);

    reconciler.reconcile_projection_recovery().unwrap();
    let pending = store.pending_operational_failures(RECONCILER_ACTOR, "node").unwrap();
    assert_eq!(pending.len(), 1);
    let failure = &pending[0];
    assert_eq!(failure.title, "Replication projection requires explicit recovery");
    assert_eq!(failure.targets, ["daemon/node"]);
    assert_eq!(failure.reviewer, "person/operator");
    assert_eq!(failure.status, "pending");
    assert_eq!(failure.actor, RECONCILER_ACTOR);
    for expected in [
        "last-good frontier None".to_owned(),
        "reason missing-health".to_owned(),
        format!("current admitted target {};", recovery.current_target),
        "explicit local operator recovery is required".to_owned(),
    ] {
        assert!(failure.reason.contains(&expected), "{}", failure.reason);
    }
    assert_eq!(store.operational_failures().unwrap().len(), 1);
    assert_eq!(store.claims_for("daemon/node", Some("operational.failure")).unwrap().len(), 1);

    let raised_index = store.index().unwrap();
    for _ in 0..3 {
        // Neither live projection nor the operator-fault producer may repair by full replay.
        assert!(!store.project_replication_backlog().unwrap());
        reconciler.reconcile_projection_recovery().unwrap();
        assert!(store.replication_projection_recovery_required().unwrap());
        let still_parked = store.projection_recovery().unwrap().unwrap();
        assert_eq!(still_parked.frontier, None);
        assert_eq!(still_parked.reason_code, "missing-health");
        assert_eq!(still_parked.updated_at_unix_ms, recovery.updated_at_unix_ms);
        let pending = store.pending_operational_failures(RECONCILER_ACTOR, "node").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].subject, failure.subject);
        assert_eq!(pending[0].request, failure.request);
        assert_eq!(store.operational_failures().unwrap().len(), 1);
        assert_eq!(store.claims_for("daemon/node", Some("operational.failure")).unwrap().len(), 1);
        assert_eq!(store.index().unwrap(), raised_index);
    }

    // Only an explicit operator replay clears the durable park; the next helper call
    // resolves the original operational failure rather than creating another episode.
    store.replay_replication_graph().unwrap();
    assert!(!store.replication_projection_recovery_required().unwrap());
    assert!(store.projection_recovery().unwrap().is_none());
    reconciler.reconcile_projection_recovery().unwrap();
    assert!(store.pending_operational_failures(RECONCILER_ACTOR, "node").unwrap().is_empty());
    let resolved = store.operational_failure(&failure.subject).unwrap().unwrap();
    assert_eq!(resolved.subject, failure.subject);
    assert_eq!(resolved.request, failure.request);
    assert_eq!(resolved.status, "resolved");
    assert_eq!(store.operational_failures().unwrap().len(), 1);
    assert_eq!(store.claims_for("daemon/node", Some("operational.failure")).unwrap().len(), 1);
    let recovered = store.claims_for("daemon/node", Some("operational.recovered")).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].body["fields"]["failure"], failure.request);

    let recovered_index = store.index().unwrap();
    reconciler.reconcile_projection_recovery().unwrap();
    assert_eq!(store.index().unwrap(), recovered_index);
}

#[test]
fn partial_late_owned_members_keep_running_seats_held_without_runtime_replacement() {
    let store = Arc::new(Store::open_memory("node").unwrap());
    let orchard = Store::open_memory("aaa-reference-orchard").unwrap();
    let meadow = Store::open_memory("aaa-reference-meadow").unwrap();
    orchard.set_write_clock_at(1_000).unwrap();
    meadow.set_write_clock_at(1_000).unwrap();
    apply_source(&orchard,
        "version 2\nagent \"orchard\" { workspace \"/tmp\"; command \"replacement-orchard\" }",
        "future-orchard");
    apply_source(&meadow,
        "version 2\nagent \"meadow\" { workspace \"/tmp\"; command \"replacement-meadow\" }",
        "future-meadow");
    let future_orchard = orchard.claims_for("agent/node.orchard", Some("intent.desired"))
        .unwrap().pop().unwrap();
    let future_meadow = meadow.claims_for("agent/node.meadow", Some("intent.desired"))
        .unwrap().pop().unwrap();
    let input = crate::graph::parse_owned_set_intent(
        "version 2\nagent \"orchard\" { workspace \"/tmp\"; command \"true\" }\nagent \"meadow\" { workspace \"/tmp\"; command \"true\" }",
        "node",
    ).unwrap();
    let mut options = crate::store::owned_sets::Options {
        set: "garden".into(), expected_set: "absent".into(),
        source: crate::store::owned_sets::Source {
            repository: "test/garden".into(), r#ref: "refs/heads/main".into(),
            sha: format!("{:040x}", 10), sequence: 10,
        },
        rollout: None, adopt: Default::default(), allow_empty: false,
        confirm_retire: None, policy: None, expected_subjects: Default::default(),
    };
    let preview = store.owned_set_preview(&input, &options).unwrap();
    options.expected_subjects = preview.expected_subjects;
    options.confirm_retire = Some(preview.digest);
    store.apply_owned_set(&input, &options, "initial", "person/operator").unwrap();
    assert!(store.project_replication_backlog().unwrap());
    let physical = || {
        let connection = store.readers.get();
        ["agent/node.orchard", "agent/node.meadow"].map(|subject|
            connection.query_row(
                "SELECT claim_id,revision,body,member FROM desired WHERE subject=?1",
                [subject], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
            ).unwrap())
    };
    let last_good = physical();
    let runtime = Arc::new(FakeRuntime::default());
    for desired in store.desired_subjects().unwrap() {
        let incarnation = format!("last-good-{}", desired.subject);
        let token = store.selected_desired_token(&desired.subject).unwrap().unwrap();
        store.append_claim(&ClaimInput {
            subject: desired.subject.clone(), kind: "runtime.action.succeeded".into(),
            actor: None, fields: BTreeMap::from([
                ("action".into(), json!("start")), ("incarnation_id".into(), json!(incarnation)),
                ("desired_token".into(), json!(token)),
            ]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        runtime.ptys.lock().unwrap().push(RuntimeObservation {
            runtime_id: desired.member.as_ref().unwrap().runtime_id.clone(),
            terminal: true, status: "running".into(), exit_code: None,
            incarnation_id: Some(incarnation),
        });
    }
    let reconciler = Reconciler::new(store.clone(), runtime.clone(), "node".into(), Arc::new(Notify::new()));
    reconciler.reconcile_once().unwrap();
    assert!(runtime.starts.lock().unwrap().is_empty());
    assert!(runtime.stops.lock().unwrap().is_empty());
    let old = store.owned_sets().unwrap().remove(0);
    let mut receipt = old.receipt.clone();
    receipt.previous = Some(format!("{}@{}", old.id, old.revision));
    receipt.source.sequence = 30;
    receipt.source.sha = format!("{:040x}", 30);
    for (subject, claim) in [
        ("agent/node.orchard", &future_orchard), ("agent/node.meadow", &future_meadow),
    ] {
        let desired: DesiredSubject = serde_json::from_value(claim.body.clone()).unwrap();
        let member = receipt.members.get_mut(subject).unwrap();
        member.claim = claim.id.clone();
        member.revision = crate::store::desired_revision(&desired);
    }
    store.set_write_clock_at(5_000).unwrap();
    let held_receipt = store.append_claim(&ClaimInput {
        subject: old.id, kind: "owned-set.revised".into(), actor: None,
        fields: serde_json::from_value(json!({
            "revision": smallclaims::hash::canonical_hash(&receipt).unwrap(), "body": receipt,
        })).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    assert_eq!(physical(), last_good);
    assert!(!store.owned_sets().unwrap()[0].blockers.is_empty());
    let receive = |source: &Store| {
        const FLEET: &str = "5b0c1d8e-6a44-4f0e-9d51-2f7f3c9a0b12";
        source.bind_fleet(FLEET).unwrap();
        store.bind_fleet(FLEET).unwrap();
        let exchange = source.export_replication_exchange(FLEET, &store.replication_inventory().unwrap()).unwrap();
        let receipt = store.receive_replication_exchange(&source.origin, FLEET, &exchange).unwrap();
        assert!(receipt.received > 0);
        store.validate_replication_backlog().unwrap();
        let replays = smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get);
        assert!(store.project_replication_backlog().unwrap());
        assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), replays);
    };
    reconciler.reconcile_once().unwrap();
    receive(&orchard);
    assert!(smallclaims::store::canonical::claim_key(&store.readers.get(), &future_orchard.id).unwrap()
        < smallclaims::store::canonical::claim_key(&store.readers.get(), &held_receipt.id).unwrap());
    assert!(store.claim_by_id(&future_meadow.id).unwrap().is_none());
    assert!(!store.owned_sets().unwrap()[0].blockers.is_empty());
    assert_eq!(physical(), last_good, "the partial arrival must retain physical last-good rows");
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
        assert_eq!(physical(), last_good);
        assert!(!reconciler.owned_desired_check(
            &store.desired_subject_with_writer("agent/node.orchard").unwrap().unwrap().0,
        ).unwrap());
        assert!(runtime.starts.lock().unwrap().is_empty());
        assert!(runtime.stops.lock().unwrap().is_empty());
        assert!(runtime.kills.lock().unwrap().is_empty());
        assert!(runtime.removes.lock().unwrap().is_empty());
        assert!(runtime.ptys.lock().unwrap().iter().all(|observation| observation.status == "running"));
    }
    receive(&meadow);
    assert!(store.owned_sets().unwrap()[0].blockers.is_empty());
    assert_eq!(physical()[0].0, future_orchard.id);
    assert_eq!(physical()[1].0, future_meadow.id);
    assert!(reconciler.owned_desired_check(
        &store.desired_subject_with_writer("agent/node.orchard").unwrap().unwrap().0,
    ).unwrap());
}
