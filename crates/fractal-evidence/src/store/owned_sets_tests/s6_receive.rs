//! Generated held-view parity and actual live receive replay-fence regressions.
//! No new production seam: FULL_REPLAYS is thread-local and every receive/project
//! below runs synchronously on this test's thread (never an HTTP worker).
use super::*;

/// Compare effective authority, including holds; physical desired rows may remain
/// present while a set is incomplete or conflicted and are deliberately not the oracle.
fn effective_view(store: &Store) -> Value {
    let sets = store.owned_sets().unwrap();
    let selected = sets.iter().map(|set| {
        let members = owned_sets::effective_members(&store.readers.get(), set, None).unwrap();
        json!({"id": set.id, "claim": set.claim, "revision": set.revision,
            "blockers": set.blockers, "members": members})
    }).collect::<Vec<_>>();
    let subjects = ["agent/garden/orchard", "agent/garden/meadow"].into_iter().map(|subject| {
        let guard = store.owned_member_guard(subject).err().map(|error| error.code);
        let desired = store.desired_subjects_named(&[subject.into()]).unwrap();
        let admitted = store.owned_desired_subjects(&desired).unwrap();
        let token = if guard.is_none() { store.selected_desired_token(subject).unwrap() } else { None };
        json!({"subject": subject, "token": token, "held": guard, "admitted": admitted})
    }).collect::<Vec<_>>();
    json!({"sets": selected, "subjects": subjects})
}

fn canonical_inputs(store: &Store) -> Vec<(smallclaims::store::canonical::ClaimKey, Value)> {
    let batch = store.export_replication_for_heads(&std::collections::BTreeMap::new()).unwrap();
    assert!(batch.batches.len() < 512, "oracle fixture must not reach the export page cap");
    let connection = store.readers.get();
    let mut inputs = batch.batches.iter().flat_map(|batch| &batch.claims).map(|claim| {
        (smallclaims::store::canonical::claim_key(&connection, &claim.id).unwrap(),
            json!({"id": claim.id, "subject": claim.subject, "kind": claim.kind,
                "origin": claim.origin, "actor": claim.actor, "body": claim.body,
                "predecessors": claim.predecessors}))
    }).collect::<Vec<_>>();
    inputs.sort_by(|left, right| left.0.cmp(&right.0));
    inputs
}

fn assert_replay_parity(store: &Store, seed: u64, step: usize) {
    let incremental = effective_view(store);
    // Replay a separate store so the oracle never repairs the incremental target
    // between arrivals. Compare every prefix, including intermediate holds.
    let replay = Store::open_memory("replay-oracle").unwrap();
    let batch = store.export_replication_for_heads(&std::collections::BTreeMap::new()).unwrap();
    assert!(batch.batches.len() < 512, "seed={seed}, step={step}: export page cap");
    replay.import_replication(&store.origin, &batch).unwrap();
    assert_eq!(replay.replica_heads().unwrap(), store.replica_heads().unwrap(),
        "seed={seed}, step={step}: oracle must admit every source batch");
    assert_eq!(canonical_inputs(&replay), canonical_inputs(store),
        "seed={seed}, step={step}: replay requires identical canonical inputs and keys");
    replay.connection.batched(replay_graph_from_nothing_tx).unwrap().unwrap();
    assert_eq!(effective_view(&replay), incremental, "seed={seed}, step={step}");
}

fn next(seed: &mut u64) -> u64 {
    // Fixed-seed generated cases are reproducible without a new RNG dependency.
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn t1_generated_owned_set_arrivals_match_full_replay_effective_held_view() {
    for seed in 1..=24_u64 {
        let mut random = seed;
        let base = Store::open_memory("base").unwrap();
        apply(&base, &bundle("initial", false), 1);
        let target = Store::open_memory("target").unwrap();
        receive_and_project(&target, "base", &exchange_from(&base, &target.replication_inventory().unwrap()));
        let branches = ["amber", "cobalt", "ivory"].map(|origin| {
            let branch = Store::open_memory(origin).unwrap();
            share(&base, &branch);
            branch
        });
        let mut arrivals = Vec::new();
        for (index, branch) in branches.iter().enumerate() {
            for revision in 0..3 {
                // Equal sequences across branches exercise conflict holds; unequal
                // sequences exercise highest-source selection and losing-only members.
                let sequence = 10 + revision * 10 + next(&mut random) % 2;
                let input = bundle(&format!("branch-{index}-{revision}-{seed}"), next(&mut random).is_multiple_of(2));
                let mut opts = options(branch, sequence);
                let preview = branch.owned_set_preview(&input, &opts).unwrap();
                opts.expected_subjects = preview.expected_subjects;
                opts.confirm_retire = Some(preview.digest);
                branch.apply_owned_set(&input, &opts, &format!("generated-{}-{revision}", branch.origin), "person/operator").unwrap();
                arrivals.push((branch.origin.clone(), exchange_from(branch, &Default::default())));
            }
        }
        for index in (1..arrivals.len()).rev() {
            let other = (next(&mut random) as usize) % (index + 1);
            arrivals.swap(index, other);
        }
        for (step, (origin, exchange)) in arrivals.iter().enumerate() {
            receive_and_project(&target, origin, exchange);
            assert_replay_parity(&target, seed, step);
        }
        // The separate late-reference case guarantees a lower canonical key and a
        // missing-reference hold, rather than relying on a random shuffle to do so.
        let (reference, receipt) = incomplete_revision(&target);
        assert!(!target.owned_sets().unwrap()[0].blockers.is_empty());
        assert_replay_parity(&target, seed, arrivals.len());
        receive_and_project(&target, &reference.origin, &exchange_from(&reference, &target.replication_inventory().unwrap()));
        assert_lower_reference(&target, &reference, &receipt);
        assert!(target.owned_sets().unwrap()[0].blockers.is_empty());
        assert_replay_parity(&target, seed, arrivals.len() + 1);
    }
}

#[test]
fn independent_publishers_reusing_a_global_request_key_park_without_losing_receipts() {
    let base = Store::open_memory("base").unwrap();
    apply(&base, &bundle("initial", false), 1);
    let target = Store::open_memory("target").unwrap();
    receive_and_project(&target, "base", &exchange_from(&base, &target.replication_inventory().unwrap()));
    let branches = ["amber", "cobalt"].map(|origin| {
        let branch = Store::open_memory(origin).unwrap();
        share(&base, &branch);
        let input = bundle(origin, false);
        let mut opts = options(&branch, 10);
        let preview = branch.owned_set_preview(&input, &opts).unwrap();
        opts.expected_subjects = preview.expected_subjects;
        opts.confirm_retire = Some(preview.digest);
        branch.apply_owned_set(&input, &opts, "generated-0", "person/operator").unwrap();
        branch
    });
    let operation_id = smallclaims::store::opaque_cache_key("owned-set:garden:generated-0");
    let claims = branches.iter().map(|branch| {
        let (_, claim_id, state) = smallclaims::store::operation_tx(
            &branch.readers.get(), &operation_id,
        ).unwrap().unwrap();
        assert_eq!(state, "active");
        branch.claim_by_id(&claim_id).unwrap().unwrap()
    }).collect::<Vec<_>>();
    assert_eq!(claims[0].operation_id.as_deref(), Some("cache/511eb8217295350bdad7a9e1959bbe466b13ba15c444a42ce96d7688a4b8778a"));
    assert_eq!(claims[0].operation_id, claims[1].operation_id);
    assert!(claims.iter().all(|claim| claim.request_digest.is_some()));
    assert_ne!(claims[0].request_digest, claims[1].request_digest);
    receive_and_project(&target, "amber", &exchange_from(&branches[0], &target.replication_inventory().unwrap()));
    let physical = || {
        target.readers.get().query_row(
            "SELECT claim_id,revision,body,member FROM desired WHERE subject='agent/garden/orchard'",
            [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
        ).unwrap()
    };
    let last_good = physical();
    let frontier = target.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, Option<u64>>(0),
    ).unwrap();
    let exchange = exchange_from(&branches[1], &target.replication_inventory().unwrap());
    target.receive_replication_exchange("cobalt", TEST_FLEET, &exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    target.apply_replication_repairs().unwrap();
    smallclaims::store::FULL_REPLAYS.with(|count| count.set(0));
    assert!(!target.project_replication_backlog().unwrap());
    let parked = target.projection_recovery().unwrap().unwrap();
    assert_eq!(parked.reason_code, "operation-conflict");
    assert_eq!(parked.frontier, frontier);
    assert_eq!(physical(), last_good);
    assert!(target.claim_by_id(&claims[1].id).unwrap().is_some());
    assert!(!target.project_replication_backlog().unwrap());
    assert_eq!(physical(), last_good);
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), 0);
}

#[test]
fn t2_live_owned_set_receive_does_not_full_replay_under_writer() {
    let source = Store::open_memory("amber").unwrap();
    let target = Store::open_memory("cobalt").unwrap();
    // Establish a healthy frontier before measuring the live receive.
    target.project_replication_backlog().unwrap();
    apply(&source, &bundle("received", true), 10);
    let exchange = exchange_from(&source, &target.replication_inventory().unwrap());
    assert!(source.claims_for("owned-set/garden", Some("owned-set.revised")).unwrap().len() == 1);
    let before = smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get);
    receive_and_project(&target, "amber", &exchange);
    let after = smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get);
    assert_eq!(after, before, "live owned-set receive must never invoke full replay");
    assert!(target.owned_sets().unwrap()[0].blockers.is_empty());
    assert_eq!(target.owned_sets().unwrap()[0].receipt.source.sequence, 10);
    assert!(target.owned_member_guard("agent/garden/orchard").is_ok());
}

/// A real, earlier declaration is referenced by a later set receipt but withheld
/// from this receiver. The reference origin sorts first even on equal timestamps.
fn incomplete_revision(target: &Store) -> (Store, String) {
    let reference = Store::open_memory("aaa-reference").unwrap();
    direct(&reference, &bundle("late-reference", false), "late-reference").unwrap();
    let future = reference.claims_for("agent/garden/orchard", Some("intent.desired")).unwrap().pop().unwrap();
    let old = target.owned_sets().unwrap()[0].clone();
    let mut receipt = old.receipt.clone();
    receipt.previous = Some(format!("{}@{}", old.id, old.revision));
    receipt.source = options(target, 100).source;
    let desired: DesiredSubject = serde_json::from_value(future.body.clone()).unwrap();
    let member = receipt.members.get_mut("agent/garden/orchard").unwrap();
    member.claim = future.id;
    member.revision = desired_revision(&desired);
    let claim = target.append_claim(&ClaimInput {
        subject: old.id, kind: "owned-set.revised".into(), actor: None,
        fields: serde_json::from_value(json!({"revision": canonical_hash(&receipt).unwrap(), "body": receipt})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    (reference, claim.id)
}

fn assert_lower_reference(target: &Store, reference: &Store, receipt: &str) {
    let future = reference.claims_for("agent/garden/orchard", Some("intent.desired")).unwrap().pop().unwrap();
    let connection = target.readers.get();
    assert!(smallclaims::store::canonical::claim_key(&connection, &future.id).unwrap()
        < smallclaims::store::canonical::claim_key(&connection, receipt).unwrap(),
        "late reference must sort below the already projected set receipt");
}

#[test]
fn t5_late_lower_sorting_reference_applies_after_reopen_with_existing_frontier() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("receiver.sqlite3");
    let source = Store::open_memory("amber").unwrap();
    apply(&source, &bundle("initial", false), 10);
    let target = Store::open(&path, "receiver").unwrap();
    receive_and_project(&target, "amber", &exchange_from(&source, &target.replication_inventory().unwrap()));
    let (reference, receipt) = incomplete_revision(&target);
    assert!(!target.owned_sets().unwrap()[0].blockers.is_empty());
    assert!(target.project_replication_backlog().unwrap());
    let frontier = target.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, u64>(0)).unwrap();
    assert_eq!(frontier, target.index().unwrap());
    let held = effective_view(&target);
    drop(target);
    let target = Store::open(&path, "receiver").unwrap();
    assert_eq!(target.readers.get().query_row(
        "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
        [], |row| row.get::<_, u64>(0)).unwrap(), frontier);
    assert_eq!(effective_view(&target), held);
    receive_and_project(&target, &reference.origin, &exchange_from(&reference, &target.replication_inventory().unwrap()));
    assert_lower_reference(&target, &reference, &receipt);
    assert!(target.owned_sets().unwrap()[0].blockers.is_empty());
    assert!(target.owned_member_guard("agent/garden/orchard").is_ok());
    let future = reference.claims_for("agent/garden/orchard", Some("intent.desired")).unwrap().pop().unwrap();
    assert_eq!(target.selected_desired_token("agent/garden/orchard").unwrap(), Some(future.id));
    assert_replay_parity(&target, 0, 0);
}
