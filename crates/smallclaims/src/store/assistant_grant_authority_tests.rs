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

#[test]
fn current_authority_install_and_idle_maintenance_do_not_write() {
    let store = Store::open_memory("anchor", Arc::new(Plain)).unwrap();
    store.persist_assistant_grant_membership_for_test().unwrap();
    let connection = store.connection.write();
    let before = (
        connection.pragma_query_value(None, "schema_version", |row| row.get::<_, i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0)).unwrap(),
    );
    authority::install(&connection, "anchor").unwrap();
    assert!(authority::backfill_slice(&connection, Duration::from_secs(2)).unwrap());
    let after = (
        connection.pragma_query_value(None, "schema_version", |row| row.get::<_, i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0)).unwrap(),
    );
    assert_eq!(after, before, "unchanged install and empty queues must be read-only");
}

#[test]
fn authority_ddl_digest_matches_recorded_schema_version() {
    use sha2::{Digest as _, Sha256};
    let store = Store::open_memory("ddl-digest", Arc::new(Plain)).unwrap();
    let connection = store.connection.write();
    authority::install(&connection, "ddl-digest").unwrap();
    let version: String = connection.query_row(
        "SELECT value FROM meta WHERE key='assistant_grant_authority_schema_version'",
        [], |row| row.get(0),
    ).unwrap();
    let mut statement = connection.prepare(
        "SELECT type,name,sql FROM sqlite_master
         WHERE type IN ('table','index','trigger') AND sql IS NOT NULL
           AND (name GLOB 'assistant_grant_*' OR name='replica_records_writer_grant_proof_index')
         ORDER BY type COLLATE BINARY,name COLLATE BINARY",
    ).unwrap();
    let objects = statement.query_map([], |row| Ok([
        row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,
    ])).unwrap();
    let mut digest = Sha256::new();
    for object in objects {
        // Length-prefix each UTF-8 field; object boundaries and SQL whitespace
        // are significant, without depending on JSON serialization behavior.
        for field in object.unwrap() {
            digest.update(u64::try_from(field.len()).unwrap().to_be_bytes());
            digest.update(field.as_bytes());
        }
    }
    let digest = hex::encode(digest.finalize());
    assert_eq!((version.as_str(),digest.as_str()),authority::SCHEMA_DDL_FINGERPRINT,
        "authority DDL changed: bump SCHEMA_VERSION, provide any required migration, and update SCHEMA_DDL_FINGERPRINT with the new version and digest");
}

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
fn assistant_grant_and_policy_accept_main_slo_and_availability_scopes() {
    for scope in ["read.assistant.slo", "read.assistant.availability"] {
        let store = node("issuer", None);
        let mut source = fields();
        source["scopes"] = json!([scope]);
        let grant = append(&store, SUBJECT, GRANTED, source.clone());
        assert!(authority::contains(&store.readers.get(), &grant.id).unwrap());
        source["scopes"] = json!(["read.assistant.summary", scope]);
        source["policy_revision"] = json!(2);
        let policy = append(&store, SUBJECT, POLICY, source);
        let projected = authority::latest(&store.readers.get(), SUBJECT, POLICY).unwrap().unwrap();
        assert_eq!(projected.id, policy.id);
        assert_eq!(projected.body["fields"]["policy_revision"], 2);
        assert!(authority::contains(&store.readers.get(), &grant.id).unwrap());
    }
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
    // Model an older build's already-admitted unsigned history; ordinary keyed
    // receive correctly holds the unsigned envelope before claim projection.
    receive_pre_projection_unsigned_history(&target, &exchange);
    drain_authority(&target);
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
    drain_authority(&store);
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
        let mut slices = 0;
        while !target.advance_assistant_grant_authority_backfill().unwrap() {
            slices += 1;
            assert!(slices < 10_000,
                "raw_signature_only_insert_survives_reopen_and_promotes_grant_policy_and_revoke_without_rescan drain exceeded 10000 slices");
        }
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
    let mut slices = 0;
    while !target.advance_assistant_grant_authority_backfill().unwrap() {
        slices += 1;
        assert!(slices < 10_000,
            "malformed_raw_signature_row_is_drained_without_authenticating_a_pending_revoke drain exceeded 10000 slices");
    }
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
        /* sqlite-large-value: allow-for-now lv-h90b20a930910 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM assistant_grant_effective;
         DELETE FROM assistant_grant_authority;
         DELETE FROM assistant_grant_pending;
         DELETE FROM assistant_grant_attachments;
         DELETE FROM assistant_grant_attachment_witnesses;
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
    let mut slices = 0;
    while !store.advance_assistant_grant_authority_backfill().unwrap() {
        slices += 1;
        assert!(slices < 10_000, "drain_authority exceeded 10000 slices");
    }
    // Raw SQL slice drivers need the durable generation, even when Store's
    // empty-membership fast path can answer from its in-memory snapshot.
    store.persist_assistant_grant_membership_for_test().unwrap();
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
    reset_projection(&victim.connection.write());
    victim.persist_assistant_grant_membership_for_test().unwrap();
    {
        let mut connection = victim.connection.write();
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
        drain_authority(&store);
        let connection = store.connection.write();
        expected = snapshot(&connection);
        assert_eq!(expected.len(), 70);
        reset_projection(&connection);
        drop(connection);
        store.persist_assistant_grant_membership_for_test().unwrap();
        let mut connection = store.connection.write();
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
            assert!(slices < 8*(70*3+3),
                "committed_backfill_slice_resumes_after_reopen_and_is_idempotent failed to make durable forward progress");
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
    connection.execute(/* sqlite-large-value: allow-for-now lv-h9e84ba14b6ed owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
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
fn every_nonempty_maintenance_queue_receives_a_unit_within_one_zero_slice_round() {
    // History and unresolved-cursor recovery are empty after completed backfill.
    // Exercise every starting turn, including those empty slots, with all seven
    // ordinary maintenance queues continuously occupied.
    const Q: usize = 9;
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let malformed_dirty = append(&store, SUBJECT, GRANTED, json!({}));
    let malformed_recheck = append(&store, SUBJECT, GRANTED, json!({}));
    drain_authority(&store);
    let mut connection = store.connection.write();
    let markers = backfill_markers(&connection);
    let expected = snapshot(&connection);
    let queues = [
        "assistant_grant_dirty", "assistant_grant_proof_rechecks",
        "assistant_grant_proof_invalidations", "assistant_grant_retractions",
        "assistant_grant_additions", "assistant_grant_attachments",
        "assistant_grant_attachment_witnesses",
    ];
    for start in 0..Q {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h6092688f3bb2 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn',?1)",
            [start.to_string()],
        ).unwrap();
        let mut visited = [false; 7];
        for _ in 0..Q {
            // Malformed real claims retire without scheduling missing-claim
            // recomputation, whose priority units are outside the fair round.
            // None of these tasks supplies authentication or inserts a forged
            // authenticated pending row.
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-he2f5293a47f9 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty(claim_id) VALUES(?1)",
                [&malformed_dirty.id],
            ).unwrap();
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hd444feebe8bf owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks(claim_id) VALUES(?1)",
                [&malformed_recheck.id],
            ).unwrap();
            connection.execute_batch(
                /* sqlite-large-value: allow-for-now lv-h3163101c230b owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO assistant_grant_proof_invalidations
                                     (id,writer,sequence,envelope_hash,member_key)
                                     VALUES(1,'fairness/missing-writer',0,'missing','missing');
                                 INSERT OR IGNORE INTO assistant_grant_retractions
                                     (id,writer,member_key,lower_sequence,upper_sequence)
                                     VALUES(1,'fairness/missing-writer','missing',0,0);
                                 INSERT OR IGNORE INTO assistant_grant_additions
                                     (id,writer,member_key,lower_sequence,upper_sequence)
                                     VALUES(1,'fairness/missing-writer','missing',0,0);
                                 INSERT OR IGNORE INTO assistant_grant_attachments
                                     VALUES('fairness/attachment','custom.client.assistant.v1.granted',
                                            'fairness/unfinished','missing','missing','','');",
            ).unwrap();
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h304ebce462ea owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_attachment_witnesses
                 SELECT claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record
                 FROM assistant_grant_effective WHERE claim_id=?1", [&grant.id],
            ).unwrap();
            let transaction = connection.transaction().unwrap();
            assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
            let mut units = 0;
            for (index, queue) in queues.iter().enumerate() {
                let left: usize = transaction.query_row(
                    /* sqlite-large-value: allow-for-now lv-hdfcb841d79cd owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT COUNT(*) FROM {queue}"), [], |row| row.get(0),
                ).unwrap();
                assert!(left<=1, "unexpected work in {queue}");
                if left==0 {
                    visited[index] = true;
                    units += 1;
                }
            }
            assert_eq!(units, 1, "ZERO must perform exactly one occupied visit");
            assert_eq!(backfill_markers(&transaction), markers, "completed history was rescanned");
            assert_eq!(snapshot(&transaction), expected, "stale queue tasks changed authority");
            transaction.commit().unwrap();
        }
        assert!(visited.iter().all(|seen| *seen),
            "starting turn {start} starved a queue within {Q} ZERO slices: {visited:?}");
    }
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
        // This test measures one authentication page, not a full fair round.
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h438a93c1e680 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn','0')", [],
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::ZERO).unwrap();
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
            assert!(!done, "the final occupied ZERO visit still consumes its slice");
            assert!(authority::contains(&transaction,&grant.id).unwrap());
        }
        transaction.commit().unwrap();
    }
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction, Duration::from_secs(60)).unwrap());
    transaction.commit().unwrap();
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
        /* sqlite-large-value: allow-for-now lv-h63d29c8422d2 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key) VALUES(?1,9223372036854775807,9223372036854775807,'~stale')",
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
        /* sqlite-large-value: allow-for-now lv-h654192aa14f2 owner=smallclaims::store::assistant_grant_authority_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start,grant_subject) VALUES(?1,7,9,'new-checkpoint',3,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))",
        [&grant.id]).unwrap();
    let before = (
        connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
    );
    authority::install(&connection,"issuer").unwrap();
    let cursor: (i64,i64,String,i64) = connection.query_row(
        "SELECT position,record_rowid,member_key,membership_start FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(cursor,(7,9,"new-checkpoint".into(),3),
        "installing the current schema must preserve resumable work");
    assert_eq!((
        connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
    ),before,"current resumable work must not make a repeated install write");
}

#[test]
fn marked_schema_migrates_legacy_addition_and_missing_auth_work_without_reinstalling_triggers() {
    let store = node("issuer",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h2693491dc162 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "ALTER TABLE assistant_grant_additions DROP COLUMN claim_id;
         ALTER TABLE assistant_grant_additions DROP COLUMN active_claim;
         INSERT INTO assistant_grant_additions
             (writer,member_key,lower_sequence,upper_sequence,sequence,envelope_hash,position)
             VALUES('legacy-writer','legacy-key',3,9,7,'stale-envelope',99);
         DELETE FROM meta WHERE key='assistant_grant_auth_work_installed';"
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8ca0c46fa209 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor
             (claim_id,position,record_rowid,member_key,membership_start,grant_subject) VALUES(?1,7,9,'current-key',3,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))",
        [&grant.id],
    ).unwrap();
    connection.execute(/* sqlite-large-value: allow-for-now lv-h6142d0df5172 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_work",[]).unwrap();
    authority::install(&connection,"issuer").unwrap();
    let addition: (String,String,i64,i64,i64,String,i64,String,String) = connection.query_row(
        "SELECT writer,member_key,lower_sequence,upper_sequence,sequence,envelope_hash,position,claim_id,active_claim
         FROM assistant_grant_additions",[],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,
            row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?)),
    ).unwrap();
    assert_eq!(addition,("legacy-writer".into(),"legacy-key".into(),3,9,-1,"".into(),-1,"".into(),"".into()));
    assert!(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_work WHERE writer=?1 AND claim_id=?2)",
        params![grant.origin,grant.id],|row| row.get::<_,bool>(0),
    ).unwrap(),"missing initialization must restore current cursor obligations");
    let before = (
        connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
    );
    authority::install(&connection,"issuer").unwrap();
    assert_eq!((
        connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
        connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
    ),before,"completed legacy migrations must not write on reopen");
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
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h26af0e87fa91 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn','0')", [],
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::ZERO).unwrap();
        if page<3 {
            assert!(!done,"expired candidates must consume their own page budget");
            let cursor: (i64,String) = transaction.query_row(
                "SELECT membership_start,member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
                [&grant.id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert!(cursor>previous,"window cursor did not advance monotonically");
            previous = cursor;
            assert!(!authority::contains(&transaction,&grant.id).unwrap());
        } else {
            assert!(!done, "the final occupied ZERO visit still consumes its slice");
            assert!(authority::contains(&transaction,&grant.id).unwrap());
        }
        transaction.commit().unwrap();
    }
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction, Duration::from_secs(60)).unwrap());
    transaction.commit().unwrap();
    // The signature remains cryptographically valid, but an expired window
    // must never authenticate it during a subsequent proof recheck.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h64769f90bcc0 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_windows SET start=0,end=0 WHERE writer=?1 AND member_key=?2",
        params![signature.writer,signature.member_key]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h94131a54d684 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hc8e60c561999 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
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
    connection.execute(/* sqlite-large-value: allow-for-now lv-hf4663a2ec011 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h1572d9b159f1 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn','0')", [],
    ).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction,Duration::ZERO).unwrap());
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
    connection.execute(/* sqlite-large-value: allow-for-now lv-he1b8b803d252 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h07764852571e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn','0')", [],
    ).unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(!authority::backfill_slice(&transaction,Duration::ZERO).unwrap());
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
    connection.execute(/* sqlite-large-value: allow-for-now lv-h07bb0fee081e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
    assert!(authority::maintenance_pending(&connection).unwrap());
    assert_subject_fenced(&connection, &grant, &independent);
    connection.execute(/* sqlite-large-value: allow-for-now lv-h7494541ec904 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [&grant.id]).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h2398935b1a44 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start,grant_subject) VALUES(?1,0,0,'',-1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))",
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
    for (subject, before) in [(&grant.subject, fenced), (&independent.subject, clean)] {
        let after = cost(subject);
        assert_eq!(after.0, before.0);
        // Tasks are deliberately outermost; each unrelated task is one indexed
        // pending seek, never a scan of retained subject history.
        eprintln!("subject-task-growth subject={subject} before={before:?} after={after:?}");
        assert!(after.1 <= before.1 + 4096*256, "{before:?} -> {after:?}");
        assert!(after.2 <= before.2 + 4096*2, "{before:?} -> {after:?}");
    }
}

// Exercise the production query and indexes, not an EXPLAIN-plan surrogate.
fn subject_pending_cost(connection: &rusqlite::Connection, subject: &str) -> (bool, i32, i32) {

    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h0e2286e502a4 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ authority::SUBJECT_PENDING_QUERY).unwrap();

    let pending = statement.query_row(/* sqlite-large-value: allow-for-now lv-h79924becc4c4 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0)).unwrap();
    (pending, statement.get_status(StatementStatus::VmStep),
        statement.get_status(StatementStatus::FullscanStep))
}

#[test]
fn assistant_grant_empty_queues_skip_subject_history() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    drain_authority(&store);
    let connection = store.connection.write();
    let mut costs = Vec::new();
    for history in [10, 10_000] {
        connection.execute(

            /* sqlite-large-value: allow-for-now lv-h351e673ef1be owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?2)
             INSERT OR IGNORE INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             SELECT 'history-'||x,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
             FROM n CROSS JOIN claims WHERE claims.id=?1",
            params![grant.id,history],
        ).unwrap();
        connection.execute(

            /* sqlite-large-value: allow-for-now lv-h860ed3718c70 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO assistant_grant_pending
             (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record,
              auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position)
             SELECT id,subject,kind,origin,actor,0,1,'{}','key',7,'proof',0
             FROM claims WHERE subject=?1", [SUBJECT],
        ).unwrap();

        connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hf7a67fc613de owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty;
            DELETE FROM assistant_grant_auth_cursor;
            DELETE FROM assistant_grant_recompute;
            DELETE FROM assistant_grant_proof_rechecks;
            DELETE FROM assistant_grant_proof_invalidations;
            DELETE FROM assistant_grant_retractions;").unwrap();
        let cost = subject_pending_cost(&connection, SUBJECT);
        eprintln!("empty queues: history={history} pending={} vm_steps={} fullscan_steps={}",cost.0,cost.1,cost.2);
        assert!(!cost.0);
        assert_eq!(cost.2, 0);
        costs.push(cost);
    }
    assert_eq!(costs[0], costs[1], "empty queues must not walk subject history");
}

#[test]
fn assistant_grant_queue_guards_preserve_every_matching_fence_and_subject_outer_seeks() {
    let store = node("issuer", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute(

        /* sqlite-large-value: allow-for-now lv-ha1de1418b8ce owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE assistant_grant_pending SET authenticated=1,auth_member_key='lost-key',
         auth_sequence=7,auth_envelope_hash='proof' WHERE claim_id=?1", [&grant.id],
    ).unwrap();
    // One test per guarded arm, including a pending-only dirty claim that is
    // absent from claims, so the first dirty arm cannot mask the second.
    for (name, insert, cleanup) in [

        ("claims dirty", /* sqlite-large-value: allow-for-now lv-ha22427005a15 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", /* sqlite-large-value: allow-for-now lv-h97aab8f622b3 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty"),

        ("pending dirty", /* sqlite-large-value: allow-for-now lv-h056f6dccca0a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) SELECT claim_id,grant_subject FROM assistant_grant_pending WHERE claim_id='pending-only'", /* sqlite-large-value: allow-for-now lv-h6e723f23d346 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty"),

        ("auth cursor", /* sqlite-large-value: allow-for-now lv-hd514453e40c1 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start,grant_subject) VALUES(?1,0,0,'',-1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", /* sqlite-large-value: allow-for-now lv-h04c9ef30f056 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor"),

        ("recompute", /* sqlite-large-value: allow-for-now lv-h71ad4e5f974d owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_recompute(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", /* sqlite-large-value: allow-for-now lv-h33cd101f3ed9 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_recompute"),

        ("proof recheck", /* sqlite-large-value: allow-for-now lv-h6fd9973dd145 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", /* sqlite-large-value: allow-for-now lv-h0abdc42f46bc owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_rechecks"),

        ("proof invalidation", /* sqlite-large-value: allow-for-now lv-h0efef89dc2e7 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key) SELECT origin,7,'proof','lost-key' FROM assistant_grant_pending WHERE claim_id=?1", /* sqlite-large-value: allow-for-now lv-ha4801ddb5300 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_invalidations"),

        ("retraction", /* sqlite-large-value: allow-for-now lv-hf7b09a18fb7f owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence) SELECT origin,'lost-key',7,7 FROM assistant_grant_pending WHERE claim_id=?1", /* sqlite-large-value: allow-for-now lv-hdf2d9ab766b4 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_retractions"),
    ] {
        if name == "pending dirty" {
            connection.execute(

                /* sqlite-large-value: allow-for-now lv-h53cef4db0f7e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_pending
                 (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record)
                 VALUES('pending-only',?1,'test','issuer','owner',0,1,'{}')", [SUBJECT],
            ).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-h039f365c6be5 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ insert, []).unwrap();
        } else {

            connection.execute(/* sqlite-large-value: allow-for-now lv-h7e1b0f302faf owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ insert, [&grant.id]).unwrap();
        }
        assert!(subject_pending_cost(&connection, SUBJECT).0, "{name} must refuse the subject");

        connection.execute(/* sqlite-large-value: allow-for-now lv-h02fa000841a2 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ cleanup, []).unwrap();

        connection.execute(/* sqlite-large-value: allow-for-now lv-h344908bb2949 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_pending WHERE claim_id='pending-only'", []).unwrap();
        assert!(!subject_pending_cost(&connection, SUBJECT).0, "{name} must release only when drained");
    }
    let mut costs = Vec::new();
    for size in [10, 10_000] {
        for index in costs.len()*10..size {
            let unrelated = format!("unrelated-{index}");

            connection.execute(/* sqlite-large-value: allow-for-now lv-hf7f6bf4eaa09 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&unrelated]).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-h28f32b27cc2c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start,grant_subject) VALUES(?1,0,0,'',-1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&unrelated]).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-hdd1c68831927 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_recompute(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&unrelated]).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-h1497f73999fe owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&unrelated]).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-hb6acf0c98e87 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key) VALUES(?1,7,'proof','lost-key')", [&unrelated]).unwrap();

            connection.execute(/* sqlite-large-value: allow-for-now lv-h1e267ccdffa4 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence) VALUES(?1,'lost-key',7,7)", [&unrelated]).unwrap();
        }
        let cost = subject_pending_cost(&connection, SUBJECT);
        eprintln!("unrelated queues: size={size} pending={} vm_steps={} fullscan_steps={}",cost.0,cost.1,cost.2);
        assert!(!cost.0);
        assert!(cost.1 <= 1024 + i32::try_from(size).unwrap()*256);
        costs.push(cost);
    }
    assert!(costs[1].1 <= costs[0].1*1000 + 1024,
        "task-outer growth must remain linear in task count, not retained history");
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
    for attempt in 0..2 {
        let before = (
            connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
            connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
        );
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
        if attempt==1 {
            assert_eq!((
                connection.pragma_query_value(None,"schema_version",|row| row.get::<_,i64>(0)).unwrap(),
                connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),
            ),before,"migrated pending obligations must not make a repeated install write");
        }
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
            /* sqlite-large-value: allow-for-now lv-ha297190b829f owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start,grant_subject)
                                     VALUES(?1,0,0,'',-1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated')) ON CONFLICT(claim_id) DO UPDATE SET position=excluded.position",
            [&grant.id],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h31237d3f8f0e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))
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

    connection.execute( /* sqlite-large-value: allow-for-now lv-he1e9fc1883d6 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_dirty(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id]).unwrap();
    let sequence: i64 = connection.query_row(
        "SELECT sequence FROM assistant_grant_queue_owners WHERE queue='dirty' AND owner=?1",
        [&grant.id], |row| row.get(0),
    ).unwrap();
    for key in ["assistant_grant_owner_cursor/dirty", "assistant_grant_owner_upper/dirty"] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hdc1a81f82dc2 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
            params![key,sequence+1],
        ).unwrap();
    }
    let transaction = connection.transaction().unwrap();
    assert!(authority::backfill_slice(&transaction,Duration::from_secs(60)).unwrap());
    assert!(authority::contains(&transaction,&grant.id).unwrap());
    transaction.commit().unwrap();
}

#[test]
fn authenticated_proof_recheck_owner_finishes_despite_lower_and_between_id_arrivals() {
    const BOUND: usize = 64;
    let (source, key) = source();
    let target = node("proof-owner-fairness", Some(key.public()));
    let grant = append(&source, SUBJECT, GRANTED, fields());
    receive(&target, &export(&source, &target));
    let mut connection = target.connection.write();
    assert!(authority::contains(&connection, &grant.id).unwrap());
    let proof: (String,u64,String,i64) = connection.query_row(
        "SELECT auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position
         FROM assistant_grant_pending WHERE claim_id=?1 AND authenticated=1",
        [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    // The victim keeps its real signed claim and evidence. Controlled IDs exist
    // only as stale queue owners whose missing claims should be retired.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-ha1eb7450890c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES('!m/initial','unrelated')", [],
    ).unwrap();
    connection.execute(
         /* sqlite-large-value: allow-for-now lv-hf54effbd7261 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&grant.id],
    ).unwrap();
    assert!(authority::subject_ready(&connection, SUBJECT).is_err());
    let mut lifted = false;
    for slice in 0..BOUND {
        for prefix in ["!a/lower", "!t/between"] {
            let id = format!("{prefix}/{slice:08}");
            assert!(id.as_str()<grant.id.as_str());
            connection.execute(

                 /* sqlite-large-value: allow-for-now lv-h5f26cae60b9a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_rechecks(claim_id,grant_subject) VALUES(?1,coalesce((SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1),'unrelated'))", [&id],
            ).unwrap();
        }
        // Each visit commits independently: neither the owner turn nor the
        // fixed round frontier may live only in a backfill invocation's memory.
        let transaction = connection.transaction().unwrap();
        assert!(!authority::backfill_slice(&transaction, Duration::ZERO).unwrap());
        assert!(transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks
             WHERE claim_id<>?1)", [&grant.id], |row| row.get::<_,bool>(0),
        ).unwrap(), "the victim must finish while unrelated rechecks remain");
        let ready = authority::subject_ready(&transaction, SUBJECT).is_ok();
        transaction.commit().unwrap();
        if ready {
            assert!(authority::contains(&connection, &grant.id).unwrap());
            let rechecked: (String,u64,String,i64) = connection.query_row(
                "SELECT auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position
                 FROM assistant_grant_pending WHERE claim_id=?1 AND authenticated=1",
                [&grant.id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(rechecked, proof, "recheck lost the victim's genuine authentication evidence");
            assert!(!connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
                [&grant.id], |row| row.get::<_,bool>(0),
            ).unwrap(), "the subject fence lifted before its proof obligation retired");
            lifted = true;
            break;
        }
    }
    assert!(lifted, "lower and between-ID arrivals starved the proof victim beyond {BOUND} ZERO slices");
    assert!(!connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id='!m/initial')",
        [], |row| row.get::<_,bool>(0),
    ).unwrap(), "stale missing-claim rechecks must retire normally");
}

const SUBJECT_QUEUES: [&str; 4] = [
    "assistant_grant_dirty", "assistant_grant_auth_cursor",
    "assistant_grant_recompute", "assistant_grant_proof_rechecks",
];

fn insert_subject_obligation(connection: &Connection, queue: &str, id: &str, subject: Option<&str>) {
    let sql = if queue == "assistant_grant_auth_cursor" {
        format!(/* sqlite-large-value: allow-for-now lv-h620ffc9ba4be owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO {queue}
            (claim_id,grant_subject,position,record_rowid,member_key,membership_start)
            VALUES(?1,?2,0,0,'',-1)")
    } else {
        format!(/* sqlite-large-value: allow-for-now lv-hd47ea0a4555d owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO {queue}(claim_id,grant_subject) VALUES(?1,?2)")
    };
    connection.execute(/* sqlite-large-value: allow-for-now lv-h5e1c0148bafe owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &sql, params![id,subject]).unwrap();
}

#[test]
fn subject_schema_v2_records_nullable_queues_and_completed_conversion() {
    let store = node("subject-schema", None);
    drain_authority(&store);
    let connection = store.connection.write();
    for (key, expected) in [
        ("assistant_grant_authority_schema_version", "2"),
        ("assistant_grant_subject_conversion_complete", "2"),
    ] {
        assert_eq!(connection.query_row("SELECT value FROM meta WHERE key=?1",
            [key], |row| row.get::<_,String>(0)).unwrap(), expected);
    }
    for queue in SUBJECT_QUEUES {
        let nullable: i64 = connection.query_row(
            "SELECT \"notnull\" FROM pragma_table_info(?1) WHERE name='grant_subject'",
            [queue], |row| row.get(0),
        ).unwrap();
        assert_eq!(nullable, 0, "{queue} must retain unknown-subject obligations");
        let indexes: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND tbl_name=?1
             AND sql LIKE '%grant_subject%'", [queue], |row| row.get(0),
        ).unwrap();
        assert_eq!(indexes, 2, "{queue} needs subject and partial NULL seeks");
    }
}

#[test]
fn every_null_subject_queue_globally_fences_bearers_candidates_and_owner_mutations() {
    let store = node("null-queue", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let other = independent_grant(&store);
    drain_authority(&store);
    let connection = store.connection.write();
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection, queue, "true-orphan", None);
        assert!(authority::ready(&connection).is_err(), "{queue}");
        for claim in [&grant,&other] {
            assert!(authority::subject_ready(&connection, &claim.subject).is_err(), "{queue}");
            assert!(authority::owner_mutation_ready(&connection, &claim.subject).is_err(), "{queue}");
            assert!(authority::contains(&connection, &claim.id).is_err(), "{queue}");
            assert!(authority::by_credential(&connection,
                claim.body["fields"]["credential_hash"].as_str().unwrap()).is_err(), "{queue}");
        }
        assert!(authority::candidates(&connection, "assistant/chatgpt", OWNER,
            Some(0), None, None).is_err(), "{queue}");
        connection.execute(&format!(/* sqlite-large-value: allow-for-now lv-hed868364988a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {queue} WHERE claim_id='true-orphan'"), []).unwrap();
        authority::ready(&connection).unwrap();
    }
}

#[test]
fn owner_mutation_bypasses_ordinary_backlog_but_never_same_subject_recompute() {
    let store = node("owner-barrier", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    let other = independent_grant(&store);
    drain_authority(&store);
    let connection = store.connection.write();
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection, queue, &grant.id, Some(SUBJECT));
        if queue == "assistant_grant_recompute" {
            assert!(authority::owner_mutation_ready(&connection, SUBJECT).is_err());
        } else {
            authority::owner_mutation_ready(&connection, SUBJECT).unwrap();
        }
        authority::owner_mutation_ready(&connection, &other.subject).unwrap();
        connection.execute(&format!(/* sqlite-large-value: allow-for-now lv-h8a20685df67c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {queue} WHERE claim_id=?1"), [&grant.id]).unwrap();
    }
}

#[test]
fn true_orphan_and_cursor_only_obligations_retire_without_authenticating_history() {
    for queue in SUBJECT_QUEUES {
        let store = node("orphan-recovery", None);
        let grant = append(&store, SUBJECT, GRANTED, fields());
        drain_authority(&store);
        {
            let connection = store.connection.write();
            insert_subject_obligation(&connection, queue, "absent-from-claims-and-pending", None);
            assert!(authority::ready(&connection).is_err());
        }
        for slice in 0..256 {
            let done = store.advance_assistant_grant_authority_backfill_for_test(
                Duration::ZERO, 1).unwrap();
            if done { break; }
            assert!(slice < 255, "{queue} orphan failed to drain in bounded units");
        }
        let connection = store.connection.write();
        authority::ready(&connection).unwrap();
        for remaining in SUBJECT_QUEUES {
            assert_eq!(connection.query_row(
                /* sqlite-large-value: allow-for-now lv-h88d72b40d65c owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {remaining} WHERE claim_id=?1"),
                ["absent-from-claims-and-pending"], |row| row.get::<_,i64>(0)).unwrap(), 0);
        }
        assert!(authority::contains(&connection, &grant.id).unwrap());
        assert_eq!(snapshot(&connection).len(), 1);
    }
}

#[test]
fn returning_claim_during_recompute_defers_without_discarding_any_owned_obligation() {
    let store = node("return-during-recompute", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    drain_authority(&store);
    let connection = store.connection.write();
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection, queue, &grant.id, Some(SUBJECT));
    }
    let before = snapshot(&connection);
    for local in [false,true] {
        let outcome = if local {
            authority::refresh_local_append(&connection, &grant.id).unwrap()
        } else {
            authority::refresh_claim(&connection, &grant.id).unwrap()
        };
        assert_eq!(outcome, authority::RefreshOutcome::Deferred);
        assert_eq!(snapshot(&connection), before);
        for queue in SUBJECT_QUEUES {
            assert_eq!(connection.query_row(
                /* sqlite-large-value: allow-for-now lv-hf984c51af52c owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {queue} WHERE claim_id=?1 AND grant_subject=?2"),
                params![grant.id,SUBJECT], |row| row.get::<_,i64>(0)).unwrap(), 1,
                "{queue} was discharged by a Deferred refresh");
        }
    }
    drop(connection);
    drain_authority(&store);
    assert_reference_and_rebuild(&store);
}

#[test]
fn nonempty_subject_queues_have_flat_vm_cost_across_policy_history_growth() {
    let store = node("subject-vm", None);
    let grant = append(&store, SUBJECT, GRANTED, fields());
    drain_authority(&store);
    let connection = store.connection.write();
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection, queue, "unrelated-work", Some("unrelated-subject"));
    }
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-ha8683a051de9 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key)
         VALUES('unrelated-writer',7,'unrelated-proof','unrelated-key');
         INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence)
         VALUES('unrelated-writer','unrelated-key',0,10000);"
    ).unwrap();
    let mut costs = Vec::new();
    for history in [1_000,10_000] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h4b5f59630df1 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?2)
             INSERT OR IGNORE INTO assistant_grant_pending
             (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record)
             SELECT 'subject-policy/'||i,?1,?3,'subject-vm','human/owner',i,1,'{}' FROM n",
            params![SUBJECT,history,POLICY],
        ).unwrap();
        let cost = subject_pending_cost(&connection, SUBJECT);
        eprintln!("subject-index-policy history={history} pending={} vm={} fullscan={}",
            cost.0,cost.1,cost.2);
        assert!(!cost.0);
        assert_eq!(cost.2,0);
        costs.push(cost);
    }
    assert_eq!(costs[0],costs[1], "unrelated nonempty queues must never walk POLICY history");
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection, queue, &grant.id, Some(SUBJECT));
        assert!(subject_pending_cost(&connection,SUBJECT).0, "{queue} missed its indexed subject");
        connection.execute(&format!(/* sqlite-large-value: allow-for-now lv-h601a163832b0 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {queue} WHERE claim_id=?1"), [&grant.id]).unwrap();
    }
}

#[test]
fn subject_conversion_commits_rowid_frontiers_and_resumes_after_each_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("subject-conversion.sqlite3");
    let expected;
    {
        let store = Store::open(&path, "conversion", Arc::new(Plain)).unwrap();
        for _ in 0..193 {
            append(&store,SUBJECT,GRANTED,fields());
        }
        drain_authority(&store);
        let connection = store.connection.write();
        expected = snapshot(&connection);
        for queue in SUBJECT_QUEUES {
            let columns = if queue == "assistant_grant_auth_cursor" {
                "claim_id,position,record_rowid,member_key,membership_start"
            } else { "claim_id" };
            let select = if queue == "assistant_grant_auth_cursor" {
                "id,0,0,'',-1"
            } else { "id" };
            connection.execute(&format!(
                /* sqlite-large-value: allow-for-now lv-hd8bdd803fbbe owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO {queue}({columns}) SELECT {select} FROM claims WHERE subject=?1"),
                [SUBJECT]).unwrap();
            connection.execute(&format!(/* sqlite-large-value: allow-for-now lv-h19a3bf68d43a owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE {queue} SET grant_subject=NULL"), []).unwrap();
        }
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hf1ba6e7709b9 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='1' WHERE key='assistant_grant_authority_schema_version';
             DELETE FROM meta WHERE key='assistant_grant_subject_conversion_complete'
                 OR key GLOB 'assistant_grant_subject_conversion/*'
                 OR key GLOB 'assistant_grant_subject_conversion_upper/*';"
        ).unwrap();
        authority::install(&connection,"conversion").unwrap();
        assert!(authority::ready(&connection).is_err());
    }
    let mut previous = [0_i64;4];
    let mut saw_frontier = false;
    let mut completed = false;
    for _ in 0..4096 {
        // Closing after every unit makes every cursor a durable recovery boundary.
        let mut connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        let transaction = connection.transaction().unwrap();
        let done = authority::backfill_slice(&transaction,Duration::ZERO).unwrap();
        for (index,queue) in SUBJECT_QUEUES.iter().enumerate() {
            let frontier: Option<String> = transaction.query_row(
                "SELECT value FROM meta WHERE key=?1",
                [format!("assistant_grant_subject_conversion/{queue}")],
                |row| row.get(0)).optional().unwrap();
            if let Some(frontier) = frontier {
                let frontier = frontier.parse::<i64>().unwrap();
                assert!(frontier >= previous[index], "{queue} conversion restarted after reopen");
                previous[index] = frontier;
                saw_frontier |= frontier > 0;
            }
        }
        let conversion_complete: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta
             WHERE key='assistant_grant_subject_conversion_complete' AND value='2')",
            [],|row| row.get(0)).unwrap();
        if !conversion_complete {
            assert!(authority::ready(&transaction).is_err());
        }
        transaction.commit().unwrap();
        if done {
            completed = true;
            assert!(conversion_complete);
            authority::ready(&connection).unwrap();
            assert_eq!(snapshot(&connection),expected);
            break;
        }
    }
    assert!(saw_frontier, "multi-page fixture never persisted a conversion frontier");
    assert!(completed, "conversion/recovery did not drain after 4096 committed one-unit slices");
    let store = Store::open(&path,"conversion",Arc::new(Plain)).unwrap();
    assert_reference_and_rebuild(&store);
}

#[test]
fn future_authority_schema_install_and_refresh_leave_database_bytes_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("future-authority.sqlite3");
    let grant_id;
    {
        let store = Store::open(&path,"future",Arc::new(Plain)).unwrap();
        grant_id = append(&store,SUBJECT,GRANTED,fields()).id;
        drain_authority(&store);
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h0103c24dcda8 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='999' WHERE key='assistant_grant_authority_schema_version'",
            [],
        ).unwrap();
        // Reader-pool connections can outlive Store's scope. Checkpoint setup
        // explicitly so later connection teardown cannot change the baseline.
        let checkpoint: (i64,i64,i64) = store.connection.write().query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(checkpoint,(0,0,0), "future-schema setup WAL must be durable");
    }
    let before = fs::read(&path).unwrap();
    {
        let connection = Connection::open(&path).unwrap();
        projection_digest::register(&connection).unwrap();
        let changes: i64 = connection.query_row("SELECT total_changes()",[],|row| row.get(0)).unwrap();
        // install may refuse directly or leave refusal to ready; neither may write.
        let _ = authority::install(&connection,"future");
        assert!(authority::ready(&connection).is_err());
        assert!(authority::owner_mutation_ready(&connection,SUBJECT).is_err());
        assert!(authority::refresh_claim(&connection,&grant_id).is_err());
        assert!(authority::refresh_local_append(&connection,&grant_id).is_err());
        assert_eq!(connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),changes);
        assert_eq!(connection.query_row(
            "SELECT value FROM meta WHERE key='assistant_grant_authority_schema_version'",
            [], |row| row.get::<_,String>(0)).unwrap(),"999");
    }
    assert_eq!(fs::read(&path).unwrap(),before, "future-schema rejection modified durable database bytes");
}

fn future_authority_contract(connection: &Connection) -> Vec<String> {
    let mut contract = Vec::new();
    for sql in [
        "SELECT json_array(type,name,tbl_name,sql) FROM sqlite_schema
         WHERE name GLOB 'assistant_grant_*' OR name='replica_records_writer_grant_proof_index'
         ORDER BY type,name",
        "SELECT json_array(key,value) FROM meta
         WHERE key IN ('assistant_grant_authority_schema_version',
             'assistant_grant_subject_conversion_complete','assistant_grant_authority_version',
             'assistant_grant_authority_local_origin','assistant_grant_authority_fleet_id')
            OR key GLOB 'assistant_grant_subject_conversion/*'
            OR key GLOB 'assistant_grant_subject_conversion_upper/*'
         ORDER BY key",
    ] {
        contract.extend(connection.prepare(/* sqlite-large-value: allow-for-now lv-h41d63ef7aa81 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ sql).unwrap().query_map([],|row| row.get::<_,String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
    }
    for queue in SUBJECT_QUEUES {
        contract.push(queue.to_owned());
        contract.extend(connection.prepare(
            "SELECT json_array(cid,name,type,\"notnull\",dflt_value,pk)
             FROM pragma_table_info(?1) ORDER BY cid",
        ).unwrap().query_map([queue],|row| row.get::<_,String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
    }
    contract
}

#[test]
fn future_schema_allows_source_writes_and_owns_its_persisted_evidence_triggers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("future-source-writes.sqlite3");
    let (issuer,key) = source();
    let grant = append(&issuer,SUBJECT,GRANTED,fields());
    {
        let target = Store::open(&path,"future-source",Arc::new(Plain)).unwrap();
        target.bind_fleet(FLEET).unwrap();
        target.pin_fleet_anchor(key.public()).unwrap();
        receive(&target,&export(&issuer,&target));
        drain_authority(&target);
        let connection = target.connection.write();
        assert!(authority::contains(&connection,&grant.id).unwrap());
        insert_subject_obligation(&connection,"assistant_grant_auth_cursor",&grant.id,Some(SUBJECT));
        // Declare this installed trigger set to be the future store's contract.
        // The older module may refuse its own operations, not disable stored SQL.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h59a8400962cb owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE meta SET value='999' WHERE key IN
             ('assistant_grant_authority_schema_version','assistant_grant_subject_conversion_complete');"
        ).unwrap();
    }
    let contract_before;
    let derived_before;
    {
        let connection = Connection::open(&path).unwrap();
        contract_before = future_authority_contract(&connection);
        derived_before = snapshot(&connection);
    }
    let generic_id;
    {
        let target = Store::open(&path,"future-source",Arc::new(Plain)).unwrap();
        generic_id = append(&target,"custom/future/unrelated","custom.future.observed",
            json!({"value":"source write remains permitted"})).id;
        let connection = target.connection.write();
        assert_eq!(future_authority_contract(&connection),contract_before);
        for (trigger,declared_write) in [
            ("assistant_grant_proof_signature_UPDATE",/* sqlite-large-value: allow-for-now lv-hbf34f88e02b6 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_invalidations"),
            ("assistant_grant_proof_envelope_UPDATE",/* sqlite-large-value: allow-for-now lv-hfe310aa5cefe owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_proof_invalidations"),
            ("assistant_grant_proof_record_UPDATE",/* sqlite-large-value: allow-for-now lv-h065584da52be owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks"),
            ("assistant_grant_proof_rechecks_subject_insert",/* sqlite-large-value: allow-for-now lv-hebcff07d594e owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_proof_rechecks SET grant_subject"),
        ] {
            let sql: String = connection.query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                [trigger],|row| row.get(0),
            ).unwrap();
            assert!(sql.contains(declared_write), "future fixture must explicitly declare {trigger}");
        }
        let (writer,sequence,hash,member,position): (String,i64,String,String,i64) = connection.query_row(
            "SELECT origin,auth_sequence,auth_envelope_hash,auth_member_key,auth_record_position
             FROM assistant_grant_pending WHERE claim_id=?1",
            [&grant.id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
        let signature: String = connection.query_row(
            "SELECT signature FROM replica_envelope_signatures
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
            params![writer,sequence,hash,member],|row| row.get(0),
        ).unwrap();
        let envelope_state: String = connection.query_row(
            "SELECT receipt_state FROM replica_envelopes
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
            params![writer,sequence,hash],|row| row.get(0),
        ).unwrap();
        let invalidations = || connection.query_row(
            "SELECT count(*) FROM assistant_grant_proof_invalidations",
            [],|row| row.get::<_,i64>(0),
        ).unwrap();
        let initial_invalidations = invalidations();
        assert_eq!(connection.execute(
            /* sqlite-large-value: allow-for-now lv-h819cb9fc584b owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET signature='invalid-future-source-proof'
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
            params![writer,sequence,hash,member],
        ).unwrap(),1);
        assert_eq!(invalidations(),initial_invalidations+1);
        assert_eq!(connection.execute(
            /* sqlite-large-value: allow-for-now lv-he21cc3755f07 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state='pending'
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
            params![writer,sequence,hash],
        ).unwrap(),1);
        assert_eq!(invalidations(),initial_invalidations+2);
        assert_eq!(connection.execute(
            /* sqlite-large-value: allow-for-now lv-h6d333292beb1 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='invalid'
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4 AND claim_id=?5",
            params![writer,sequence,hash,position,grant.id],
        ).unwrap(),1);
        assert_eq!(connection.query_row(
            "SELECT grant_subject FROM assistant_grant_proof_rechecks WHERE claim_id=?1",
            [&grant.id],|row| row.get::<_,String>(0)).unwrap(),SUBJECT);
        assert_eq!(connection.query_row(
            "SELECT count(*) FROM assistant_grant_auth_cursor WHERE claim_id=?1",
            [&grant.id],|row| row.get::<_,i64>(0)).unwrap(),0);
        // Restore the same cryptographically signed evidence, through ordinary
        // source writes whose persistent trigger effects remain in force.
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-he1003000aef0 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelope_signatures SET signature=?5
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND member_key=?4",
            params![writer,sequence,hash,member,signature],
        ).unwrap();
        assert_eq!(invalidations(),initial_invalidations+3);
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h6eaefd243961 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state=?4
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
            params![writer,sequence,hash,envelope_state],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hfac8e5b6624c owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='valid'
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3 AND position=?4 AND claim_id=?5",
            params![writer,sequence,hash,position,grant.id],
        ).unwrap();
        assert_eq!(future_authority_contract(&connection),contract_before);
        assert_eq!(snapshot(&connection),derived_before);
        assert_eq!(connection.query_row(
            "SELECT grant_subject FROM assistant_grant_pending WHERE claim_id=?1",
            [&grant.id],|row| row.get::<_,String>(0)).unwrap(),SUBJECT);
        assert!(authority::ready(&connection).is_err());
        assert!(authority::contains(&connection,&grant.id).is_err());
        assert!(authority::by_credential(&connection,&"a".repeat(64)).is_err());
        assert!(authority::candidates(&connection,"assistant/chatgpt",OWNER,Some(0),None,None).is_err());
        let changes: i64 = connection.query_row("SELECT total_changes()",[],|row| row.get(0)).unwrap();
        let _ = authority::install(&connection,"future-source");
        assert!(authority::refresh_claim(&connection,&grant.id).is_err());
        assert!(authority::refresh_local_append(&connection,&grant.id).is_err());
        assert!(authority::owner_mutation_ready(&connection,SUBJECT).is_err());
        let _ = authority::backfill_slice(&connection,Duration::from_millis(100));
        assert_eq!(connection.query_row("SELECT total_changes()",[],|row| row.get::<_,i64>(0)).unwrap(),changes,
            "explicit current-module authority operations wrote into future-owned state");
    }
    // Ordinary source commits change database bytes; exact authority DDL, marker,
    // and queue-column bytes, rather than the whole file, are the preservation boundary.
    let connection = Connection::open(&path).unwrap();
    assert_eq!(future_authority_contract(&connection),contract_before);
    assert_eq!(snapshot(&connection),derived_before);
    assert!(connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)",
        [&generic_id],|row| row.get::<_,bool>(0)).unwrap());
}

#[test]
fn every_current_queue_insert_site_resolves_claim_and_pending_subjects() {
    let store = node("insert-sites",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    let connection = store.connection.write();
    for queue in SUBJECT_QUEUES {
        // Persistent compatibility triggers must recover both sources without a UDF.
        insert_subject_obligation(&connection,queue,&grant.id,None);
        assert_eq!(connection.query_row(
            /* sqlite-large-value: allow-for-now lv-h3dfd8e94359a owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT grant_subject FROM {queue} WHERE claim_id=?1"),
            [&grant.id], |row| row.get::<_,String>(0)).unwrap(),SUBJECT);
        connection.execute(&format!(/* sqlite-large-value: allow-for-now lv-h541fae28228d owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {queue} WHERE claim_id=?1"),[&grant.id]).unwrap();
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h3536e4e7ca73 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_pending
         (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record)
         VALUES('pending-only-source',?1,?2,'remote','owner',0,0,'{}')",
        params![SUBJECT,POLICY],
    ).unwrap();
    for queue in SUBJECT_QUEUES {
        insert_subject_obligation(&connection,queue,"pending-only-source",None);
        assert_eq!(connection.query_row(
            /* sqlite-large-value: allow-for-now lv-h215320867f9a owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT grant_subject FROM {queue} WHERE claim_id='pending-only-source'"),
            [], |row| row.get::<_,String>(0)).unwrap(),SUBJECT);
    }
}



/* sqlite-large-value: allow-for-now lv-h70e364d23629 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ include!("assistant_grant_receipt_cost_tests.rs");
/* sqlite-large-value: allow-for-now lv-h5d5b16b3dd2b owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ include!("assistant_grant_attachment_tests.rs");

#[path = "assistant_grant_membership_property_tests.rs"]
mod membership_property_tests;

#[test]
fn retraction_window_complement_preserves_endpoints_open_ends_and_full_tuple_frontier() {
    let store = node("window-complement",None);
    drain_authority(&store);
    let connection = store.connection.write();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8837937145e2 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions
         (writer,member_key,lower_sequence,upper_sequence,sequence,claim_id)
         VALUES('window-writer','window-key',0,20000,-1,'')", [],
    ).unwrap();
    let mut costs = Vec::new();
    let mut proof_costs = [Vec::new(),Vec::new()];
    for count in [1_000,10_000] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h9b4a654d4225 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
             INSERT OR IGNORE INTO assistant_grant_pending
             (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record,
              auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position)
             SELECT printf('covered/%08d',i),?2,?3,'window-writer','owner',i,1,'{}',
                    'window-key',i,'proof',0 FROM n",
            params![count,SUBJECT,POLICY],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-ha044e419d33f owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO assistant_grant_membership_windows(writer,member_key,start,end)
             VALUES('window-writer','window-key',1,10000)",[],
        ).unwrap();
        let cost = subject_pending_cost(&connection,SUBJECT);
        eprintln!("subject-index-arm=retraction-covered history={count} pending={} vm={} fullscan={}",
            cost.0,cost.1,cost.2);
        assert!(!cost.0, "inclusive membership endpoints are covered");
        costs.push(cost);
        for (arm,key,index) in [("signature","window-key",0),("envelope","",1)] {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h408db030dce6 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_proof_invalidations
                 (writer,sequence,envelope_hash,member_key,claim_id)
                 VALUES('window-writer',10000,'proof',?1,'zz-completed-frontier')",
                [key],
            ).unwrap();
            let proof_cost = subject_pending_cost(&connection,SUBJECT);
            assert!(!proof_cost.0, "{arm} ignored its claim frontier");
            assert_eq!(proof_cost.2,0);
            eprintln!("subject-index-arm={arm} history={count} pending={} vm={} fullscan={}",
                proof_cost.0,proof_cost.1,proof_cost.2);
            proof_costs[index].push(proof_cost);
            connection.execute(/* sqlite-large-value: allow-for-now lv-h6aa3adc99987 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_invalidations",[]).unwrap();
        }
    }
    assert_eq!(costs[0],costs[1], "covered 10k history must be subtracted before pending seeks");
    for (arm,arm_costs) in ["signature","envelope"].into_iter().zip(proof_costs) {
        assert_eq!(arm_costs[0],arm_costs[1], "{arm} probe walked completed POLICY history");
    }
    for (start,end,expected) in [
        (1,Some(10_000),false), (2,Some(10_000),true),
        (1,Some(9_999),true), (1,None,false), (2,None,true),
    ] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-he0569ce4aae5 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_windows SET start=?1,end=?2
             WHERE writer='window-writer' AND member_key='window-key'",
            params![start,end],
        ).unwrap();
        assert_eq!(subject_pending_cost(&connection,SUBJECT).0,expected,
            "window start={start} end={end:?}");
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h774e2911061d owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_membership_windows WHERE writer='window-writer'",[]).unwrap();
    for (sequence,id,expected) in [
        (9_999,"~",true), (10_000,"covered/00010000",false),
        (10_000,"covered/00009999",true), (10_001,"",false),
    ] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hb937c9a3ffeb owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_retractions SET sequence=?1,claim_id=?2
             WHERE writer='window-writer'",params![sequence,id],
        ).unwrap();
        assert_eq!(subject_pending_cost(&connection,SUBJECT).0,expected,
            "full tuple frontier ({sequence},{id})");
    }
    // Splitting the same task range must preserve complement semantics.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h52d115707895 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_retractions SET lower_sequence=1,upper_sequence=5000,sequence=-1,claim_id=''",
        [],
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h91e0a7e98d1c owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence)
         VALUES('window-writer','window-key',5001,10000)",[],
    ).unwrap();
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h3b3df5552112 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows(writer,member_key,start,end)
         VALUES('window-writer','window-key',1,10000)",[],
    ).unwrap();
    assert!(!subject_pending_cost(&connection,SUBJECT).0);
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h32589fbc2ae4 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_windows SET end=9999 WHERE writer='window-writer'",[],
    ).unwrap();
    assert!(subject_pending_cost(&connection,SUBJECT).0);
}

#[test]
fn orphan_base_cleanup_cancels_own_work_preserves_other_witnesses_and_matches_fresh_fold() {
    let store = node("orphan-base-cleanup",None);
    let grants = attachment_append_group(&store,true,9);
    drain_authority(&store);
    let mut policy_fields = fields();
    policy_fields["expires_at_unix_ms"] = json!(20_000);
    let policy = append(&store,SUBJECT,POLICY,policy_fields);
    {
        let connection = store.connection.write();
        let witnesses: Vec<String> = connection.prepare(
            "SELECT base_claim_id FROM assistant_grant_attachment_witnesses ORDER BY base_claim_id",
        ).unwrap().query_map([],|row| row.get(0)).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert!(witnesses.len()>1, "fixture needs genuine other-base conflict witnesses");
        connection.execute(/* sqlite-large-value: allow-for-now lv-h74662fb4771e owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1",[&grants[0].id]).unwrap();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h589f405bf262 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_pending WHERE claim_id=?1",[&grants[0].id]).unwrap();
        // Cursor-only orphan: raw and pending sources are both absent, but real
        // authority pairs and witnesses still prove cleanup cannot be skipped.
        connection.execute(/* sqlite-large-value: allow-for-now lv-he62394538573 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1",[&grants[0].id]).unwrap();
        insert_subject_obligation(&connection,"assistant_grant_auth_cursor",&grants[0].id,None);
        assert!(authority::ready(&connection).is_err());
        for base in witnesses.iter().filter(|id| id.as_str()!=grants[0].id.as_str()) {
            assert!(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachment_witnesses WHERE base_claim_id=?1)",
                [base],|row| row.get::<_,bool>(0)).unwrap());
        }
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE claim_id=?1)",
            [&policy.id],|row| row.get::<_,bool>(0)).unwrap(),
            "other owner's bounded attachment work must survive orphan admission");
    }
    assert!(!store.advance_assistant_grant_authority_backfill_for_test(Duration::ZERO,1).unwrap());
    let other_witnesses: i64 = store.readers.get().query_row(
        "SELECT count(*) FROM assistant_grant_attachment_witnesses WHERE base_claim_id<>?1",
        [&grants[0].id],|row| row.get(0),
    ).unwrap();
    assert!(other_witnesses>0, "one orphan recovery unit erased other-base conflict witnesses");
    drain_authority(&store);
    let connection = store.connection.write();
    for (table,predicate) in [
        ("assistant_grant_authority","claim_id=?1 OR base_claim_id=?1"),
        ("assistant_grant_effective","claim_id=?1"),
        ("assistant_grant_attachments","claim_id=?1"),
        ("assistant_grant_attachment_witnesses","base_claim_id=?1"),
    ] {
        assert_eq!(connection.query_row(
            /* sqlite-large-value: allow-for-now lv-h404f0b8b2d79 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {table} WHERE {predicate}"),
            [&grants[0].id],|row| row.get::<_,i64>(0)).unwrap(),0,
            "orphan retained own derived rows in {table}");
    }
    drop(connection);
    assert_reference_and_rebuild(&store);
}

#[test]
fn conversion_marker_alone_fences_reads_until_durable_completion() {
    let store = node("conversion-marker-gate",None);
    let grant = append(&store,SUBJECT,GRANTED,fields());
    drain_authority(&store);
    {
        let connection = store.connection.write();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-ha2da57c725cc owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='assistant_grant_subject_conversion_complete'",[],
        ).unwrap();
        for queue in SUBJECT_QUEUES {
            assert_eq!(connection.query_row(
                /* sqlite-large-value: allow-for-now lv-h0418fb3d4af6 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!("SELECT count(*) FROM {queue}"),[],
                |row| row.get::<_,i64>(0)).unwrap(),0);
        }
        assert!(authority::ready(&connection).is_err());
        assert!(authority::contains(&connection,&grant.id).is_err());
        assert!(authority::maintenance_pending(&connection).unwrap());
    }
    drain_authority(&store);
    assert!(authority::contains(&store.readers.get(),&grant.id).unwrap());
}

#[test]
fn restored_subject_queue_database_recovers_then_replication_replay_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.sqlite3");
    let restored = directory.path().join("restored.sqlite3");
    let (issuer,key) = source();
    let grant = append(&issuer,SUBJECT,GRANTED,fields());
    let exchange;
    let expected;
    {
        let target = Store::open(&original,"restore-reader",Arc::new(Plain)).unwrap();
        target.bind_fleet(FLEET).unwrap();
        target.pin_fleet_anchor(key.public()).unwrap();
        exchange = export(&issuer,&target);
        receive(&target,&exchange);
        drain_authority(&target);
        let connection = target.connection.write();
        expected = snapshot(&connection);
        // Backup a real signed store with outstanding persisted legacy work.
        insert_subject_obligation(&connection,"assistant_grant_proof_rechecks",&grant.id,Some(SUBJECT));
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h10673eb869ec owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_proof_rechecks SET grant_subject=NULL WHERE claim_id=?1",
            [&grant.id],
        ).unwrap();
        // Store teardown alone need not close every pooled reader connection.
        let checkpoint: (i64,i64,i64) = connection.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(checkpoint,(0,0,0), "restore source WAL must be durable");
    }
    // Setup was explicitly checkpointed before copying the main file; this is a
    // restore test, not an older-binary substitute.
    fs::copy(&original,&restored).unwrap();
    let target = Store::open(&restored,"restore-reader",Arc::new(Plain)).unwrap();
    drain_authority(&target);
    assert_eq!(snapshot(&target.readers.get()),expected);
    receive(&target,&exchange);
    drain_authority(&target);
    assert_eq!(snapshot(&target.readers.get()),expected);
    assert!(authority::contains(&target.readers.get(),&grant.id).unwrap());
    assert_reference_and_rebuild(&target);
}

fn assert_obligation_subjects_match_real_sources(store: &Store) {
    let connection = store.readers.get();
    for queue in SUBJECT_QUEUES {
        let mismatched: i64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-hd3c69c086b61 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!(
            "SELECT count(*) FROM {queue} AS work
             LEFT JOIN claims ON claims.id=work.claim_id
             LEFT JOIN assistant_grant_pending AS pending ON pending.claim_id=work.claim_id
             WHERE coalesce(claims.subject,pending.grant_subject) IS NOT NULL
               AND work.grant_subject IS NOT coalesce(claims.subject,pending.grant_subject)"),
            [],|row| row.get(0),
        ).unwrap();
        assert_eq!(mismatched,0, "{queue} insert/update site lost a known subject");
    }
}

#[test]
fn real_append_receipt_signature_recheck_and_trim_paths_keep_queue_subjects_complete() {
    let (issuer,key) = source();
    let target = node("insert-site-receipt",Some(key.public()));
    receive(&target,&export(&issuer,&target));
    for kind in [GRANTED,POLICY,REVOKED] {
        let claim = append(&issuer,SUBJECT,kind,fields());
        assert_obligation_subjects_match_real_sources(&issuer);
        let exchange = export(&issuer,&target);
        receive_pre_projection_unsigned_history(&target,&exchange);
        assert_obligation_subjects_match_real_sources(&target);
        receive(&target,&signatures(&exchange));
        assert_obligation_subjects_match_real_sources(&target);
        {
            let connection = target.connection.write();
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h7f899e25a368 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelope_signatures
                 WHERE (writer,sequence,envelope_hash) IN
                     (SELECT writer,sequence,envelope_hash FROM replica_records WHERE claim_id=?1)",
                [&claim.id],
            ).unwrap();
        }
        assert_obligation_subjects_match_real_sources(&target);
        receive(&target,&signatures(&exchange));
        drain_authority(&target);
        assert_obligation_subjects_match_real_sources(&target);
    }
    let removed = append(&issuer,SUBJECT,POLICY,fields());
    {
        let connection = issuer.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h295f134bcb86 owner=smallclaims::store::assistant_grant_authority_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1",[&removed.id]).unwrap();
        authority::refresh_claim(&connection,&removed.id).unwrap();
    }
    assert_obligation_subjects_match_real_sources(&issuer);
    drain_authority(&issuer);
    assert_obligation_subjects_match_real_sources(&issuer);
    assert_reference_and_rebuild(&issuer);
}

#[test]
fn retraction_complement_handles_i64_max_without_real_number_promotion() {
    let store = node("window-integer-boundary",None);
    drain_authority(&store);
    let connection = store.connection.write();
    let maximum = i64::MAX;
    for (id,sequence) in [("max/a",maximum-1),("max/b",maximum),("max/c",maximum)] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h743aad7088a2 owner=smallclaims::store::assistant_grant_authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_pending
             (claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record,
              auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position)
             VALUES(?1,?2,?3,'max-writer','owner',?4,1,'{}','max-key',?4,'proof',0)",
            params![id,SUBJECT,POLICY,sequence],
        ).unwrap();
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h4b9fc9d64616 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions
         (writer,member_key,lower_sequence,upper_sequence,sequence,claim_id)
         VALUES('max-writer','max-key',?1,?2,?1,'')",
        params![maximum-1,maximum],
    ).unwrap();
    for (start,end,frontier,id,expected) in [
        (maximum-1,Some(maximum),maximum-1,"",false),
        (maximum,Some(maximum),maximum-1,"",true),
        (maximum,Some(maximum),maximum-1,"max/a",false),
        (maximum-1,Some(maximum-1),maximum-1,"max/a",true),
        (maximum-1,Some(maximum-1),maximum,"max/b",true),
        (maximum-1,Some(maximum-1),maximum,"max/c",false),
        (maximum-1,None,maximum,"",false),
    ] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hc6f1c2de6597 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO assistant_grant_membership_windows(writer,member_key,start,end)
             VALUES('max-writer','max-key',?1,?2)", params![start,end],
        ).unwrap();
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h0604a338c812 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_retractions SET sequence=?1,claim_id=?2
             WHERE writer='max-writer'",params![frontier,id],
        ).unwrap();
        assert_eq!(subject_pending_cost(&connection,SUBJECT).0,expected,
            "i64 boundary start={start} end={end:?} frontier=({frontier},{id})");
    }
}

/// Invoked by scripts/assistant-grant-authority-cross-version with an already
/// built new libtest executable. The old executable is never simulated here.
#[test]
#[ignore = "cross-version harness hook; requires AUTHORITY_CROSS_VERSION_ROOT and PHASE"]
fn assistant_grant_authority_cross_version_fixture() {
    let root = PathBuf::from(std::env::var_os("AUTHORITY_CROSS_VERSION_ROOT").unwrap());
    let path = root.join("store/claims.sqlite3");
    let phase = std::env::var("AUTHORITY_CROSS_VERSION_PHASE").unwrap();
    if phase == "init" {
        assert!(!path.exists(), "cross-version init requires a fresh fixture");
        let state = root.join("state");
        let founded = crate::fleet::join::found(&state,"authority-rollback",
            &crate::fleet::join::MemberSettings {
                mode:crate::fleet::FleetMode::DialOut,
                transports:Some(Vec::new()),..Default::default()
            },
        ).unwrap();
        let fleet = crate::fleet::FleetFile::load(&state).unwrap().unwrap();
        assert_eq!(fleet.anchor_key.as_deref(),Some(founded.anchor_key.as_str()));
    } else if phase == "setup" {
        // The real new daemon initializes the store and its writer identity.
        // Never synthesize daemon-owned metadata to bypass startup admission.
        assert!(path.exists(), "the new daemon must initialize the fixture first");
        assert!(root.join("state/node-identity.json").exists());
        let store = Store::open(&path,"authority-rollback",Arc::new(Plain)).unwrap();
        let state = root.join("state");
        let fleet = crate::fleet::FleetFile::load(&state).unwrap().unwrap();
        crate::fleet::activate(&store,&state,&fleet).unwrap();
        let mut value = fields();
        value["issuer"] = json!("person/owner");
        value["expires_at_unix_ms"] = json!(4_102_444_800_000_u64);
        value["scopes"] = json!([
            "read.assistant.summary","read.assistant.attention",
            "read.assistant.missions","read.assistant.agents",
        ]);
        let grant = store.append_claim(&ClaimInput {
            subject:SUBJECT.into(),kind:GRANTED.into(),actor:Some("person/owner".into()),
            fields:serde_json::from_value(value).unwrap(),evidence:Vec::new(),
            expected_subject:None,idempotency_key:None,
        }).unwrap();
        store.seal_local_batches().unwrap();
        // Require the base to survive without the new binary's local-append
        // exception: genuine signed anchor/member history is the trust source.
        {
            let connection = store.connection.write();
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hee912ae08c9f owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */
                "DELETE FROM assistant_grant_local_appends WHERE claim_id=?1",
                [&grant.id],
            ).unwrap();
            // An ordinary refresh preserves an already admitted local receipt.
            // Own a proof recheck so maintenance replaces that receipt with the
            // sealed member signature before the old binary opens the store.
            insert_subject_obligation(&connection,"assistant_grant_proof_rechecks",
                &grant.id,Some(SUBJECT));
            authority::refresh_claim(&connection,&grant.id).unwrap();
        }
        drain_authority(&store);
        assert!(store.claim_has_authenticated_member_writer(&grant.id).unwrap());
        let connection = store.connection.write();
        for queue in SUBJECT_QUEUES {
            insert_subject_obligation(&connection,queue,
                &format!("cross-version-orphan/{queue}"),None);
        }
        assert!(authority::ready(&connection).is_err());
        assert!(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_pending WHERE authenticated=1 AND auth_member_key IS NOT NULL)",
            [],|row| row.get::<_,bool>(0)).unwrap());
    } else if phase == "assert-old" || phase == "assert-new" {
        {
            // Observe old-build output without letting new Store::open repair it.
            let connection = Connection::open(&path).unwrap();
            for key in ["assistant_grant_authority_schema_version","assistant_grant_subject_conversion_complete"] {
                assert_eq!(connection.query_row("SELECT value FROM meta WHERE key=?1",
                    [key],|row| row.get::<_,String>(0)).unwrap(),"2",
                    "old binary rewrote converted schema marker {key}");
            }
            for queue in SUBJECT_QUEUES {
                let invalid: i64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-hf5d34d18039e owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!(
                    "SELECT count(*) FROM {queue} AS work
                     LEFT JOIN claims ON claims.id=work.claim_id
                     LEFT JOIN assistant_grant_pending AS pending ON pending.claim_id=work.claim_id
                     WHERE coalesce(claims.subject,pending.grant_subject) IS NOT NULL
                       AND work.grant_subject IS NOT coalesce(claims.subject,pending.grant_subject)"),
                    [],|row| row.get(0),
                ).unwrap();
                assert_eq!(invalid,0,"old-build write lost known subjects in {queue}");
                let invented: i64 = connection.query_row(/* sqlite-large-value: allow-for-now lv-h3b4fb9ff0217 owner=smallclaims::store::assistant_grant_authority_tests TODO=trace-statement-and-value-representation */ &format!(
                    "SELECT count(*) FROM {queue}
                     WHERE claim_id GLOB 'cross-version-orphan/*' AND grant_subject IS NOT NULL"),
                    [],|row| row.get(0),
                ).unwrap();
                assert_eq!(invented,0,"old binary invented an orphan subject");
            }
            assert!(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind=?2)",
                params![SUBJECT,REVOKED],|row| row.get::<_,bool>(0)).unwrap(),
                "actual old binary did not append the native grant revocation");
            assert!(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject='custom/rollback/old-write'
                 AND kind='custom.rollback.observed')",[],|row| row.get::<_,bool>(0)).unwrap());
            let fleet = crate::fleet::FleetFile::load(&root.join("state")).unwrap().unwrap();
            let shard = fleet_meta(&connection,"shard_id").unwrap();
            let signed_revoke: Option<(String,u64,String,String,String)> = connection.query_row(
                "SELECT records.writer,records.sequence,records.envelope_hash,
                    signatures.member_key,signatures.signature
                 FROM claims
                 JOIN replica_records AS records ON records.claim_id=claims.id
                 JOIN replica_envelopes AS envelopes ON envelopes.writer=records.writer
                    AND envelopes.sequence=records.sequence AND envelopes.envelope_hash=records.envelope_hash
                 JOIN replica_envelope_signatures AS signatures ON signatures.writer=records.writer
                    AND signatures.sequence=records.sequence AND signatures.envelope_hash=records.envelope_hash
                 JOIN assistant_grant_membership_windows AS windows ON windows.writer=records.writer
                    AND windows.member_key=signatures.member_key
                    AND records.sequence>=windows.start
                    AND (windows.end IS NULL OR records.sequence<=windows.end)
                 WHERE claims.subject=?1 AND claims.kind=?2 AND records.state='valid'
                    AND envelopes.receipt_state IN ('validated','degraded')
                 ORDER BY claims.store_index DESC LIMIT 1",
                params![SUBJECT,REVOKED],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).optional().unwrap();
            let (writer,sequence,hash,member,signature) = signed_revoke
                .expect("old native revoke lacks real sealed member evidence");
            assert_eq!(fleet.anchor_key.as_deref(),Some(member.as_str()));
            let message = crate::fleet::envelope_signature_message_in(
                shard.as_deref(),&fleet.fleet_id,&writer,sequence,&hash);
            assert!(crate::fleet::verify_signature(&member,&message,&signature),
                "old native revoke signature failed cryptographic verification");
        }
        if phase == "assert-new" {
            let store = Store::open(&path,"authority-rollback",Arc::new(Plain)).unwrap();
            drain_authority(&store);
            authority::ready(&store.readers.get()).unwrap();
            assert_obligation_subjects_match_real_sources(&store);
            assert!(authority::latest(&store.readers.get(),SUBJECT,REVOKED).unwrap().is_some());
            let revoke = authority::latest(&store.readers.get(),SUBJECT,REVOKED).unwrap().unwrap();
            assert!(store.claim_has_authenticated_member_writer(&revoke.id).unwrap(),
                "old append must authenticate through genuine signed membership after re-upgrade");
            assert_reference_and_rebuild(&store);
        }
    } else {
        panic!("unknown AUTHORITY_CROSS_VERSION_PHASE: {phase}");
    }
    println!("authority_cross_version phase={phase} schema=2");
}
