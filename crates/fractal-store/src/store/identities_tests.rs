use super::*;
use proptest::prelude::*;
use crate::store::fleet_admission_tests_shared_seats::{node, sync};

const ORIGINAL: &str = "agent/example-original/worker";
const RENAMED: &str = "agent/example-renamed/worker";

fn declare(store: &Store, name: &str) -> String {
    let name = name.strip_prefix("agent/").unwrap();
    let source = format!("version 2\nagent \"{name}\" {{ command \"true\"; workspace \"/tmp\"; restart \"never\"; }}\n");
    let intent = crate::parse_intent(&source, store.origin()).unwrap();
    let expected = intent.subjects.keys().map(|subject| {
        (subject.clone(), store.selected_desired_token(subject).unwrap().into_iter().collect())
    }).collect();
    store.apply(&intent, &expected, &format!("declare-{name}")).unwrap();
    store.selected_desired_token(&format!("agent/{name}")).unwrap().unwrap()
}

fn request(from: &str, to: &str, key: &str) -> RenameRequest {
    RenameRequest {
        from: from.into(), to: to.into(), actor: "person/robin".into(),
        reason: "Clarify responsibility".into(), expected_naming_revision: None,
        expected_declaration: None, idempotency_key: key.into(),
    }
}

#[test]
fn desired_reads_fence_pending_lifecycle_without_a_naming_head() {
    for state in ["unknown", "invalid"] {
        for kind in ["agent.retired", "intent.desired", "agent.renamed"] {
            for declared in [false, true] {
            let store = Store::open_memory("example").unwrap();
            if declared { declare(&store, ORIGINAL); }
            let connection = store.connection.write();
            let heads: u64 = connection.query_row(
                "SELECT COUNT(*) FROM identity_heads WHERE identity=?1", [ORIGINAL], |row| row.get(0),
            ).unwrap();
            assert_eq!(heads, 0);
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h80e80dafa3db owner=fractal_store::store::identities_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,subject_hint,kind_hint,updated_at_unix_ms)
                 VALUES('record/pending','example-peer',1,'pending',0,'{}',?1,?2,?3,'1')",
                params![state, ORIGINAL, kind],
            ).unwrap();
            let expected = if state == "invalid" { "conflict" } else { "pending-replication" };
            let error = current_desired_row(&connection, ORIGINAL).err().unwrap();
            let error = error.downcast_ref::<FractalError>().unwrap();
            assert_eq!(error.code, "lifecycle-unavailable");
            assert_eq!(error.details["reason"], expected);
            drop(connection);
            let error = store.selected_desired_tokens(&[ORIGINAL]).unwrap_err();
            let error = error.downcast_ref::<FractalError>().unwrap();
            assert_eq!(error.code, "lifecycle-unavailable");
            assert_eq!(error.details["reason"], expected);
            }
        }
    }
}

#[test]
fn replicated_rename_request_digest_survives_canonical_json_field_order() {
    for kind in [IdentityKind::Agent, IdentityKind::Mission] {
        let input = request(ORIGINAL, RENAMED, "canonical-request");
        let request = serde_json::to_value(input).unwrap();
        let canonical = canonical_json_value(&request);
        assert_eq!(
            request_digest(kind.claim_kind(), &request).unwrap(),
            request_digest(kind.claim_kind(), &canonical).unwrap(),
        );
    }
    let store = Store::open_memory("example").unwrap();
    declare(&store, ORIGINAL);
    let receipt = store.rename_identity(
        IdentityKind::Agent, &request(ORIGINAL, RENAMED, "canonical-replication"), &[],
    ).unwrap();
    let mut claim = store.claim_by_id(&receipt.claim_id).unwrap().unwrap();
    claim.body = canonical_json_value(&claim.body);
    validate_replicated(&claim).unwrap();
}

#[test]
fn rename_resolves_old_and_current_names_without_replacing_declaration() {
    let store = Store::open_memory("example").unwrap();
    let token = declare(&store, ORIGINAL);
    store.append_claim(&ClaimInput {
        subject: ORIGINAL.into(), kind: "runtime.observed".into(),
        actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("status".into(), json!("running")),
            ("incarnation_id".into(), json!("example-incarnation")),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let native = store.append_claim(&ClaimInput {
        subject: ORIGINAL.into(), kind: "harness.session-file".into(),
        actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("harness".into(), json!("omp")),
            ("session_id".into(), json!("example-native")),
            ("incarnation_id".into(), json!("example-incarnation")),
            ("desired_token".into(), json!(token)),
            ("status".into(), json!("active")),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: Some("native-binding".into()),
    }).unwrap();
    let receipt = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "rename"), &[]).unwrap();
    assert_eq!(receipt.identity, ORIGINAL);
    assert_eq!(receipt.canonical_id, RENAMED);
    for name in [ORIGINAL, RENAMED] {
        let resolved = store.resolve_identity(IdentityKind::Agent, name).unwrap();
        assert_eq!(resolved.identity, ORIGINAL);
        assert_eq!(resolved.canonical_id, RENAMED);
        let binding = store.latest_claim(&resolved.identity, Some("harness.session-file")).unwrap().unwrap();
        assert_eq!(binding.id, native.id);
        assert_eq!(binding.body, native.body);
        assert_eq!(binding.body["fields"]["session_id"], "example-native");
        assert_eq!(binding.body["fields"]["incarnation_id"], "example-incarnation");
    }
    assert_eq!(store.selected_desired_token(ORIGINAL).unwrap().as_deref(), Some(token.as_str()));
    assert!(store.latest_claim(RENAMED, Some("harness.session-file")).unwrap().is_none());
}

#[test]
fn retry_is_idempotent_and_changed_request_is_not() {
    let store = Store::open_memory("example").unwrap();
    declare(&store, ORIGINAL);
    let input = request(ORIGINAL, RENAMED, "retry");
    let first = store.rename_identity(IdentityKind::Agent, &input, &[]).unwrap();
    let repeat = store.rename_identity(IdentityKind::Agent, &input, &[]).unwrap();
    assert_eq!(first.claim_id, repeat.claim_id);
    assert_eq!(repeat.applied, "already-applied");
    let mut changed = input;
    changed.to = "agent/example-other".into();
    assert!(store.rename_identity(IdentityKind::Agent, &changed, &[]).is_err());
}

#[test]
fn unknown_cross_kind_occupied_cycle_and_fleet_refusals() {
    let store = Store::open_memory("example").unwrap();
    let error = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "unknown"), &[]).unwrap_err();
    assert_eq!(error.code, "identity-unknown");
    declare(&store, ORIGINAL);
    declare(&store, "agent/example-occupied/worker");
    let error = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, "mission/example-other", "cross-kind"), &[]).unwrap_err();
    assert_eq!(error.code, "identity-cross-kind");
    let error = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, "agent/example-occupied/worker", "occupied"), &[]).unwrap_err();
    assert_eq!(error.code, "identity-occupied");
    let error = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "fleet"), &["example-peer".into()]).unwrap_err();
    assert_eq!(error.code, "lifecycle-unavailable");
    assert_eq!(error.details["reason"], "incompatible-fleet");
    store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "forward"), &[]).unwrap();
    let error = store.rename_identity(IdentityKind::Agent, &request(RENAMED, ORIGINAL, "cycle"), &[]).unwrap_err();
    assert_eq!(error.code, "identity-cycle");
}

#[test]
fn declaration_and_naming_revision_compare_and_swap() {
    let store = Store::open_memory("example").unwrap();
    declare(&store, ORIGINAL);
    let mut input = request(ORIGINAL, RENAMED, "stale-declaration");
    input.expected_declaration = Some("not-the-selected-declaration".into());
    let error = store.rename_identity(IdentityKind::Agent, &input, &[]).unwrap_err();
    assert_eq!(error.code, "lifecycle-unavailable");
    assert_eq!(error.details["reason"], "stale-revision");
    let first = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "first"), &[]).unwrap();
    let mut stale = request(RENAMED, "agent/example-third", "stale-naming");
    stale.expected_naming_revision = Some("not-the-naming-revision".into());
    assert_eq!(store.rename_identity(IdentityKind::Agent, &stale, &[]).unwrap_err().details["reason"], "stale-revision");
    stale.idempotency_key = "current-naming".into();
    stale.expected_naming_revision = Some(first.naming_revision);
    store.rename_identity(IdentityKind::Agent, &stale, &[]).unwrap();
}

#[test]
fn real_two_store_replication_preserves_old_name_resolution() {
    let owner = node("example-owner", None, None);
    let peer = node("example-peer", None, None);
    declare(&owner, ORIGINAL);
    sync(&owner, &peer);
    sync(&peer, &owner);
    let receipt = owner.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "replicated"), &[]).unwrap();
    sync(&owner, &peer);
    sync(&peer, &owner);
    for store in [&owner, &peer] {
        for name in [ORIGINAL, RENAMED] {
            let resolved = store.resolve_identity(IdentityKind::Agent, name).unwrap();
            assert_eq!(resolved.identity, ORIGINAL);
            assert_eq!(resolved.canonical_id, RENAMED);
            assert_eq!(resolved.naming_revision.as_deref(), Some(receipt.naming_revision.as_str()));
        }
        assert!(store.claim_by_id(&receipt.claim_id).unwrap().is_some());
    }
    let refusal = peer.rename_identity(
        IdentityKind::Agent, &request(RENAMED, "agent/example-third", "non-owner"), &[],
    ).unwrap_err();
    assert_eq!(refusal.code, "lifecycle-unavailable");
    assert_eq!(refusal.details["reason"], "owner-unavailable");
}
#[test]
fn retiring_storage_identity_refuses_rename_through_every_alias() {
    let store = Store::open_memory("example").unwrap();
    let declaration = declare(&store, ORIGINAL);
    store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "rename-before-retirement"), &[]).unwrap();
    let retirement = ClaimInput {
        subject: ORIGINAL.into(), kind: "agent.retired".into(), actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("reason".into(), json!("Responsibility ended")),
            ("retired_by".into(), json!("person/robin")),
            ("declaration".into(), json!(declaration)),
            ("owner_host".into(), json!(store.origin())),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: Some("retire".into()),
    };
    append_claim_with_commit_context(
        &store.graph, &retirement, None, None, None, None,
        ClaimCommitContext { retirement_writer: true, ..Default::default() },
    ).unwrap();
    for (index, source) in [ORIGINAL, RENAMED].into_iter().enumerate() {
        let error = store.rename_identity(
            IdentityKind::Agent, &request(source, "agent/example-successor", &format!("retired-{index}")), &[],
        ).unwrap_err();
        assert_eq!(error.code, "identity-retired-source");
    }
}


#[test]
fn partition_target_collision_has_same_winner_in_both_arrival_orders() {
    let left = node("example-left", None, None);
    let right = node("example-right", None, None);
    let first = node("example-first", None, None);
    let second = node("example-second", None, None);
    let other = "agent/example-other/worker";
    let target = "agent/example-target";
    declare(&left, ORIGINAL);
    declare(&right, other);
    // Exchange declarations and registry advertisements before the partition.
    sync(&left, &right);
    sync(&right, &left);
    let a = left.rename_identity(IdentityKind::Agent, &request(ORIGINAL, target, "left"), &[]).unwrap();
    let b = right.rename_identity(IdentityKind::Agent, &request(other, target, "right"), &[]).unwrap();
    sync(&left, &first);
    sync(&right, &first);
    sync(&right, &second);
    sync(&left, &second);
    let (winner, loser) = if a.claim_id < b.claim_id { (ORIGINAL, other) } else { (other, ORIGINAL) };
    for store in [&first, &second] {
        let resolved = store.resolve_identity(IdentityKind::Agent, target).unwrap();
        assert_eq!(resolved.identity, winner);
        assert_eq!(resolved.canonical_id, target);
        let error = store.resolve_identity(IdentityKind::Agent, loser).unwrap_err();
        assert_eq!(error.code, "lifecycle-unavailable");
        assert_eq!(error.details["reason"], "conflict");
        assert!(store.claim_by_id(&a.claim_id).unwrap().is_some());
        assert!(store.claim_by_id(&b.claim_id).unwrap().is_some());
        let retry = request(loser, target, if loser == ORIGINAL { "left" } else { "right" });
        let error = store.rename_identity(IdentityKind::Agent, &retry, &[]).unwrap_err();
        assert_eq!(error.code, "lifecycle-unavailable");
        assert_eq!(error.details["reason"], "conflict");
    }
}

#[test]
fn replicated_declarations_keep_signed_subjects_across_arrival_order_and_replay() {
    let owner = node("example-owner", None, None);
    let independent = node("example-independent", None, None);
    let rename_first = node("example-rename-first", None, None);
    let declaration_first = node("example-declaration-first", None, None);
    let desired_rows = |store: &Store| {
        let connection = store.readers.get();
        let mut statement = connection.prepare(
            "SELECT subject,kind,revision,claim_id,body,member,owner_run,owner_generation,owner_step FROM desired ORDER BY subject",
        ).unwrap();
        statement.query_map([], |row| {
            (0..9).map(|column| row.get::<_, Option<String>>(column)).collect::<rusqlite::Result<Vec<_>>>()
        }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    declare(&owner, ORIGINAL);
    declare(&independent, RENAMED);
    let mut expected = desired_rows(&owner);
    expected.extend(desired_rows(&independent));
    expected.sort();
    assert_eq!(expected.len(), 2);
    owner.rename_identity(
        IdentityKind::Agent, &request(ORIGINAL, RENAMED, "partition-rename"), &[],
    ).unwrap();

    sync(&owner, &rename_first);
    sync(&independent, &rename_first);
    sync(&independent, &declaration_first);
    sync(&owner, &declaration_first);

    for store in [&rename_first, &declaration_first] {
        // Inspect literal storage rows, not an alias-resolving desired lookup.
        let before = desired_rows(store);
        assert_eq!(before, expected);
        for row in &before {
            let subject = row[0].as_deref().unwrap();
            let claim = store.claim_by_id(row[3].as_deref().unwrap()).unwrap().unwrap();
            assert_eq!(claim.subject, subject);
            assert_eq!(claim.kind, "intent.desired");
        }
        store.replay_replication_graph().unwrap();
        assert_eq!(desired_rows(store), before);
    }
    assert_eq!(desired_rows(&rename_first), desired_rows(&declaration_first));
}

#[test]
fn naming_writer_cannot_be_bypassed_and_full_replay_preserves_resolution() {
    let store = Store::open_memory("example").unwrap();
    declare(&store, ORIGINAL);
    let receipt = store.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "replay"), &[]).unwrap();
    let claim = store.claim_by_id(&receipt.claim_id).unwrap().unwrap();
    let direct = ClaimInput {
        subject: ORIGINAL.into(), kind: AGENT_RENAMED.into(), actor: Some("person/robin".into()),
        fields: serde_json::from_value(claim.body["fields"].clone()).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: Some("bypass".into()),
    };
    assert_eq!(store.append_claim(&direct).unwrap_err().code, "invalid-rename");
    let before = store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap();
    store.replay_replication_graph().unwrap();
    assert_eq!(store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap(), before);
    assert_eq!(store.resolve_identity(IdentityKind::Agent, RENAMED).unwrap(), before);
    assert!(store.claim_by_id(&receipt.claim_id).unwrap().is_some());
}

#[test]
fn missing_predecessor_fences_resolution_and_retains_the_fact() {
    let store = Store::open_memory("example").unwrap();
    let declaration = declare(&store, ORIGINAL);
    let missing = ClaimInput {
        subject: ORIGINAL.into(), kind: AGENT_RENAMED.into(), actor: Some("person/robin".into()),
        fields: BTreeMap::from([
            ("from".into(), json!(ORIGINAL)),
            ("to".into(), json!(RENAMED)),
            ("reason".into(), json!("Clarify responsibility")),
            ("owner_host".into(), json!(store.origin())),
            ("declaration".into(), json!(declaration)),
            ("previous_revision".into(), json!("missing-predecessor")),
            ("request".into(), json!(request(ORIGINAL, RENAMED, "missing"))),
            ("preserved_incarnation".into(), Value::Null),
        ]),
        evidence: vec![], expected_subject: None, idempotency_key: Some("missing".into()),
    };
    store.graph.ensure_principal_key("person/robin").unwrap();
    let fact = append_claim_with_commit_context(
        &store.graph, &missing, None, None, None, None,
        ClaimCommitContext { identity_writer: true, ..Default::default() },
    ).unwrap().0;
    for name in [ORIGINAL, RENAMED] {
        let error = store.resolve_identity(IdentityKind::Agent, name).unwrap_err();
        assert_eq!(error.code, "lifecycle-unavailable");
        assert_eq!(error.details["reason"], "pending-replication");
    }
    assert!(store.claim_by_id(&fact.id).unwrap().is_some());
}

#[test]
fn repaired_naming_fact_does_not_release_historical_name_reservation() {
    let owner = node("example-owner", None, None);
    let peer = node("example-peer", None, None);
    declare(&owner, ORIGINAL);
    let receipt = owner.rename_identity(IdentityKind::Agent, &request(ORIGINAL, RENAMED, "repair"), &[]).unwrap();
    sync(&owner, &peer);
    {
        let mut connection = peer.connection.write();
        let transaction = connection.transaction().unwrap();
        let changed = transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hcb882e211fa6 owner=fractal_store::store::identities_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired' WHERE claim_id=?1",
            [&receipt.claim_id],
        ).unwrap();
        assert!(changed > 0);
        flush(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    let resolved = peer.resolve_identity(IdentityKind::Agent, RENAMED).unwrap();
    assert_eq!(resolved.identity, ORIGINAL);
    assert_eq!(resolved.canonical_id, ORIGINAL);
    peer.replay_replication_graph().unwrap();
    let replayed = peer.resolve_identity(IdentityKind::Agent, RENAMED).unwrap();
    assert_eq!(replayed, resolved);
    declare(&peer, "agent/example-other/worker");
    let refused = peer.rename_identity(
        IdentityKind::Agent, &request("agent/example-other/worker", RENAMED, "never-reuse"), &[],
    ).unwrap_err();
    assert_eq!(refused.code, "identity-occupied");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn alias_chains_are_acyclic_and_names_have_one_immutable_identity(length in 1usize..10) {
        let store = Store::open_memory("example").unwrap();
        declare(&store, ORIGINAL);
        let mut names = vec![ORIGINAL.to_owned()];
        for index in 0..length {
            let current = names.last().unwrap();
            let next = format!("agent/example-chain-{index}");
            store.rename_identity(IdentityKind::Agent, &request(current, &next, &format!("chain-{index}")), &[]).unwrap();
            names.push(next);
            for name in &names {
                let resolved = store.resolve_identity(IdentityKind::Agent, name).unwrap();
                prop_assert_eq!(&resolved.identity, ORIGINAL);
                prop_assert_eq!(&resolved.canonical_id, names.last().unwrap());
                let again = store.resolve_identity(IdentityKind::Agent, &resolved.canonical_id).unwrap();
                prop_assert_eq!(again.identity, resolved.identity);
            }
            let error = store.rename_identity(IdentityKind::Agent, &request(names.last().unwrap(), ORIGINAL, &format!("reject-{index}")), &[]).unwrap_err();
            prop_assert_eq!(error.code, "identity-cycle");
        }
    }
}

#[test]
fn mixed_structural_and_neutral_operation_history_requires_graph_repair_in_both_orders() {
    for neutral_first in [false, true] {
        let source = node("node-a", None, None);
        let store = node("node-b", None, None);
        declare(&source, ORIGINAL);
        sync(&source, &store);
        store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap();
        let receipt = source.rename_identity(
            IdentityKind::Agent, &request(ORIGINAL, RENAMED, "mixed-operation"), &[],
        ).unwrap();
        let rename = source.claim_by_id(&receipt.claim_id).unwrap().unwrap();
        let (operation, _) = operation_parts(&rename.body).unwrap();
        let neutral_body = json!({"fields": {}, "_operation": {
            "id": operation, "request_digest": "invented-neutral-digest"
        }});
        for neutral in [neutral_first, !neutral_first] {
            {
                let mut connection = store.connection.write();
                let tx = connection.transaction().unwrap();
                let (subject, kind, actor, body) = if neutral {
                    ("github-event/github/acme/garden/pull-request/7/checks/failed",
                        "github.event", None, &neutral_body)
                } else {
                    (rename.subject.as_str(), rename.kind.as_str(), rename.actor.as_deref(), &rename.body)
                };
                append_claim_record_tx(&tx, source.origin(), subject, kind, actor, body, &[], None).unwrap();
                tx.commit().unwrap();
            }
            if neutral != neutral_first {
                let mut connection = store.connection.write();
                let tx = connection.transaction().unwrap();
                let result = try_project_simple_replication_tx(
                    &tx, store.origin(), current_index_tx(&tx).unwrap(),
                ).unwrap();
                // A mixed history must never take the graph-neutral registry-only path.
                assert!(matches!(result, IncrementalProjection::ProjectedWithContext {
                    code: "operation-conflict", ..
                }), "neutral_first={neutral_first}");
                tx.rollback().unwrap();
            }
            store.project_replication_backlog().unwrap();
        }
        assert_eq!(operation_tx(&store.readers.get(), operation).unwrap().unwrap().2, "conflict");
        let error = store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap_err();
        assert_eq!(error.code, "lifecycle-unavailable");
        assert_eq!(error.details["reason"], "conflict");
        let digests = store.replication_snapshot().unwrap().projection_digests.clone();
        store.replay_replication_graph().unwrap();
        let replayed = store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap_err();
        assert_eq!(replayed.code, error.code);
        assert_eq!(replayed.details["reason"], error.details["reason"]);
        assert_eq!(store.replication_snapshot().unwrap().projection_digests, digests);
    }
}

#[test]
fn naming_operation_conflict_above_bound_defers_and_explicit_repair_recovers() {
    let store = node("node-a", None, None);
    let declaration = declare(&store, ORIGINAL);
    let receipt = store.rename_identity(
        IdentityKind::Agent, &request(ORIGINAL, RENAMED, "large-naming-conflict"), &[],
    ).unwrap();
    let rename = store.claim_by_id(&receipt.claim_id).unwrap().unwrap();
    let (operation, _) = operation_parts(&rename.body).unwrap();
    let declaration = store.claim_by_id(&declaration).unwrap().unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for _ in 0..OPERATION_CONFLICT_NAMING_CLAIMS {
            append_claim_record_tx(&tx, store.origin(), ORIGINAL, "intent.desired",
                declaration.actor.as_deref(), &declaration.body, &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    assert_eq!(store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap().canonical_id, RENAMED);
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, store.origin(), "github-event/github/acme/garden/issue/7/closed",
            "github.event", None, &json!({"fields":{},"_operation":{"id":operation,"request_digest":"different"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert!(store.replication_projection_needs_recovery().unwrap());
    let health = store.replication_status(false, None, &[]).unwrap();
    let degraded = health.unhealthy.iter().find(|row|
        row.aggregate == "projection:naming-operation-conflict").unwrap();
    assert_eq!(degraded.error_code.as_deref(), Some("operation-conflict-naming-refold-limit"));
    let details: Value = serde_json::from_str(degraded.error_message.as_ref().unwrap()).unwrap();
    assert_eq!(details["projection_operation_id"], operation);
    assert_eq!(store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap_err().code, "lifecycle-unavailable");
    assert!(resolve_reference_conn(&store.readers.get(), RENAMED).is_err());
    assert!(store.selected_desired_tokens(&[ORIGINAL]).is_err());
    assert!(store.selected_desired_token(ORIGINAL).is_err());
    store.replay_replication_graph().unwrap();
    assert!(!store.replication_projection_needs_recovery().unwrap());
    assert!(!store.replication_status(false, None, &[]).unwrap().unhealthy.iter()
        .any(|row| row.aggregate == "projection:naming-operation-conflict"));
    let error = store.resolve_identity(IdentityKind::Agent, ORIGINAL).unwrap_err();
    assert_eq!(error.details["reason"], "conflict");
    let digest = store.replication_snapshot().unwrap().projection_digests.clone();
    store.replay_replication_graph().unwrap();
    assert_eq!(digest, store.replication_snapshot().unwrap().projection_digests);
}

#[test]
#[ignore = "explicit dirty-mark mutation probe; intentionally catches the parity assertion failure"]
fn naming_operation_conflict_dirty_mark_mutation_is_detected() {
    OMIT_OPERATION_CONFLICT_DIRTY.set(true);
    let mutant = std::panic::catch_unwind(
        mixed_structural_and_neutral_operation_history_requires_graph_repair_in_both_orders);
    OMIT_OPERATION_CONFLICT_DIRTY.set(false);
    assert!(mutant.is_err(), "removing the dirty mark must fail the mixed-history regression");
    eprintln!("NAMING_DIRTY_MUTATION: mixed-history regression failed with the dirty mark omitted");
}

#[test]
#[ignore = "writer-held naming refold measurement; run explicitly under a build ticket"]
fn naming_operation_conflict_writer_hold_measurement() {
    for count in [16, 32, 64, 128, 256, 512, 1024, 2048, 4096] {
        let store = Store::open_memory("naming-bench").unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            for number in 0..count / 2 {
                let identity = format!("mission/bench-{number}");
                let declaration = append_claim_record_tx(&tx, store.origin(), &identity,
                    "mission.published", None, &json!({}), &[], None).unwrap();
                let rename = append_claim_record_tx(&tx, store.origin(), &identity, "mission.renamed",
                    None, &json!({"fields":{"from":identity,"to":format!("mission/renamed-{number}"),
                        "declaration":declaration.id,"owner_host":store.origin()},
                        "_operation":{"id":format!("op/bench-{number}"),"request_digest":"benchmark"}}),
                    &[], None).unwrap();
                register_operation_tx(&tx, &rename).unwrap();
            }
            rebuild(&tx).unwrap();
            tx.commit().unwrap();
        }
        let mut samples = Vec::new();
        for iteration in 0..110 {
            let started = std::time::Instant::now();
            {
                let mut connection = store.connection.write();
                let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
                tx.execute(/* sqlite-large-value: allow-for-now lv-h18f8ad5b42e2 owner=fractal_store::store::identities_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE operations SET state='conflict' WHERE id='op/bench-0'", []).unwrap();
                mark_operation_conflict_dirty(&tx).unwrap();
                // Explicit benchmark of the canonical fold even if a measured lower cap
                // later makes this history too large for automatic receipt repair.
                reduce(&tx, false).unwrap();
                tx.rollback().unwrap();
            }
            if iteration >= 10 { samples.push(started.elapsed().as_micros()); }
        }
        samples.sort_unstable();
        eprintln!("NAMING_REFOLD_BENCH claims={count} samples=100 p50_us={} p99_us={}",
            samples[49], samples[98]);
    }
}

#[test]
fn naming_operation_conflict_scope_fences_other_claimants_not_unrelated_names() {
    const OTHER: &str = "agent/example-other/worker";
    const UNRELATED: &str = "agent/example-unrelated/worker";
    const DESTINATION: &str = "agent/example-contested/worker";
    let a = node("node-a", None, None);
    let b = node("node-b", None, None);
    let store = node("node-c", None, None);
    declare(&a, ORIGINAL);
    declare(&b, OTHER);
    let first = a.rename_identity(IdentityKind::Agent,
        &request(ORIGINAL, DESTINATION, "scope-a"), &[]).unwrap();
    let second = b.rename_identity(IdentityKind::Agent,
        &request(OTHER, DESTINATION, "scope-b"), &[]).unwrap();
    let first = a.claim_by_id(&first.claim_id).unwrap().unwrap();
    let second = b.claim_by_id(&second.claim_id).unwrap().unwrap();
    sync(&a, &store);
    sync(&b, &store);
    let declaration = declare(&store, UNRELATED);
    let declaration = store.claim_by_id(&declaration).unwrap().unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for _ in 0..OPERATION_CONFLICT_NAMING_CLAIMS {
            append_claim_record_tx(&tx, store.origin(), UNRELATED, "intent.desired",
                declaration.actor.as_deref(), &declaration.body, &[], None).unwrap();
        }
        tx.commit().unwrap();
    }
    store.replay_replication_graph().unwrap();
    let (winner, loser) = if first.id < second.id { (&first, &second) } else { (&second, &first) };
    assert_eq!(store.resolve_identity(IdentityKind::Agent, &winner.subject).unwrap().canonical_id, DESTINATION);
    let (operation, _) = operation_parts(&loser.body).unwrap();
    {
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_record_tx(&tx, store.origin(), "custom/test/naming-scope", "custom.test.observed",
            None, &json!({"fields":{},"_operation":{"id":operation,"request_digest":"different"}}),
            &[], None).unwrap();
        tx.commit().unwrap();
    }
    FULL_REPLAYS.with(|count| count.set(0));
    store.project_replication_backlog().unwrap();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    // The conflicting operation belongs to the loser. Its scope must also fence the
    // currently healthy winner, which a subject-only or cache-derived scope would miss.
    let error = store.resolve_identity(IdentityKind::Agent, &winner.subject).unwrap_err();
    assert_eq!(error.details["reason"], "operation-conflict-naming-refold-limit");
    assert!(store.resolve_identity(IdentityKind::Agent, &loser.subject).is_err());
    assert!(resolve_reference_conn(&store.readers.get(), DESTINATION).is_err());
    assert_eq!(store.resolve_identity(IdentityKind::Agent, UNRELATED).unwrap().identity, UNRELATED);
    assert!(store.selected_desired_tokens(&[UNRELATED]).unwrap().contains_key(UNRELATED));
    let health = store.replication_status(false, None, &[]).unwrap();
    let degraded = health.unhealthy.iter().find(|row|
        row.aggregate == "projection:naming-operation-conflict").unwrap();
    let report: Value = serde_json::from_str(degraded.error_message.as_ref().unwrap()).unwrap();
    assert_eq!(report["scope"], "scoped");
    assert!(report["affected_names"].as_array().unwrap().contains(&json!(winner.subject)));
    assert!(report["affected_names"].as_array().unwrap().contains(&json!(DESTINATION)));
    assert_eq!(report["repair_command"], "run fractal replication replay --confirm in a planned window");
    let scoped_keys: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM meta WHERE key GLOB 'operation-conflict-naming-name:*'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(scoped_keys, report["affected_names"].as_array().unwrap().len());
    // A typed nested reference in a non-naming declaration must use the same fence.
    let mut desired: DesiredSubject = serde_json::from_value(declaration.body.clone()).unwrap();
    desired.subject = "resource/example-nested".into();
    desired.kind = "resource".into();
    desired.desired = json!({"name":"resource","arguments":["example-nested"],
        "children":[{"name":"to","arguments":[winner.subject]}]});
    assert!(normalize_desired(&store.readers.get(), &desired).is_err());
    let mut intent = crate::parse_intent("version 2\nagent \"example-intent\" { command \"true\"; workspace \"/tmp\"; }\n", store.origin()).unwrap();
    intent.subjects.clear();
    intent.subjects.insert(desired.subject.clone(), desired);
    assert!(normalize_intent(&store.readers.get(), &intent).is_err());
    store.replay_replication_graph().unwrap();
    let scoped_keys: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM meta WHERE key GLOB 'operation-conflict-naming-name:*'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(scoped_keys, 0, "explicit repair clears keyed failure scope");
    assert!(!store.replication_projection_needs_recovery().unwrap());
    assert!(!store.replication_status(false, None, &[]).unwrap().unhealthy.iter()
        .any(|row| row.aggregate == "projection:naming-operation-conflict"));
    assert_eq!(store.resolve_identity(IdentityKind::Agent, &winner.subject).unwrap().canonical_id, DESTINATION);
    assert_eq!(store.resolve_identity(IdentityKind::Agent, UNRELATED).unwrap().identity, UNRELATED);
    let reference = store.replication_snapshot().unwrap().projection_digests.clone();
    store.replay_replication_graph().unwrap();
    assert_eq!(reference, store.replication_snapshot().unwrap().projection_digests);
}

#[test]
#[ignore = "explicit scope mutation probe; intentionally catches a stale healthy competitor"]
fn naming_operation_conflict_other_claimants_mutation_is_detected() {
    conflict_scope::OMIT_OTHER_CLAIMANTS.set(true);
    let mutant = std::panic::catch_unwind(
        naming_operation_conflict_scope_fences_other_claimants_not_unrelated_names);
    conflict_scope::OMIT_OTHER_CLAIMANTS.set(false);
    assert!(mutant.is_err(), "omitting other claimants must fail the scope regression");
    eprintln!("NAMING_SCOPE_MUTATION: other-claimant regression failed when competitor traversal was omitted");
}
