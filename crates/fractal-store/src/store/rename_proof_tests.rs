//! Real in-memory SQLite stores, real member keys and sealed replication bytes; no mocks.
use super::*;
use super::identities::{IdentityKind, IdentityStore, RenameReceipt, RenameRequest};
use crate::store::fleet_admission_tests_shared_seats::{FLEET, admit, key, node, sync};
use smallclaims::proof::{RenameProofBundle, verify_rename_proof};

const ORIGINAL: &str = "agent/example-original/worker";
const RENAMED: &str = "agent/example-renamed/worker";
const FINAL: &str = "agent/example-final/worker";
const ACTOR: &str = "person/operator";

fn signed_store() -> (Store, Arc<crate::fleet::MemberKey>) {
    let member = key();
    let store = node("example", Some(&member), Some(&member));
    store.admit_fleet_anchor(FLEET, member.public(), "listening").unwrap();
    store.ensure_principal_key(ACTOR).unwrap();
    (store, member)
}

fn publish(store: &Store, source: &str, operation: &str) {
    let intent = crate::parse_intent(source, store.origin()).unwrap();
    let preview = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply_as(&intent, &preview.subject_tokens, operation, Some(ACTOR)).unwrap();
    store.seal_local_batches().unwrap();
}

fn rename(store: &Store, kind: IdentityKind, from: &str, to: &str, operation: &str) -> RenameReceipt {
    let receipt = store.rename_identity(kind, &RenameRequest {
        from: from.into(), to: to.into(), actor: ACTOR.into(), reason: "Clarify responsibility".into(),
        expected_naming_revision: None, expected_declaration: None, idempotency_key: operation.into(),
    }, &[]).unwrap();
    store.seal_local_batches().unwrap();
    receipt
}

fn twice_renamed() -> (Store, Arc<crate::fleet::MemberKey>, RenameReceipt, RenameReceipt) {
    let (store, member) = signed_store();
    publish(&store, "version 2\nagent \"example-original/worker\" { command \"true\"; workspace \"/tmp\"; restart \"never\"; }\n", "declare-agent");
    let first = rename(&store, IdentityKind::Agent, ORIGINAL, RENAMED, "first");
    let second = rename(&store, IdentityKind::Agent, RENAMED, FINAL, "second");
    (store, member, first, second)
}

fn exported() -> (RenameProofBundle, Arc<crate::fleet::MemberKey>) {
    let (store, member, _, _) = twice_renamed();
    let proof = store.export_rename_proof(IdentityKind::Agent, FINAL, None).unwrap();
    // The verifier below cannot consult this store, its principal cache or its anchor.
    drop(store);
    (proof, member)
}

#[test]
fn real_agent_twice_rename_exports_every_alias_and_exact_historical_revision() {
    let (store, member, first, second) = twice_renamed();
    for name in [ORIGINAL, RENAMED, FINAL] {
        let proof = store.export_rename_proof(IdentityKind::Agent, name, None).unwrap();
        assert_eq!(proof.storage_id, ORIGINAL);
        assert_eq!(proof.canonical_id, FINAL);
        assert_eq!(proof.chain, [first.claim_id.clone(), second.claim_id.clone()]);
        assert_eq!(proof.naming_revision, second.naming_revision);
        verify_rename_proof(&proof, member.public(), Some(&first.naming_revision)).unwrap();
        let old = store.export_rename_proof(IdentityKind::Agent, name, Some(&first.naming_revision)).unwrap();
        assert_eq!(old.canonical_id, RENAMED);
        assert_eq!(old.chain.as_slice(), std::slice::from_ref(&first.claim_id));
        verify_rename_proof(&old, member.public(), None).unwrap();
    }
    let error = store.export_rename_proof(IdentityKind::Agent, ORIGINAL, Some("not-a-revision")).unwrap_err();
    assert_eq!(error.code, "stale-revision");
}

#[test]
fn real_mission_rename_proves_published_declaration() {
    let (store, member) = signed_store();
    publish(&store, "version 2\nmission \"example-original\" state=\"ready\" { goal \"Deliver the result.\"; step \"work\" { agentless } }\n", "publish-mission");
    let receipt = rename(&store, IdentityKind::Mission, "mission/example-original", "mission/example-renamed", "rename-mission");
    let proof = store.export_rename_proof(IdentityKind::Mission, "mission/example-renamed", None).unwrap();
    assert_eq!(proof.identity_kind, "mission");
    assert_eq!(proof.storage_id, "mission/example-original");
    assert_eq!(proof.canonical_id, "mission/example-renamed");
    assert_eq!(proof.naming_revision, receipt.naming_revision);
    verify_rename_proof(&proof, member.public(), None).unwrap();
}

#[test]
fn portable_proof_verifies_after_source_store_is_dropped() {
    let (proof, member) = exported();
    let wire = serde_json::to_vec(&proof).unwrap();
    let isolated: RenameProofBundle = serde_json::from_slice(&wire).unwrap();
    let verdict = verify_rename_proof(&isolated, member.public(), None).unwrap();
    let value = serde_json::to_value(verdict).unwrap();
    assert_eq!(value["schema"], "fractal.rename-proof-verdict.v1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["kind"], "agent");
    assert_eq!(value["storage_identity"], ORIGINAL);
    assert_eq!(value["from"], RENAMED);
    assert_eq!(value["to"], FINAL);
    assert_eq!(value["revision"], proof.naming_revision);
    assert_eq!(value["chain"][0]["from"], ORIGINAL);
    assert_eq!(value["chain"][0]["to"], RENAMED);
    assert_eq!(value["chain"][1]["from"], RENAMED);
    assert_eq!(value["chain"][1]["to"], FINAL);
    assert_eq!(value["actor"], ACTOR);
    assert_eq!(value["reason"], "Clarify responsibility");
    assert_eq!(value["owner_host"], "example");
    assert_eq!(value["signer_member"], "example");
    assert_eq!(value["signer_key"], member.public());
    assert_eq!(value["fleet_id"], FLEET);
    assert_eq!(value["storage_id"], ORIGINAL);
    assert_eq!(value["canonical_id"], FINAL);
}

#[test]
fn portable_proof_rejects_tampered_metadata_envelope_and_signature() {
    let (proof, member) = exported();
    let mut changed = proof.clone();
    changed.canonical_id = "agent/example-forged".into();
    let error = verify_rename_proof(&changed, member.public(), None).unwrap_err();
    assert_eq!(error.code, "malformed-bundle");
    assert_eq!(error.exit_code(), 6);

    let mut changed = proof.clone();
    let envelope = changed.envelopes.last_mut().unwrap();
    let mut bytes = envelope.payload.bytes().unwrap().to_vec();
    bytes[0] ^= 1;
    envelope.payload = bytes.into();
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "invalid-signature");

    let mut changed = proof;
    changed.envelopes.last_mut().unwrap().signature = Some("not-a-signature".into());
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "invalid-signature");
}

#[test]
fn portable_proof_rejects_outsider_member_wrong_anchor_and_truncated_history() {
    let (proof, member) = exported();
    let outsider = key();
    let mut changed = proof.clone();
    let envelope = changed.envelopes.last_mut().unwrap();
    envelope.member_key = Some(outsider.public().to_owned());
    envelope.signature = Some(outsider.sign(&crate::fleet::envelope_signature_message(
        &proof.fleet_id, &envelope.writer, envelope.sequence, &envelope.hash,
    )));
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "untrusted-membership");
    assert_eq!(verify_rename_proof(&proof, outsider.public(), None).unwrap_err().code, "untrusted-membership");

    let mut changed = proof;
    assert!(changed.envelopes.len() > 2);
    changed.envelopes.remove(1);
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "broken-chain");
}

#[test]
fn after_revision_requires_strict_proven_ancestry_not_lexical_order() {
    let (proof, member) = exported();
    verify_rename_proof(&proof, member.public(), Some(&proof.chain[0])).unwrap();
    for token in [&proof.naming_revision[..], "absent-revision", "0", "999999999"] {
        let error = verify_rename_proof(&proof, member.public(), Some(token)).unwrap_err();
        assert_eq!(error.code, "stale-revision");
    }
}

#[test]
fn portable_proof_rejects_claim_fields_and_signatures_inside_valid_member_envelopes() {
    let (original, member) = exported();
    for tamper_signature in [false, true] {
        let mut proof = original.clone();
        let envelope = proof.envelopes.last_mut().unwrap();
        let mut payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        if tamper_signature {
            payload.claim_signatures.get_mut(&proof.naming_revision).unwrap().signature = "invalid-claim-signature".into();
        } else {
            let claim = payload.batch.claims.iter_mut().find(|claim| claim.id == proof.naming_revision).unwrap();
            claim.body["fields"]["to"] = json!("agent/example-forged");
        }
        let mut bytes = Vec::new();
        ciborium::into_writer(&payload, &mut bytes).unwrap();
        envelope.hash = smallclaims::hash::replica_envelope_hash(
            &envelope.writer, envelope.sequence, envelope.previous_hash.as_deref(),
            envelope.accepted_at_unix_ms, &bytes,
        );
        envelope.payload = bytes.into();
        envelope.signature = Some(member.sign(&crate::fleet::envelope_signature_message(
            &proof.fleet_id, &envelope.writer, envelope.sequence, &envelope.hash,
        )));
        assert_eq!(verify_rename_proof(&proof, member.public(), None).unwrap_err().code, "invalid-signature");
    }
}

#[test]
fn renamed_actor_alias_is_proven_against_immutable_signed_actor() {
    let (store, member) = signed_store();
    publish(&store, "version 2\nagent \"example-actor/worker\" { command \"true\"; }\nagent \"example-target/worker\" { command \"true\"; }\n", "declare-actor-and-target");
    rename(&store, IdentityKind::Agent, "agent/example-actor/worker", "agent/example-actor-alias/worker", "rename-actor");
    let receipt = store.rename_identity(IdentityKind::Agent, &RenameRequest {
        from: "agent/example-target/worker".into(), to: "agent/example-target-renamed/worker".into(),
        actor: "agent/example-actor-alias/worker".into(), reason: "Clarify responsibility".into(),
        expected_naming_revision: None, expected_declaration: None, idempotency_key: "rename-by-alias".into(),
    }, &[]).unwrap();
    let proof = store.export_rename_proof(IdentityKind::Agent, "agent/example-target-renamed/worker", None).unwrap();
    let claim = store.claim_by_id(&receipt.claim_id).unwrap().unwrap();
    assert_eq!(claim.actor.as_deref(), Some("agent/example-actor/worker"));
    assert_eq!(claim.body["fields"]["request"]["actor"], "agent/example-actor-alias/worker");
    drop(store);
    let verdict = verify_rename_proof(&proof, member.public(), None).unwrap();
    assert_eq!(verdict.actor, "agent/example-actor-alias/worker");
    assert_eq!(verdict.request_actor, "agent/example-actor-alias/worker");
}

#[test]
fn portable_proof_rejects_unknown_schema_and_forged_embedded_anchor() {
    let (proof, member) = exported();
    let mut changed = proof.clone();
    changed.schema = "fractal.rename-proof.unknown".into();
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "malformed-bundle");
    let mut changed = proof;
    changed.fleet_anchor = key().public().to_owned();
    assert_eq!(verify_rename_proof(&changed, member.public(), None).unwrap_err().code, "untrusted-membership");
}

#[test]
fn portable_proof_rejects_removed_anchor_admission_evidence() {
    let (mut proof, member) = exported();
    let admission_index = proof.envelopes.iter().position(|envelope| {
        let payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        payload.batch.claims.iter().any(|claim| {
            claim.kind == "fleet.member-admitted"
                && claim.body["fields"]["via"] == "anchor"
        })
    }).unwrap();
    proof.envelopes.remove(admission_index);
    // Without the founding admission, no member window can authenticate the writer.
    let error = verify_rename_proof(&proof, member.public(), None).unwrap_err();
    assert_eq!(error.code, "untrusted-membership");
    assert_eq!(error.exit_code(), 4);
}

#[test]
fn retained_member_removal_fences_a_validly_signed_later_rename() {
    let anchor_key = key();
    let writer_key = key();
    let anchor = node("example-anchor", Some(&anchor_key), Some(&anchor_key));
    anchor.admit_fleet_anchor(FLEET, anchor_key.public(), "listening").unwrap();
    admit(&anchor, "example-writer", &writer_key, "invite", None);
    let writer = node("example-writer", Some(&writer_key), Some(&anchor_key));
    sync(&anchor, &writer);
    writer.ensure_principal_key(ACTOR).unwrap();
    publish(&writer, "version 2\nagent \"example-original/worker\" { command \"true\"; }\n", "declare-removed-member-agent");
    rename(&writer, IdentityKind::Agent, ORIGINAL, RENAMED, "before-removal");
    sync(&writer, &anchor);
    let before = writer.export_rename_proof(IdentityKind::Agent, RENAMED, None).unwrap();
    let high_water = before.envelopes.iter()
        .filter(|envelope| envelope.writer == "example-writer")
        .map(|envelope| envelope.sequence).max().unwrap();

    // The writer is partitioned: it can still create a real signed rename, while the
    // anchor removes it at the last writer sequence the anchor actually observed.
    anchor.remove_fleet_member("example-writer", "End this member window", ACTOR).unwrap();
    rename(&writer, IdentityKind::Agent, RENAMED, FINAL, "partitioned-after-removal");
    let mut proof = writer.export_rename_proof(IdentityKind::Agent, FINAL, None).unwrap();
    let head_sequence = proof.envelopes.iter().filter_map(|envelope| {
        let payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        payload.batch.claims.iter().any(|claim| claim.id == proof.naming_revision)
            .then_some(envelope.sequence)
    }).next().unwrap();
    assert!(head_sequence > high_water);

    // Combine only genuine sealed envelopes: retain the partitioned writer's signed
    // suffix and replace the anchor prefix with its real removal-bearing history.
    let anchor_history = anchor.rename_proof_envelopes().unwrap();
    let removal = anchor_history.iter().flat_map(|envelope| {
        let payload: smallclaims::claim::ReplicaEnvelopePayload =
            ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
        payload.batch.claims
    }).find(|claim| claim.kind == "fleet.member-removed").unwrap();
    assert_eq!(removal.subject, "host/example-writer");
    assert_eq!(removal.body["fields"]["high_water"], high_water);
    proof.envelopes.retain(|envelope| envelope.writer != "example-anchor");
    proof.envelopes.extend(anchor_history.into_iter()
        .filter(|envelope| envelope.writer == "example-anchor"));
    let error = verify_rename_proof(&proof, anchor_key.public(), None).unwrap_err();
    assert_eq!(error.code, "untrusted-membership");
    assert_eq!(error.exit_code(), 4);
}

#[test]
fn quarantined_unsigned_fork_does_not_pollute_valid_rename_evidence() {
    let (store, member, _, _) = twice_renamed();
    // A genuine unsigned sender reuses the admitted writer's name. Transport retains
    // the fork, but the runtime must hold it outside the keyed member's history.
    let unsigned = node("example", None, Some(&member));
    crate::store::fleet_admission_tests_shared_seats::note(&unsigned, "unsigned unrelated receipt");
    let exchange = unsigned.export_replication_exchange_answering(
        FLEET, &store.replication_inventory().unwrap(), &[],
    ).unwrap();
    assert!(!exchange.envelopes.is_empty());
    let quarantined = exchange.envelopes.iter().map(|envelope| envelope.hash.clone())
        .collect::<BTreeSet<_>>();
    store.receive_replication_exchange("example", FLEET, &exchange).unwrap();
    let admission = store.validate_replication_backlog().unwrap();
    assert!(admission.held > 0);
    store.project_replication_backlog().unwrap();
    let proof = store.export_rename_proof(IdentityKind::Agent, FINAL, None).unwrap();
    assert!(proof.envelopes.iter().all(|envelope| !quarantined.contains(&envelope.hash)));
    verify_rename_proof(&proof, member.public(), None).unwrap();
}

#[test]
fn unrelated_admitted_diagnostic_writer_is_not_disclosed() {
    let (store, member, _, _) = twice_renamed();
    let unrelated_key = key();
    admit(&store, "example-diagnostics", &unrelated_key, "invite", None);
    let unrelated = node("example-diagnostics", Some(&unrelated_key), Some(&member));
    sync(&store, &unrelated);
    crate::store::fleet_admission_tests_shared_seats::note(&unrelated, "unrelated private diagnostic");
    sync(&unrelated, &store);
    assert!(store.replication_inventory().unwrap().envelopes.iter()
        .any(|envelope| envelope.writer == "example-diagnostics"));
    let proof = store.export_rename_proof(IdentityKind::Agent, FINAL, None).unwrap();
    assert!(proof.envelopes.iter().all(|envelope| envelope.writer != "example-diagnostics"));
    verify_rename_proof(&proof, member.public(), None).unwrap();
}

#[test]
fn real_disk_export_keyset_pages_release_readers_and_pin_the_inventory_horizon() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("proof.sqlite3");
    let member = key();
    let store = Store::open(&path, "example").unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(member.public()).unwrap();
    store.set_member_key(Some(member.clone())).unwrap();
    store.admit_fleet_anchor(FLEET, member.public(), "listening").unwrap();
    store.ensure_principal_key(ACTOR).unwrap();
    publish(&store, "version 2\nagent \"example-original/worker\" { command \"true\"; }\n", "paged-declaration");
    rename(&store, IdentityKind::Agent, ORIGINAL, RENAMED, "paged-rename");
    for index in 0..130 {
        crate::store::fleet_admission_tests_shared_seats::note(&store, &format!("retained diagnostic {index}"));
        store.seal_local_batches().unwrap();
    }
    let mut proof = store.export_rename_proof(IdentityKind::Agent, RENAMED, None).unwrap();
    let expected = proof.envelopes.iter().map(|envelope| envelope.hash.clone()).collect::<Vec<_>>();
    let checkpoint = Connection::open(&path).unwrap();
    checkpoint.busy_timeout(std::time::Duration::ZERO).unwrap();
    let mut pages = Vec::new();
    proof.envelopes = store.rename_proof_envelopes_with_page_observer(|count| {
        assert!((1..=64).contains(&count));
        pages.push(count);
        // Real runtime writes and sealing during every page expose a lingering read
        // transaction to TRUNCATE and ensure the scan does not chase arriving history.
        crate::store::fleet_admission_tests_shared_seats::note(&store, "arrived after proof horizon");
        store.seal_local_batches().unwrap();
        let busy: i64 = checkpoint.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        assert_eq!(busy, 0, "an export page retained a SQLite reader snapshot");
        Ok(())
    }).unwrap();
    assert!(pages.len() >= 9, "each of the three passes must traverse multiple bounded pages");
    assert_eq!(proof.envelopes.iter().map(|envelope| envelope.hash.clone()).collect::<Vec<_>>(), expected);
    verify_rename_proof(&proof, member.public(), None).unwrap();
}
