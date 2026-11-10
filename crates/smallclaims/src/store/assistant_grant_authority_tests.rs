//! Maintenance controls use real sealed envelopes; the reference reads raw history only.
use super::assistant_grant_authority::{self as authority, GRANTED, POLICY, REVOKED};
use super::runtime::Plain;
use super::*;
use crate::claim::ReplicaEnvelopeSignature;
use crate::fleet::MemberKey;
use crate::replication::ReplicationExchange;
use rusqlite::{StatementStatus, types::Value as SqlValue};
use std::time::Duration;

const FLEET: &str = "assistant-authority-maintenance";
const SUBJECT: &str = "custom/client/assistant-v1-0123456789abcdef01234567";
const OWNER: &str = "human/owner";

fn node(name: &str, anchor: Option<&str>) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    if let Some(anchor) = anchor {
        store.pin_fleet_anchor(anchor).unwrap();
    }
    store
}

fn source() -> (Store, Arc<MemberKey>) {
    let store = node("anchor", None);
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    store.seal_local_batches().unwrap();
    (store, key)
}

fn fields() -> Value {
    json!({
        "grant_id": "assistant-grant/0123456789abcdef01234567",
        "issuer": OWNER,
        "principal": "assistant/chatgpt",
        "profile": "assistant",
        "session_actor": "assistant/chatgpt/session/assistant-grant/0123456789abcdef01234567",
        "credential_hash": "a".repeat(64),
        "minimal_read_mode": "approval-once",
        "disclosure_consent": true,
        "expires_at_unix_ms": 10000,
        "scopes": ["read.assistant.summary"]
    })
}

fn service_fields() -> Value {
    // Main's Service fixtures use bare grant IDs and omit assistant consent/read mode.
    json!({
        "grant_id": "0123456789abcdef01234567",
        "issuer": OWNER,
        "principal": "plugin/test/reader",
        "profile": "service",
        "session_actor": "plugin/test/reader/session/0123456789abcdef01234567",
        "credential_hash": "b".repeat(64),
        "expires_at_unix_ms": 10000,
        "scopes": ["read.claims", "read.work", "read.documents"]
    })
}

#[test]
fn service_local_grant_authenticates_projection_immediately_without_assistant_consent() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, service_fields());
    let connection = store.readers.get();
    assert!(authority::contains(&connection, &grant.id).unwrap());
    assert_eq!(authority::by_credential(&connection, &"b".repeat(64)).unwrap().unwrap().id, grant.id);
    let candidates = authority::candidates(&connection, "plugin/test/reader", OWNER, Some(0), None, None).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, grant.id);
    let has_envelope: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1)",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    assert!(!has_envelope, "Service authority must not wait for sealing");
}

#[test]
fn service_projection_validators_enforce_closed_identity_policy_and_common_checks() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, service_fields());
    assert!(authority::structurally_valid(&grant, &grant));
    for (field, value) in [
        ("principal", json!("assistant/other")),
        ("principal", json!("plugin//reader")),
        ("principal", json!("plugin/test/reader/extra")),
        ("principal", json!("plugin/test/read er")),
        ("profile", json!("assistant")),
        ("profile", Value::Null),
        ("session_actor", json!("plugin/test/other/session/0123456789abcdef01234567")),
        ("issuer", json!("human/other")),
        ("grant_id", json!("bad")),
        ("credential_hash", json!("not-a-digest")),
        ("code_hash", json!("not-a-digest")),
        ("credential", json!("raw-secret")),
        ("code", json!("raw-code")),
        ("expires_at_unix_ms", json!(-1)),
        ("scopes", json!(["read.assistant.summary"])),
        ("scopes", json!(["write.claims"])),
        ("scopes", json!(["read.claims", 1])),
    ] {
        let mut malformed = grant.clone();
        malformed.body["fields"][field] = value;
        assert!(!authority::structurally_valid(&malformed, &malformed), "accepted malformed {field}");
    }
    for principal in ["plugin/issuer-name/reader_1.0", "plugin/Test/Reader"] {
        let mut valid = grant.clone();
        valid.body["fields"]["principal"] = json!(principal);
        valid.body["fields"]["session_actor"] = json!(format!("{principal}/session/0123456789abcdef01234567"));
        assert!(authority::structurally_valid(&valid, &valid));
    }
}

#[test]
fn service_late_signature_promotes_grant_policy_and_revocation() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, service_fields());
    let exchange = export(&source, &target);
    receive(&target, &unsigned(&exchange));
    assert!(authority::by_credential(&target.readers.get(), &"b".repeat(64)).unwrap().is_none());
    receive(&target, &signatures(&exchange));
    assert_eq!(authority::by_credential(&target.readers.get(), &"b".repeat(64)).unwrap().unwrap().id, grant.id);
    let mut changed = service_fields();
    changed["expires_at_unix_ms"] = json!(20000);
    changed["scopes"] = json!(["read.work"]);
    let policy = append(&source, SUBJECT, POLICY, changed);
    receive(&target, &export(&source, &target));
    assert_eq!(authority::transition(&target.readers.get(), &grant, POLICY).unwrap().unwrap().id, policy.id);
    let revoke = append(&source, SUBJECT, REVOKED, service_fields());
    let exchange = export(&source, &target);
    receive(&target, &unsigned(&exchange));
    assert!(authority::transition(&target.readers.get(), &grant, REVOKED).is_err());
    receive(&target, &signatures(&exchange));
    assert_eq!(authority::transition(&target.readers.get(), &grant, REVOKED).unwrap().unwrap().id, revoke.id);
    assert!(authority::candidates(&target.readers.get(), "plugin/test/reader", OWNER, Some(0), None, None).unwrap().is_empty());
}

#[test]
fn service_signed_and_local_unsealed_grants_survive_projection_backfill() {
    let (store, _) = source();
    let sealed = append(&store, SUBJECT, GRANTED, service_fields());
    store.seal_local_batches().unwrap();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-ha76427048a9d owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_local_appends WHERE claim_id=?1", [&sealed.id],
    ).unwrap();
    let mut changed = service_fields();
    changed["credential_hash"] = json!("c".repeat(64));
    let unsealed = append(&store, SUBJECT, GRANTED, changed);
    reset_projection(&store.connection.write());
    drain_authority(&store);
    let connection = store.readers.get();
    assert!(authority::contains(&connection, &sealed.id).unwrap());
    assert!(authority::contains(&connection, &unsealed.id).unwrap());
    assert_eq!(authority::by_credential(&connection, &"b".repeat(64)).unwrap().unwrap().id, sealed.id);
    assert_eq!(authority::by_credential(&connection, &"c".repeat(64)).unwrap().unwrap().id, unsealed.id);
}

#[test]
fn legacy_service_on_assistant_remains_cross_profile_conflict_evidence() {
    let store = node("issuer", None);
    let assistant = append(&store, SUBJECT, GRANTED, fields());
    let mut legacy = fields();
    legacy["profile"] = json!("service");
    legacy["grant_id"] = json!("assistant-grant/fedcba9876543210fedcba98");
    legacy["session_actor"] = json!("assistant/chatgpt/session/assistant-grant/fedcba9876543210fedcba98");
    let service = append(&store, "custom/client/assistant-v1-fedcba9876543210fedcba98", GRANTED, legacy);
    assert!(authority::structurally_valid(&service, &service));
    let connection = store.readers.get();
    let candidates = authority::candidates(&connection, "assistant/chatgpt", OWNER, Some(0), None, None).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|claim| claim.id == assistant.id));
    assert!(candidates.iter().any(|claim| claim.id == service.id));
    let mut invalid = service.clone();
    invalid.body["fields"]["scopes"] = json!(["read.claims"]);
    assert!(!authority::structurally_valid(&invalid, &invalid));
    invalid = service.clone();
    invalid.body["fields"]["disclosure_consent"] = Value::Null;
    assert!(!authority::structurally_valid(&invalid, &invalid));
    invalid = service.clone();
    invalid.body["fields"]["minimal_read_mode"] = Value::Null;
    assert!(!authority::structurally_valid(&invalid, &invalid));
}

fn append(store: &Store, subject: &str, kind: &str, fields: Value) -> ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some(OWNER.into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap()
}

fn export(source: &Store, target: &Store) -> ReplicationExchange {
    source.seal_local_batches().unwrap();
    source.export_replication_exchange(FLEET, &target.replication_inventory().unwrap()).unwrap()
}

fn receive(target: &Store, exchange: &ReplicationExchange) {
    target.receive_replication_exchange(&exchange.peer, FLEET, exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    target.project_replication_backlog().unwrap();
    drain_authority(target);
}

fn unsigned(exchange: &ReplicationExchange) -> ReplicationExchange {
    let mut result = exchange.clone();
    result.signatures.clear();
    for envelope in &mut result.envelopes {
        envelope.member_key = None;
        envelope.signature = None;
    }
    result
}

fn signatures(exchange: &ReplicationExchange) -> ReplicationExchange {
    let mut result = exchange.clone();
    result.signatures.extend(result.envelopes.iter().filter_map(|envelope| {
        Some(ReplicaEnvelopeSignature {
            writer: envelope.writer.clone(), sequence: envelope.sequence,
            hash: envelope.hash.clone(), member_key: envelope.member_key.clone()?,
            signature: envelope.signature.clone()?,
        })
    }));
    result.envelopes.clear();
    result
}

fn snapshot(connection: &Connection) -> Vec<Vec<SqlValue>> {
    let mut statement = connection.prepare(
        "SELECT claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record
         FROM assistant_grant_effective ORDER BY claim_id"
    ).unwrap();
    statement.query_map([], |row| (0..8).map(|index| row.get(index)).collect())
        .unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn raw_claims(store: &Store) -> Vec<ClaimRecord> {
    store.readers.get().prepare(/* sqlite-large-value: allow-for-now lv-hef28dd74e68f owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT {CLAIM_COLUMNS} FROM claims ORDER BY store_index"))
        .unwrap().query_map([], claim_from_row).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap()
}

// This intentionally knows the fixture's wire contract, not the projection's validator,
// pending table, or authority rows. Authentication goes through the public raw-provenance API.
fn reference(store: &Store) -> Vec<(String, String, bool)> {
    let claims = raw_claims(store);
    let trusted = |claim: &ClaimRecord| {
        claim.origin == store.origin
            || store.claim_has_authenticated_member_writer(&claim.id).unwrap()
    };
    let identity = |claim: &ClaimRecord| {
        let f = &claim.body["fields"];
        claim.subject == SUBJECT && claim.actor.as_deref() == Some(OWNER)
            && f["issuer"] == OWNER && f["grant_id"] == fields()["grant_id"]
            && f["principal"] == "assistant/chatgpt" && f["profile"] == "assistant"
    };
    let policy = |claim: &ClaimRecord| {
        let f = &claim.body["fields"];
        identity(claim) && f["session_actor"] == fields()["session_actor"]
            && f["minimal_read_mode"] == "approval-once" && f["disclosure_consent"] == true
            && f["expires_at_unix_ms"].as_u64().is_some()
            && f["scopes"] == json!(["read.assistant.summary"])
            && f.get("credential").is_none() && f.get("code").is_none()
            && f["credential_hash"].as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let mut expected = Vec::new();
    let base = |claim: &ClaimRecord| {
        let f = &claim.body["fields"];
        identity(claim) && f.get("credential").is_none() && f.get("code").is_none()
            && f["credential_hash"].as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    for grant in claims.iter().filter(|claim| claim.kind == GRANTED && trusted(claim) && base(claim)) {
        let selected = claims.iter().rev().find(|claim| {
            claim.kind == POLICY && claim.origin == grant.origin && trusted(claim) && policy(claim)
        }).unwrap_or(grant);
        if !policy(selected) { continue; }
        let revoked = claims.iter().any(|claim| {
            claim.kind == REVOKED && claim.origin == grant.origin && trusted(claim) && identity(claim)
        });
        expected.push((grant.id.clone(), format!("{:020}", selected.body["fields"]["expires_at_unix_ms"].as_u64().unwrap()), revoked));
    }
    expected.sort();
    expected
}

fn assert_reference_and_rebuild(store: &Store) {
    let expected = reference(store);
    let mut connection = store.connection.write();
    let actual: Vec<(String, String, bool)> = connection.prepare(
        "SELECT claim_id,expires_at,revoked FROM assistant_grant_effective ORDER BY claim_id"
    ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(actual, expected, "incremental rows disagree with independent raw history");
    let before = snapshot(&connection);
    let transaction = connection.transaction().unwrap();
    let subjects: Vec<String> = transaction.prepare(
        "SELECT DISTINCT grant_subject FROM assistant_grant_authority UNION SELECT grant_subject FROM assistant_grant_effective"
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    for subject in subjects {
        authority::rebuild_subject(&transaction, &subject).unwrap();
    }
    assert_eq!(snapshot(&transaction), before, "full subject rebuild changed incremental state");
    transaction.commit().unwrap();
}

fn decisions(store: &Store, now: u64) -> Vec<String> {
    authority::candidates(&store.readers.get(), "assistant/chatgpt", OWNER, Some(now), None, None)
        .unwrap().into_iter()
        .filter(|claim| authority::contains(&store.readers.get(), &claim.id).unwrap())
        .map(|claim| claim.id).collect()
}

#[test]
fn malformed_base_cannot_mask_a_valid_credential_but_valid_policy_can_repair_it() {
    let store = node("issuer", None);
    let granted = append(&store, SUBJECT, GRANTED, fields());
    let digest = fields()["credential_hash"].as_str().unwrap().to_owned();
    let mut malformed = fields();
    malformed["expires_at_unix_ms"] = Value::Null;
    let pending = append(&store, SUBJECT, GRANTED, malformed);
    assert!(!authority::contains(&store.readers.get(), &pending.id).unwrap());
    assert_eq!(authority::by_credential(&store.readers.get(), &digest).unwrap().unwrap().id, granted.id);
    assert_eq!(authority::latest(&store.readers.get(), SUBJECT, GRANTED).unwrap().unwrap().id, granted.id);
    let mut renewal = fields();
    renewal["expires_at_unix_ms"] = json!(20000);
    append(&store, SUBJECT, POLICY, renewal);
    assert!(authority::contains(&store.readers.get(), &pending.id).unwrap());
    assert_eq!(authority::by_credential(&store.readers.get(), &digest).unwrap().unwrap().id, pending.id);
    assert_reference_and_rebuild(&store);
}

#[test]
fn late_envelope_signature_promotes_previously_admitted_grant() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    let exchange = export(&source, &target);
    receive(&target, &unsigned(&exchange));
    assert!(raw_claims(&target).iter().any(|claim| claim.id == grant.id));
    assert!(decisions(&target, 0).is_empty());
    assert_reference_and_rebuild(&target);
    receive(&target, &signatures(&exchange));
    assert_eq!(decisions(&target, 0), vec![grant.id]);
    assert_reference_and_rebuild(&target);
    receive(&target, &signatures(&exchange));
    assert_reference_and_rebuild(&target);
}

fn backfill_markers(connection: &Connection) -> Vec<(String, String)> {
    connection.prepare(
        "SELECT key,value FROM meta WHERE key='assistant_grant_authority_version'
            OR key LIKE 'assistant_grant_authority_cursor/%' ORDER BY key"
    ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

fn insert_raw_signature(connection: &Connection, signature: &ReplicaEnvelopeSignature) {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hdd61bb7dd3ca owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO replica_envelope_signatures
            (writer,sequence,envelope_hash,member_key,signature,stored_at_unix_ms)
         VALUES(?1,?2,?3,?4,?5,'0')",
        params![signature.writer,signature.sequence,signature.hash,signature.member_key,signature.signature],
    ).unwrap();
}

#[test]
fn raw_signature_only_insert_survives_reopen_and_promotes_grant_policy_and_revoke_without_rescan() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("late-signature.sqlite3");
    let (source, key) = source();
    let mut target = Store::open(&path, "reader", Arc::new(Plain)).unwrap();
    target.bind_fleet(FLEET).unwrap();
    target.pin_fleet_anchor(key.public()).unwrap();
    // Establish membership first: no membership change may drive these promotions.
    receive(&target, &export(&source, &target));
    let mut grant_id = String::new();
    for kind in [GRANTED, POLICY, REVOKED] {
        let mut value = fields();
        if kind == POLICY { value["expires_at_unix_ms"] = json!(20000); }
        let claim = append(&source, SUBJECT, kind, value);
        if kind == GRANTED { grant_id.clone_from(&claim.id); }
        let exchange = export(&source, &target);
        receive_pre_projection_unsigned_history(&target, &exchange);
        assert!(raw_claims(&target).iter().any(|record| record.id == claim.id));
        assert!(!authority::contains(&target.readers.get(), &claim.id).unwrap());
        while !target.advance_assistant_grant_authority_backfill().unwrap() {}
        let markers = backfill_markers(&target.readers.get());
        authority::ready(&target.readers.get()).unwrap();
        drop(target);

        // Simulate an older writer, bypassing every Store admission/refresh hook.
        {
            let connection = Connection::open(&path).unwrap();
            for signature in signatures(&exchange).signatures {
                insert_raw_signature(&connection, &signature);
            }
            let dirty: Vec<String> = connection.prepare(
                "SELECT claim_id FROM assistant_grant_dirty ORDER BY claim_id"
            ).unwrap().query_map([], |row| row.get(0)).unwrap()
                .collect::<rusqlite::Result<_>>().unwrap();
            assert_eq!(dirty, vec![claim.id.clone()]);
            authority::ready(&connection).unwrap();
            assert!(authority::maintenance_pending(&connection).unwrap());
            assert!(authority::subject_ready(&connection, SUBJECT).is_err());
            assert_eq!(backfill_markers(&connection), markers);
        }
        // Reopen before Store startup can drain the durable queue automatically.
        {
            let mut connection = Connection::open(&path).unwrap();
            authority::ready(&connection).unwrap();
            assert!(authority::maintenance_pending(&connection).unwrap());
            let transaction = connection.transaction().unwrap();
            assert!(authority::backfill_slice(&transaction, Duration::from_secs(60)).unwrap());
            authority::ready(&transaction).unwrap();
            assert!(authority::contains(&transaction, &claim.id).unwrap());
            assert_eq!(backfill_markers(&transaction), markers, "completed history or membership was rescanned");
            transaction.commit().unwrap();
        }
        target = Store::open(&path, "reader", Arc::new(Plain)).unwrap();
        assert_reference_and_rebuild(&target);
        if kind == REVOKED {
            assert!(decisions(&target, 0).is_empty());
            let revoked: bool = target.readers.get().query_row(
                "SELECT revoked FROM assistant_grant_effective WHERE claim_id=?1",
                [&grant_id], |row| row.get(0),
            ).unwrap();
            assert!(revoked);
        } else {
            assert_eq!(decisions(&target, 0), vec![grant_id.clone()]);
            assert_eq!(decisions(&target, 15000).is_empty(), kind == GRANTED);
        }
    }
}

#[test]
fn malformed_raw_signature_row_is_drained_without_authenticating_a_pending_revoke() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    receive(&target, &export(&source, &target));
    let revoke = append(&source, SUBJECT, REVOKED, fields());
    let exchange = export(&source, &target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    while !target.advance_assistant_grant_authority_backfill().unwrap() {}
    let markers = backfill_markers(&target.readers.get());
    {
        let mut connection = target.connection.write();
        for mut signature in signatures(&exchange).signatures {
            signature.signature = "not-an-ed25519-signature".into();
            insert_raw_signature(&connection, &signature);
        }
        authority::ready(&connection).unwrap();
        assert!(authority::subject_ready(&connection, SUBJECT).is_err());
        let transaction = connection.transaction().unwrap();
        assert!(authority::backfill_slice(&transaction, Duration::from_secs(60)).unwrap());
        authority::ready(&transaction).unwrap();
        assert!(!authority::contains(&transaction, &revoke.id).unwrap());
        let authenticated: bool = transaction.query_row(
            "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
            [&revoke.id], |row| row.get(0),
        ).unwrap();
        assert!(!authenticated);
        assert_eq!(backfill_markers(&transaction), markers);
        transaction.commit().unwrap();
    }
    assert!(decisions(&target, 0).is_empty());
    assert_bearer_rejected(&target, &grant);
    assert_reference_and_rebuild(&target);
}

#[test]
fn membership_sealed_after_grant_admission_promotes_pending_authority() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    let exchange = export(&source, &target);
    let mut delayed_anchor_signature = exchange.clone();
    let anchor_sequence = exchange.envelopes.iter().map(|e| e.sequence).min().unwrap();
    for envelope in &mut delayed_anchor_signature.envelopes {
        if envelope.sequence == anchor_sequence {
            envelope.member_key = None;
            envelope.signature = None;
        }
    }
    delayed_anchor_signature.signatures.retain(|s| s.sequence != anchor_sequence);
    receive(&target, &delayed_anchor_signature);
    assert!(raw_claims(&target).iter().any(|claim| claim.id == grant.id));
    assert!(decisions(&target, 0).is_empty());
    assert_reference_and_rebuild(&target);
    let mut seal = signatures(&exchange);
    seal.signatures.retain(|s| s.sequence == anchor_sequence);
    receive(&target, &seal);
    assert!(target.claim_has_authenticated_member_writer(&grant.id).unwrap());
    assert_eq!(decisions(&target, 0), vec![grant.id]);
    assert_reference_and_rebuild(&target);
}

#[test]
fn sealing_local_membership_promotes_an_already_received_peer_grant() {
    let target = node("anchor", None);
    let anchor_key = Arc::new(MemberKey::generate().unwrap().0);
    target.pin_fleet_anchor(anchor_key.public()).unwrap();
    target.set_member_key(Some(anchor_key.clone())).unwrap();
    target.admit_fleet_anchor(FLEET, anchor_key.public(), "listening").unwrap();
    target.seal_local_batches().unwrap();

    let peer = node("peer", Some(anchor_key.public()));
    let peer_key = Arc::new(MemberKey::generate().unwrap().0);
    peer.set_member_key(Some(peer_key.clone())).unwrap();
    let grant = append(&peer, SUBJECT, GRANTED, fields());
    receive(&target, &export(&peer, &target));
    assert!(raw_claims(&target).iter().any(|claim| claim.id == grant.id));
    assert!(decisions(&target, 0).is_empty());
    assert_reference_and_rebuild(&target);

    append(&target, "host/peer", "fleet.member-admitted", json!({
        "member_key": peer_key.public(), "via": "member", "sponsor": "anchor",
        "writer_floor": 0, "mode": "listening"
    }));
    // The membership claim exists but does not yet have a sealed signer.
    assert!(!target.claim_has_authenticated_member_writer(&grant.id).unwrap());
    assert!(decisions(&target, 0).is_empty());
    target.seal_local_batches().unwrap();
    drain_authority(&target);
    assert!(target.claim_has_authenticated_member_writer(&grant.id).unwrap());
    assert_eq!(decisions(&target, 0), vec![grant.id]);
    assert_reference_and_rebuild(&target);
}

#[test]
fn revoke_before_grant_never_temporarily_authorizes() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    receive(&target, &export(&source, &target));
    append(&source, SUBJECT, REVOKED, fields());
    receive(&target, &export(&source, &target));
    assert!(decisions(&target, 0).is_empty());
    assert_reference_and_rebuild(&target);
    let grant = append(&source, SUBJECT, GRANTED, fields());
    receive(&target, &export(&source, &target));
    assert!(authority::contains(&target.readers.get(), &grant.id).unwrap());
    assert!(decisions(&target, 0).is_empty());
    assert_reference_and_rebuild(&target);
}

#[test]
fn deterministic_event_permutations_match_raw_history_and_subject_rebuilds() {
    // Each seed shuffles raw grant/policy/revoke receipt, their signatures, and the
    // anchor signature that seals membership. Signatures may arrive before records.
    for seed in 1_u64..=16 {
        let (source, key) = source();
        let target = node("reader", Some(key.public()));
        let grant = append(&source, SUBJECT, GRANTED, fields());
        source.seal_local_batches().unwrap();
        let mut changed = fields();
        changed["expires_at_unix_ms"] = json!(20000);
        append(&source, SUBJECT, POLICY, changed);
        source.seal_local_batches().unwrap();
        append(&source, SUBJECT, REVOKED, fields());
        let exchange = export(&source, &target);
        assert_eq!(exchange.envelopes.len(), 4);
        let mut events = Vec::new();
        for envelope in &exchange.envelopes {
            let mut single = exchange.clone();
            single.envelopes = vec![envelope.clone()];
            single.signatures.clear();
            events.push(unsigned(&single));
            events.push(signatures(&single));
        }
        let mut state = seed;
        for index in (1..events.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let selected = (state % (index as u64 + 1)) as usize;
            events.swap(index, selected);
        }
        for event in events {
            receive(&target, &event);
            assert_reference_and_rebuild(&target);
        }
        // Drain predecessors that an out-of-order envelope initially held.
        target.validate_replication_backlog().unwrap();
        target.project_replication_backlog().unwrap();
        drain_authority(&target);
        assert_reference_and_rebuild(&target);
        assert!(authority::contains(&target.readers.get(), &grant.id).unwrap());
        assert_eq!(reference(&target), vec![(grant.id, format!("{:020}", 20000), true)]);
        assert!(decisions(&target, 0).is_empty());
    }
}

fn reset_projection(connection: &Connection) {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-ha93665c85209 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM assistant_grant_effective;
         DELETE FROM assistant_grant_authority;
         DELETE FROM assistant_grant_pending;
         DELETE FROM assistant_grant_membership_windows;
         DELETE FROM assistant_grant_dirty;
         DELETE FROM assistant_grant_auth_cursor;
         DELETE FROM assistant_grant_additions;
         DELETE FROM assistant_grant_retractions;
         DELETE FROM assistant_grant_recompute;
         DELETE FROM assistant_grant_proof_invalidations;
         DELETE FROM assistant_grant_proof_rechecks;
         UPDATE assistant_grant_membership_state SET applied_generation=-1 WHERE id=1;
         UPDATE meta SET value=value WHERE key='fleet_anchor_key';
         DELETE FROM meta WHERE key='assistant_grant_authority_version'
            OR key LIKE 'assistant_grant_authority_cursor/%';"
    ).unwrap();
}

fn drain_authority(store: &Store) {
    while !store.advance_assistant_grant_authority_backfill().unwrap() {}
}

// Build the raw, unsigned keyed-writer history an older importer could leave.
// Today's admission correctly holds unsigned keyed envelopes before raw claims
// exist, so receive(unsigned(exchange)) cannot exercise projection authentication.
// Use the real signed transport to materialize batches/receipt records, then
// discard this exchange's proofs and rebuild the projection from the remaining
// evidence before making any assertion. Existing earlier signatures survive.
fn receive_pre_projection_unsigned_history(target: &Store, exchange: &ReplicationExchange) {
    receive(target, exchange);
    {
        let mut connection = target.connection.write();
        let transaction = connection.transaction().unwrap();
        for signature in signatures(exchange).signatures {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h9b903d60d4d6 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
                params![signature.writer, signature.sequence, signature.hash, signature.member_key],
            ).unwrap();
        }
        reset_projection(&transaction);
        transaction.commit().unwrap();
    }
    drain_authority(target);
}

fn assert_bearer_rejected(store: &Store, grant: &ClaimRecord) {
    let digest = grant.body["fields"]["credential_hash"].as_str().unwrap();
    assert!(!authority::contains(&store.readers.get(), &grant.id).unwrap());
    assert!(authority::by_credential(&store.readers.get(), digest).unwrap().is_none());
    assert!(!decisions(store, 0).contains(&grant.id));
}

fn attacker_fork() -> (Store, ClaimRecord) {
    // An admitted Legacy sender invents a fork of the victim's writer history.
    // No member key or signature authenticates the claimed local origin.
    let sender = node("victim", None);
    let mut forged = fields();
    forged["credential_hash"] = json!("b".repeat(64));
    let grant = append(&sender, SUBJECT, GRANTED, forged);
    (sender, grant)
}

#[test]
fn unsigned_victim_origin_legacy_fork_is_rejected_on_receive_and_upgrade_backfill() {
    let (sender, grant) = attacker_fork();
    let victim = node("victim", None);
    receive(&victim, &unsigned(&export(&sender, &victim)));
    assert!(victim.claim_by_id(&grant.id).unwrap().is_some(), "Legacy evidence was admitted");
    assert_bearer_rejected(&victim, &grant);
    reset_projection(&victim.connection.write());
    drain_authority(&victim);
    assert_bearer_rejected(&victim, &grant);
}

#[test]
fn victim_origin_legacy_fork_injected_after_partial_cursor_cannot_authenticate() {
    let victim = node("victim", None);
    let local = append(&victim, SUBJECT, GRANTED, fields());
    {
        let mut connection = victim.connection.write();
        reset_projection(&connection);
        let transaction = connection.transaction().unwrap();
        assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
        transaction.commit().unwrap();
    }
    let (sender, grant) = attacker_fork();
    receive(&victim, &unsigned(&export(&sender, &victim)));
    drain_authority(&victim);
    assert_bearer_rejected(&victim, &grant);
    assert_bearer_rejected(&victim, &local);
}

#[test]
fn older_build_raw_legacy_replication_cannot_authenticate_on_new_build_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("victim.sqlite3");
    let (sender, grant) = attacker_fork();
    {
        let victim = Store::open(&path, "victim", Arc::new(Plain)).unwrap();
        drain_authority(&victim);
        // The pre-envelope import path wrote raw batches and claims without
        // receipt records. Reproduce that older-build transaction, deliberately
        // bypassing all new admission hooks and the local append marker.
        let (sequence, previous, hash): (u64, Option<String>, String) = sender.readers.get()
            .query_row("SELECT replica_sequence,previous_hash,hash FROM batches WHERE id=?1",
                [&grant.batch_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        let mut connection = victim.connection.write();
        let transaction = connection.transaction().unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-hebabcaf7fd3f owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![grant.batch_id,grant.origin,sequence,previous,hash,grant.accepted_at_unix_ms.to_string()],
        ).unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-he87b7a8b6b00 owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![grant.id,grant.batch_id,grant.subject,grant.kind,grant.origin,grant.actor,
                canonical_json_text(&grant.body).unwrap(),serde_json::to_string(&grant.predecessors).unwrap(),
                grant.accepted_at_unix_ms.to_string()],
        ).unwrap();
        transaction.commit().unwrap();
        authority::ready(&connection).unwrap();
        assert!(authority::maintenance_pending(&connection).unwrap());
    }
    let reopened = Store::open(&path, "victim", Arc::new(Plain)).unwrap();
    drain_authority(&reopened);
    assert_bearer_rejected(&reopened, &grant);
}

#[test]
fn unsigned_remote_without_membership_drains_once_and_maintenance_goes_idle() {
    let sender = node("unsigned-remote", None);
    let grant = append(&sender, SUBJECT, GRANTED, fields());
    let target = node("reader", None);
    drain_authority(&target);
    {
        let (sequence, previous, hash): (u64, Option<String>, String) = sender.readers.get()
            .query_row("SELECT replica_sequence,previous_hash,hash FROM batches WHERE id=?1",
                [&grant.batch_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        let mut connection = target.connection.write();
        let transaction = connection.transaction().unwrap();
        // Reproduce a pre-envelope remote import: no local append marker,
        // replica receipt, signature, or membership window can authenticate it.
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h99e2f6069dc7 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,previous_hash,hash,accepted_at_unix_ms)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![grant.batch_id,grant.origin,sequence,previous,hash,grant.accepted_at_unix_ms.to_string()],
        ).unwrap();
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h4d9aa050b4a3 owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![grant.id,grant.batch_id,grant.subject,grant.kind,grant.origin,grant.actor,
                canonical_json_text(&grant.body).unwrap(),serde_json::to_string(&grant.predecessors).unwrap(),
                grant.accepted_at_unix_ms.to_string()],
        ).unwrap();
        assert!(authority::maintenance_pending(&transaction).unwrap());
        assert_eq!(transaction.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get::<_,u64>(0)).unwrap(), 1);
        transaction.commit().unwrap();
    }
    assert!(target.advance_assistant_grant_authority_backfill().unwrap());
    {
        let connection = target.readers.get();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get::<_,u64>(0)).unwrap(), 0);
        assert!(!connection.query_row(
            "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
            [&grant.id], |row| row.get::<_,bool>(0)).unwrap());
        assert!(!authority::maintenance_pending(&connection).unwrap());
    }
    assert!(!target.assistant_grant_authority_maintenance_pending().unwrap());
    let writes = target.connection.write().total_changes();
    for _ in 0..3 {
        assert!(target.advance_assistant_grant_authority_backfill().unwrap());
        assert!(!target.assistant_grant_authority_maintenance_pending().unwrap());
    }
    assert_eq!(target.connection.write().total_changes(), writes, "idle maintenance must not requeue or refresh unsigned evidence");
    assert_bearer_rejected(&target, &grant);
}

#[test]
fn local_append_authenticates_immediately_and_local_unsealed_and_signed_history_survive_backfill() {
    let (store, _) = source();
    let sealed = append(&store, SUBJECT, GRANTED, fields());
    store.seal_local_batches().unwrap();
    // A pre-marker sealed claim must stand on its real signature, not its
    // origin. A post-upgrade unsealed claim stands on durable local admission.
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-he990aaffd4df owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_local_appends WHERE claim_id=?1", [&sealed.id],
    ).unwrap();
    let mut changed = fields();
    changed["credential_hash"] = json!("c".repeat(64));
    let unsealed = append(&store, SUBJECT, GRANTED, changed);
    assert!(authority::by_credential(&store.readers.get(), &"c".repeat(64)).unwrap().is_some());
    let has_envelope: bool = store.readers.get().query_row(
        "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1)",
        [&unsealed.id], |row| row.get(0),
    ).unwrap();
    assert!(!has_envelope, "local authority has no signature/sealing delay");
    reset_projection(&store.connection.write());
    drain_authority(&store);
    assert!(authority::contains(&store.readers.get(), &sealed.id).unwrap());
    assert!(authority::contains(&store.readers.get(), &unsealed.id).unwrap());
}

#[test]
fn ambiguous_pre_marker_local_unsealed_history_fails_closed() {
    let store = node("victim", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h2dd1bfa09a04 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_local_appends", []).unwrap();
    reset_projection(&store.connection.write());
    drain_authority(&store);
    assert_bearer_rejected(&store, &grant);
}

#[test]
fn committed_backfill_slice_resumes_after_reopen_and_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.sqlite3");
    let expected;
    {
        let store = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
        for _ in 0..70 {
            append(&store, SUBJECT, GRANTED, fields());
        }
        let mut changed = fields();
        changed["expires_at_unix_ms"] = json!(20000);
        append(&store, SUBJECT, POLICY, changed);
        append(&store, SUBJECT, REVOKED, fields());
        let mut connection = store.connection.write();
        expected = snapshot(&connection);
        assert_eq!(expected.len(), 70);
        reset_projection(&connection);
        let transaction = connection.transaction().unwrap();
        assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
        let cursor: String = transaction.query_row(
            "SELECT value FROM meta WHERE key=?1", [format!("assistant_grant_authority_cursor/{GRANTED}")], |row| row.get(0)
        ).unwrap();
        assert!(cursor.parse::<u64>().unwrap() > 0);
        assert!(authority::ready(&transaction).is_err());
        transaction.commit().unwrap();
    }
    // Reopen the durable connection before Store's startup continuation can finish it.
    {
        let mut connection = Connection::open(&path).unwrap();
        assert!(authority::ready(&connection).is_err());
        let remaining: u64 = connection.query_row("SELECT COUNT(*) FROM assistant_grant_authority", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, 1);
        let mut slices = 0;
        loop {
            let transaction = connection.transaction().unwrap();
            let complete = authority::backfill_slice(&transaction, Duration::ZERO).unwrap();
            transaction.commit().unwrap();
            slices += 1;
            if complete { break; }
            assert!(slices < 200, "backfill failed to make durable forward progress");
        }
        assert!(slices > 1);
        assert_eq!(snapshot(&connection), expected);
        let transaction = connection.transaction().unwrap();
        assert!(authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
        assert_eq!(snapshot(&transaction), expected);
        transaction.commit().unwrap();
    }
    let reopened = Store::open(&path, "anchor", Arc::new(Plain)).unwrap();
    assert_eq!(snapshot(&reopened.readers.get()), expected);
    assert_reference_and_rebuild(&reopened);
    assert!(decisions(&reopened, 0).is_empty());
}

fn transition_cost(store: &Store, grant: &ClaimRecord) -> (String, i32, i32, usize) {
    let connection = store.readers.get();
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h611b5b2ef9a9 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ authority::TRANSITION_QUERY).unwrap();
    let mut rows = statement.query(params![grant.subject, POLICY, grant.origin, grant.actor, grant.id]).unwrap();
    let mut records = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        records.push(row.get::<_, String>(0).unwrap());
    }
    drop(rows);
    let record: ClaimRecord = serde_json::from_str(&records[0]).unwrap();
    (record.id, statement.get_status(StatementStatus::VmStep), statement.get_status(StatementStatus::FullscanStep), records.len())
}

fn candidate_cost(store: &Store) -> (Vec<String>, i32, i32) {
    let connection = store.readers.get();
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-he990dace4c7d owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ authority::CANDIDATES_QUERY).unwrap();
    let mut rows = statement.query(params![
        "assistant/chatgpt", OWNER, format!("{:020}", 15000),
        Option::<String>::None, Option::<String>::None
    ]).unwrap();
    let mut ids = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        let record: String = row.get(0).unwrap();
        ids.push(serde_json::from_str::<ClaimRecord>(&record).unwrap().id);
    }
    drop(rows);
    (ids, statement.get_status(StatementStatus::VmStep), statement.get_status(StatementStatus::FullscanStep))
}

#[test]
fn malformed_and_unsigned_same_origin_flood_preserves_previous_authority_and_constant_reads() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    receive(&target, &export(&source, &target));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    let mut valid = fields();
    valid["expires_at_unix_ms"] = json!(20000);
    let policy = append(&source, SUBJECT, POLICY, valid);
    receive(&target, &export(&source, &target));
    let before = transition_cost(&target, &grant);
    assert_eq!(before.0, policy.id);
    assert_eq!(before.2, 0);
    assert_eq!(before.3, 1);
    let candidate_before = candidate_cost(&target);
    assert_eq!(candidate_before.0, vec![grant.id.clone()]);
    assert_eq!(candidate_before.2, 0);
    for count in [1, 128] {
        for index in 0..count {
            let mut malformed = fields();
            if index % 2 == 0 { malformed["issuer"] = json!("human/not-owner"); }
            else { malformed["scopes"] = json!(["read.everything"]); }
            append(&source, SUBJECT, POLICY, malformed);
        }
        // Malformed rows are genuinely signed by the correct same-origin member.
        let mut malformed_rows = export(&source, &target);
        // Do not accidentally deliver requested signatures for the earlier unsigned flood.
        malformed_rows.signatures.clear();
        receive(&target, &malformed_rows);
        for _ in 0..count {
            let mut unauthenticated = fields();
            unauthenticated["expires_at_unix_ms"] = json!(30000);
            append(&source, SUBJECT, POLICY, unauthenticated);
            append(&source, SUBJECT, REVOKED, fields());
        }
        let unsigned_rows = export(&source, &target);
        receive_pre_projection_unsigned_history(&target, &unsigned_rows);
        let after = transition_cost(&target, &grant);
        assert_eq!(after, before, "same-origin flood changed selected authority or request VM/read cost at {count}");
        assert_eq!(candidate_cost(&target), candidate_before, "candidate request cost grew at {count}");
        assert!(decisions(&target, 15000).is_empty(), "pending revokes fence only this subject");
        assert_bearer_rejected(&target, &grant);
        assert_reference_and_rebuild(&target);
    }
}

#[test]
fn duplicate_verified_signature_does_not_refresh_or_dirty_claims() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    let exchange = export(&source, &target);
    receive(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    let mut connection = target.connection.write();
    // Any refresh would delete this sentinel. Abort that attempted write rather
    // than merely checking the final projection, which might be unchanged.
    connection.execute(/* sqlite-large-value: allow-for-now lv-hbf1fdaf2776a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9c6f8e69acbb owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER reject_duplicate_refresh BEFORE DELETE ON assistant_grant_dirty
         BEGIN SELECT RAISE(ABORT,'duplicate refreshed authority'); END;",
    ).unwrap();
    let transaction = connection.transaction().unwrap();
    assert_eq!(store_envelope_signature_tx(
        &transaction,None,FLEET,&signature.writer,signature.sequence,&signature.hash,
        &signature.member_key,&signature.signature,"1",
    ).unwrap(), 0);
    let dirty: usize = transaction.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get(0)).unwrap();
    assert_eq!(dirty,1);
    transaction.commit().unwrap();
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h68996f133d87 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER reject_duplicate_refresh; DELETE FROM assistant_grant_dirty;").unwrap();
    drop(connection);
    assert_reference_and_rebuild(&target);
}

#[test]
fn dirty_claim_flood_reserves_half_the_slice_for_other_maintenance() {
    let store = node("issuer", None);
    for index in 0..(authority::CLAIMS_PER_SLICE * 3 + 7) {
        let mut invalid = fields();
        invalid["issuer"] = json!(format!("invalid/{index}"));
        append(&store,SUBJECT,GRANTED,invalid);
    }
    drain_authority(&store);
    let mut connection = store.connection.write();
    let markers = backfill_markers(&connection);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h4863c5bdfd48 owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO assistant_grant_dirty SELECT id FROM claims WHERE kind=?1", [GRANTED]).unwrap();
    let mut remaining = authority::CLAIMS_PER_SLICE * 3 + 7;
    loop {
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap();
        let after: usize = transaction.query_row("SELECT COUNT(*) FROM assistant_grant_dirty", [], |row| row.get(0)).unwrap();
        // Dirty authentication owns half the 64-unit slice; the other half
        // remains available to proof withdrawals and membership additions.
        assert_eq!(remaining-after, remaining.min(authority::CLAIMS_PER_SLICE.div_ceil(2)));
        assert_eq!(backfill_markers(&transaction),markers,"completed history was rescanned");
        transaction.commit().unwrap();
        remaining = after;
        if done { break; }
    }
    assert_eq!(remaining,0);
}

fn authentication_key_cost(connection: &Connection, signature: &ReplicaEnvelopeSignature) -> (usize,i32,i32) {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h418f3ec55f8d owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ authority::AUTHENTICATION_KEYS_QUERY).unwrap();
    let rows = statement.query_map(
        params![signature.writer,signature.sequence,signature.hash,-1,"",authority::MEMBERSHIP_KEYS_PER_REFRESH],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)),
    ).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    (rows.len(),statement.get_status(StatementStatus::VmStep),statement.get_status(StatementStatus::FullscanStep))
}

#[test]
fn nonmember_signature_flood_has_constant_membership_first_vm_cost() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    let exchange = export(&source,&target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    let mut connection = target.connection.write();
    let before = authentication_key_cost(&connection,&signature);
    assert_eq!(before.0,1);
    assert_eq!(before.2,0);
    for count in [128,4096] {
        for index in 0..count {
            let mut nonmember = signature.clone();
            nonmember.member_key = format!("nonmember/{count}/{index:08}");
            nonmember.signature = "invalid".into();
            insert_raw_signature(&connection,&nonmember);
        }
        assert_eq!(authentication_key_cost(&connection,&signature),before,
            "nonmember signature rows increased authentication VM work");
        let transaction = connection.transaction().unwrap();
        assert!(authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
        assert!(!authority::contains(&transaction,&grant.id).unwrap());
        transaction.commit().unwrap();
    }
    // Real late evidence still verifies after the arbitrary-signature flood.
    insert_raw_signature(&connection,&signature);
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    assert!(authority::contains(&transaction,&grant.id).unwrap());
    transaction.commit().unwrap();
}

#[test]
fn eligible_evidence_beyond_first_key_page_resumes_and_cryptographically_verifies() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    let exchange = export(&source,&target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    let mut connection = target.connection.write();
    let markers = backfill_markers(&connection);
    // These deliberately malformed persisted proofs must not authenticate.
    // Their eligible windows sort before the real member's key.
    for index in 0..(authority::MEMBERSHIP_KEYS_PER_REFRESH*3+1) {
        let mut invalid = signature.clone();
        invalid.member_key = format!("!{index:08}");
        invalid.signature = "invalid".into();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h02562e83e833 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,0,NULL)",
            params![invalid.writer,invalid.member_key]).unwrap();
        insert_raw_signature(&connection,&invalid);
    }
    insert_raw_signature(&connection,&signature);
    let cost = authentication_key_cost(&connection,&signature);
    assert_eq!(cost.0,authority::MEMBERSHIP_KEYS_PER_REFRESH);
    assert_eq!(cost.2,0);
    let mut previous_key = String::new();
    for page in 0..4 {
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap();
        assert_eq!(backfill_markers(&transaction),markers);
        if page<3 {
            assert!(!done,"invalid earlier evidence authenticated the claim");
            assert!(authority::ready(&transaction).is_ok());
            assert!(authority::subject_ready(&transaction, SUBJECT).is_err());
            assert!(authority::maintenance_pending(&transaction).unwrap());
            let cursor: String = transaction.query_row(
                "SELECT member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
                [&grant.id], |row| row.get(0)).unwrap();
            assert!(cursor>previous_key,"authentication did not advance durably");
            previous_key = cursor;
            let admitted: bool = transaction.query_row(
                "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1", [&grant.id], |row| row.get(0)).unwrap();
            assert!(!admitted);
        } else {
            assert!(done);
            assert!(authority::contains(&transaction,&grant.id).unwrap());
        }
        transaction.commit().unwrap();
    }
}

#[test]
fn key_only_authentication_cursor_migration_requeues_without_trusting_old_position() {
    let store = node("issuer",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h1ae4a2fcf40e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "ALTER TABLE assistant_grant_auth_cursor DROP COLUMN membership_start;"
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-ha9371a8fef8c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor VALUES(?1,9223372036854775807,9223372036854775807,'~stale')",
        [&grant.id]).unwrap();
    authority::install(&connection,"issuer").unwrap();
    assert!(!connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)",
        [&grant.id], |row| row.get::<_,bool>(0)).unwrap());
    assert!(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)",
        [&grant.id], |row| row.get::<_,bool>(0)).unwrap(),
        "migration must retain the interrupted authentication obligation");
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hba715e11f267 owner=smallclaims::store::assistant_grant_authority_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO assistant_grant_auth_cursor VALUES(?1,7,9,'new-checkpoint',3)",
        [&grant.id]).unwrap();
    authority::install(&connection,"issuer").unwrap();
    let cursor: (i64,i64,String,i64) = connection.query_row(
        "SELECT position,record_rowid,member_key,membership_start FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(cursor,(7,9,"new-checkpoint".into(),3),
        "installing the current schema must preserve resumable work");
}

#[test]
fn expired_membership_windows_consume_bounded_pages_and_future_windows_stay_out_of_seek() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    let exchange = export(&source,&target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    assert!(signature.sequence>0);
    let mut connection = target.connection.write();
    // Put the real key after all expired windows by start, not member-key order.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h06703a74a220 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_windows SET start=?2 WHERE writer=?1 AND member_key=?3",
        params![signature.writer,signature.sequence,signature.member_key]).unwrap();
    for index in 0..(authority::MEMBERSHIP_KEYS_PER_REFRESH*3+1) {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h3bfdf85ceb51 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,0,0)",
            params![signature.writer,format!("~expired/{index:08}")]).unwrap();
    }
    insert_raw_signature(&connection,&signature);
    let before = authentication_key_cost(&connection,&signature);
    assert_eq!(before.0,authority::MEMBERSHIP_KEYS_PER_REFRESH);
    assert_eq!(before.2,0);
    for index in 0..4096 {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h0e74c062c397 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,NULL)",
            params![signature.writer,format!("!future/{index:08}"),signature.sequence+1]).unwrap();
    }
    assert_eq!(authentication_key_cost(&connection,&signature),before,
        "future-only windows entered the candidate seek");
    let mut previous = (-1,String::new());
    for page in 0..4 {
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap();
        if page<3 {
            assert!(!done,"expired candidates must consume their own page budget");
            let cursor: (i64,String) = transaction.query_row(
                "SELECT membership_start,member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
                [&grant.id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert!(cursor>previous,"window cursor did not advance monotonically");
            previous = cursor;
            assert!(!authority::contains(&transaction,&grant.id).unwrap());
        } else {
            assert!(done);
            assert!(authority::contains(&transaction,&grant.id).unwrap());
        }
        transaction.commit().unwrap();
    }
    // The signature remains cryptographically valid, but an expired window
    // must never authenticate it during a subsequent proof recheck.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h64769f90bcc0 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_windows SET start=0,end=0 WHERE writer=?1 AND member_key=?2",
        params![signature.writer,signature.member_key]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h642b57ed2c8a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks VALUES(?1)", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hafd668e892ee owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    drop(connection);
    drain_authority(&target);
    assert!(!authority::contains(&target.readers.get(),&grant.id).unwrap(),
        "an expired window authenticated a valid signature");
}

#[test]
fn membership_addition_receipt_flood_uses_the_same_fixed_refresh_budget() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    for _ in 0..(authority::CLAIMS_PER_SLICE*2+7) {
        append(&source,SUBJECT,GRANTED,fields());
    }
    receive_pre_projection_unsigned_history(&target, &export(&source,&target));
    let mut connection = target.connection.write();
    let markers = backfill_markers(&connection);
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h00986dca7a94 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_additions(writer,member_key,lower_sequence,upper_sequence)
         VALUES('anchor',?1,0,9223372036854775807)", [key.public()],
    ).unwrap();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h49244ed4f44b owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE refreshed_items (claim_id TEXT);
         CREATE TEMP TRIGGER count_membership_refresh AFTER UPDATE ON assistant_grant_additions
         BEGIN INSERT INTO refreshed_items VALUES(NEW.sequence); END;",
    ).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    let refreshed: usize = transaction.query_row(
        "SELECT COUNT(*) FROM refreshed_items", [], |row| row.get(0)).unwrap();
    assert_eq!(refreshed,authority::CLAIMS_PER_SLICE);
    assert!(authority::ready(&transaction).is_ok());
    assert!(authority::subject_ready(&transaction, SUBJECT).is_err());
    assert!(authority::maintenance_pending(&transaction).unwrap());
    assert_eq!(backfill_markers(&transaction),markers);
    transaction.commit().unwrap();
    drop(connection);
    drain_authority(&target);
    assert!(decisions(&target,0).is_empty(),"unsigned pending grants acquired authority");
}

#[test]
fn late_signature_restarts_an_in_progress_key_cursor_before_its_old_position() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    let exchange = export(&source,&target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    let mut connection = target.connection.write();
    let member_start: u64 = connection.query_row(
        "SELECT start FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
        params![signature.writer,signature.member_key], |row| row.get(0),
    ).unwrap();
    // The cursor now orders by (window start,key), not key alone. Keep these
    // windows in the same start range so the genuine key precedes the cursor.
    for index in 0..(authority::MEMBERSHIP_KEYS_PER_REFRESH*2) {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h3135f9c18750 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,NULL)",
            params![signature.writer,format!("~{index:08}"),member_start]).unwrap();
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h403d59b4d3b8 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    let cursor: String = transaction.query_row(
        "SELECT member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| row.get(0)).unwrap();
    assert!(cursor>signature.member_key);
    transaction.commit().unwrap();
    insert_raw_signature(&connection,&signature);
    let cursors: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| row.get(0)).unwrap();
    assert_eq!(cursors,0,"new evidence did not invalidate the earlier search position");
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    assert!(authority::contains(&transaction,&grant.id).unwrap());
    transaction.commit().unwrap();
}

#[test]
fn envelope_record_arriving_before_auth_cursor_restarts_preexisting_signature_search() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    let exchange = export(&source,&target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    let signature = signatures(&exchange).signatures.into_iter().find(|signature| {
        target.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND sequence=?2)",
            params![grant.id,signature.sequence], |row| row.get::<_,bool>(0)).unwrap()
    }).unwrap();
    let mut connection = target.connection.write();
    // Keep an unsigned later-position copy, leaving the signed envelope with
    // no attached claim record when its signature arrives.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h664aa3828314 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_envelopes
         SELECT writer,sequence,'decoy-envelope',previous_hash,accepted_at_unix_ms,
                payload,batch_id,relay,receipt_state,validation_error,received_at_unix_ms
         FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        params![signature.writer,signature.sequence,signature.hash]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h1c051df8af17 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET envelope_hash='decoy-envelope',position=1 WHERE claim_id=?1",
        [&grant.id]).unwrap();
    for index in 0..(authority::MEMBERSHIP_KEYS_PER_REFRESH*2) {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hec4dd1196f81 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,0,NULL)",
            params![signature.writer,format!("~{index:08}")]).unwrap();
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-hcd2447942ecb owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    let position: i64 = transaction.query_row(
        "SELECT position FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| row.get(0)).unwrap();
    assert_eq!(position,1);
    transaction.commit().unwrap();
    insert_raw_signature(&connection,&signature);
    let cursors: usize = connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| row.get(0)).unwrap();
    assert_eq!(cursors,1,"signature unexpectedly had a linked claim record");
    let transaction = connection.transaction().unwrap();
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-h74c78298f3cf owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records
            (record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms)
         SELECT 'earlier-signed-record',writer,sequence,?2,0,raw,'valid',claim_id,'0'
         FROM replica_records WHERE claim_id=?1",
        params![grant.id,signature.hash]).unwrap();
    authority::refresh_envelope(&transaction,&signature.writer,signature.sequence,&signature.hash).unwrap();
    let cursor_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    assert!(!cursor_exists, "new earlier evidence must restart its authentication cursor");
    transaction.commit().unwrap();
    drop(connection);
    drain_authority(&target);
    assert!(authority::contains(&target.readers.get(),&grant.id).unwrap(),
        "the earlier signed record was hidden behind the old position/key cursor");
}

fn independent_grant(store: &Store) -> ClaimRecord {
    let suffix = "fedcba9876543210fedcba98";
    let mut independent = fields();
    independent["grant_id"] = json!(format!("assistant-grant/{suffix}"));
    independent["session_actor"] = json!(format!("assistant/chatgpt/session/assistant-grant/{suffix}"));
    independent["credential_hash"] = json!("b".repeat(64));
    append(store, &format!("custom/client/assistant-v1-{suffix}"), GRANTED, independent)
}

fn assert_subject_fenced(connection: &Connection, grant: &ClaimRecord, independent: &ClaimRecord) {
    authority::ready(connection).unwrap();
    assert!(authority::subject_ready(connection, &grant.subject).is_err());
    assert!(!authority::contains(connection, &grant.id).unwrap());
    assert!(authority::by_credential(connection, grant.body["fields"]["credential_hash"].as_str().unwrap()).unwrap().is_none());
    assert!(authority::latest(connection, &grant.subject, GRANTED).is_err());
    for kind in [POLICY, REVOKED] {
        assert!(authority::transition(connection, grant, kind).is_err());
    }
    authority::subject_ready(connection, &independent.subject).unwrap();
    assert!(authority::contains(connection, &independent.id).unwrap());
    assert_eq!(authority::by_credential(connection, independent.body["fields"]["credential_hash"].as_str().unwrap()).unwrap().unwrap().id, independent.id);
    assert_eq!(authority::latest(connection, &independent.subject, GRANTED).unwrap().unwrap().id, independent.id);
    for kind in [POLICY, REVOKED] {
        assert!(authority::transition(connection, independent, kind).unwrap().is_none());
    }
    for cutoff in [Some(0), None] {
        let candidates = authority::candidates(connection, "assistant/chatgpt", OWNER, cutoff, None, None).unwrap();
        // Conflict snapshots retain this authenticated witness even while its
        // own credential is fenced; a pending revoke cannot clear a conflict.
        assert_eq!(candidates.iter().map(|claim| claim.id.as_str()).collect::<BTreeSet<_>>(),
            BTreeSet::from([grant.id.as_str(), independent.id.as_str()]));
    }
}

#[test]
fn dirty_and_auth_cursor_work_fence_only_their_subject_on_every_read_path() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let independent = independent_grant(&store);
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute(/* sqlite-large-value: allow-for-now lv-hf6ea61b38c5c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    assert!(authority::maintenance_pending(&connection).unwrap());
    assert_subject_fenced(&connection, &grant, &independent);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h7494541ec904 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h5b218f4cabfc owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start) VALUES(?1,0,0,'',-1)",
        [&grant.id],
    ).unwrap();
    assert_subject_fenced(&connection, &grant, &independent);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h15542ec4c6ea owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&grant.id]).unwrap();
    assert!(authority::contains(&connection, &grant.id).unwrap());
}

#[test]
fn unsigned_pending_revoke_fences_only_its_established_subject_after_dirty_work_drains() {
    let (source, key) = source();
    let target = node("reader", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    receive(&target, &export(&source, &target));
    let independent = independent_grant(&target);
    let revoke = append(&source, SUBJECT, REVOKED, fields());
    let exchange = export(&source, &target);
    receive_pre_projection_unsigned_history(&target, &exchange);
    drain_authority(&target);
    {
        let connection = target.readers.get();
        assert!(!authority::maintenance_pending(&connection).unwrap());
        let authenticated: bool = connection.query_row(
            "SELECT authenticated FROM assistant_grant_pending WHERE claim_id=?1",
            [&revoke.id], |row| row.get(0),
        ).unwrap();
        assert!(!authenticated);
        assert_subject_fenced(&connection, &grant, &independent);
    }
    receive(&target, &signatures(&exchange));
    let connection = target.readers.get();
    authority::subject_ready(&connection, SUBJECT).unwrap();
    assert_eq!(authority::transition(&connection, &grant, REVOKED).unwrap().unwrap().id, revoke.id);
    assert_eq!(decisions(&target, 0), vec![independent.id]);
}

#[test]
fn subject_readiness_seeks_only_its_indexed_obligations_with_large_unrelated_queues() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let independent = independent_grant(&store);
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h5df4dd1dd74d owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE assistant_grant_pending SET auth_member_key='lost-key',auth_sequence=7,
         auth_envelope_hash='proof' WHERE claim_id=?1", [&grant.id],
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hc91d2f57cab1 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence)
         VALUES(?1,'lost-key',7,7)", [&grant.origin],
    ).unwrap();
    let cost = |subject: &str| {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hb30e13c4dc6f owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ authority::SUBJECT_PENDING_QUERY).unwrap();
        let pending: bool = statement.query_row(/* sqlite-large-value: allow-for-now lv-hade54ba19418 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0)).unwrap();
        (pending,statement.get_status(StatementStatus::VmStep),
            statement.get_status(StatementStatus::FullscanStep))
    };
    let fenced = cost(&grant.subject);
    let clean = cost(&independent.subject);
    assert!(fenced.0);
    assert!(!clean.0);
    assert_eq!(fenced.2,0);
    assert_eq!(clean.2,0);
    for index in 0..4096 {
        let writer = format!("unrelated-{index}");
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hb8b541534641 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence)
             VALUES(?1,'lost-key',0,100)", [&writer],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h77dca66dd9fc owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key)
             VALUES(?1,7,'proof','lost-key')", [&writer],
        ).unwrap();
    }
    assert_eq!(cost(&grant.subject),fenced);
    assert_eq!(cost(&independent.subject),clean);
}

#[test]
fn legacy_membership_dirty_obligations_migrate_idempotently_to_verifier_rechecks() {
    let (source,key) = source();
    let target = node("reader",Some(key.public()));
    receive(&target,&export(&source,&target));
    let grant = append(&source,SUBJECT,GRANTED,fields());
    receive_pre_projection_unsigned_history(&target,&export(&source,&target));
    let connection = target.connection.write();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h64bd448a0436 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE assistant_grant_membership_dirty (
           writer TEXT PRIMARY KEY, sequence INTEGER NOT NULL, claim_id TEXT NOT NULL);"
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8ae28328b54c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_dirty VALUES(?1,-1,'')", [&grant.origin],
    ).unwrap();
    for _ in 0..2 {
        authority::install(&connection,"reader").unwrap();
        assert!(!connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='assistant_grant_membership_dirty')",
            [], |row| row.get::<_,bool>(0),
        ).unwrap());
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
            [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap());
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_work WHERE writer=?1 AND claim_id=?2)",
            params![grant.origin,grant.id], |row| row.get::<_,bool>(0),
        ).unwrap());
        assert!(authority::subject_ready(&connection,SUBJECT).is_err());
    }
}

#[test]
fn auth_work_set_is_idempotent_under_outer_upsert_conflict_policy() {
    let store = node("issuer",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    let connection = store.connection.write();
    for _ in 0..2 {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h5ae439b456a4 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start)
             VALUES(?1,0,0,'',-1) ON CONFLICT(claim_id) DO UPDATE SET position=excluded.position",
            [&grant.id],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hef325d41f3b7 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks VALUES(?1)
             ON CONFLICT(claim_id) DO UPDATE SET claim_id=excluded.claim_id", [&grant.id],
        ).unwrap();
        assert_eq!(connection.query_row(
            "SELECT COUNT(*) FROM assistant_grant_auth_work WHERE claim_id=?1",
            [&grant.id], |row| row.get::<_,usize>(0),
        ).unwrap(),1);
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h32d93849d196 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&grant.id]).unwrap();
    assert_eq!(connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_auth_work WHERE claim_id=?1",
        [&grant.id], |row| row.get::<_,usize>(0),
    ).unwrap(),1,"another durable obligation still needs this claim");
    connection.execute(/* sqlite-large-value: allow-for-now lv-ha57a171023e8 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_rechecks WHERE claim_id=?1", [&grant.id]).unwrap();
    assert_eq!(connection.query_row(
        "SELECT COUNT(*) FROM assistant_grant_auth_work WHERE claim_id=?1",
        [&grant.id], |row| row.get::<_,usize>(0),
    ).unwrap(),0);
}

#[test]
fn dirty_work_before_saved_round_robin_cursor_is_serviced_in_the_same_slice() {
    let store = node("issuer",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    let mut connection = store.connection.write();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h74200cad8b0c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_dirty_cursor',?1)",
        [format!("{}z",grant.id)],
    ).unwrap();
    connection.execute(/* sqlite-large-value: allow-for-now lv-hb7abee1d3c34 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty VALUES(?1)", [&grant.id]).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    assert!(authority::contains(&transaction,&grant.id).unwrap());
    transaction.commit().unwrap();
}

/* sqlite-large-value: allow-for-now lv-h70e364d23629 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ include!("assistant_grant_receipt_cost_tests.rs");

#[path = "assistant_grant_membership_property_tests.rs"]
mod membership_property_tests;
