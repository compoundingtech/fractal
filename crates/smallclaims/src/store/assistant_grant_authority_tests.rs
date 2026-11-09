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
        .unwrap().into_iter().map(|claim| claim.id).collect()
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
        assert_reference_and_rebuild(&target);
        assert!(authority::contains(&target.readers.get(), &grant.id).unwrap());
        assert_eq!(reference(&target), vec![(grant.id, format!("{:020}", 20000), true)]);
        assert!(decisions(&target, 0).is_empty());
    }
}

fn reset_projection(connection: &Connection) {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h9fa3ab767307 owner=smallclaims::store::assistant_grant_authority_tests TODO=bound-or-reference-values-and-reclamation */
        "DELETE FROM assistant_grant_effective;
         DELETE FROM assistant_grant_authority;
         DELETE FROM assistant_grant_pending;
         DELETE FROM assistant_grant_membership_windows;
         DELETE FROM assistant_grant_dirty;
         DELETE FROM meta WHERE key='assistant_grant_authority_version'
            OR key='assistant_grant_authority_membership_generation'
            OR key LIKE 'assistant_grant_authority_cursor/%';"
    ).unwrap();
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
        receive(&target, &unsigned(&unsigned_rows));
        let after = transition_cost(&target, &grant);
        assert_eq!(after, before, "same-origin flood changed selected authority or request VM/read cost at {count}");
        assert_eq!(candidate_cost(&target), candidate_before, "candidate request cost grew at {count}");
        assert_eq!(decisions(&target, 15000), vec![grant.id.clone()]);
        assert_reference_and_rebuild(&target);
    }
}
