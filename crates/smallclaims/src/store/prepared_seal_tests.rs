//! Sealing prepares each chunk before it takes the writer. These controls seal real batches
//! (plain notes, notes that name a stored blob or a missing one, claims signed before sealing
//! or by it) through sealing and through a frozen copy of the sealing functions as they were
//! before preparation (main at 5bf3105b9), and compare every stored row. Then they change the
//! store between preparation and the writer: a blob stored or dropped, a claim signature that
//! arrives, the member key, the fleet id or the keyring replaced, a batch sealed by another pass.
use super::runtime::Plain;
use super::*;
use crate::fleet::{MemberKey, envelope_signature_message, verify_signature};
use crate::principal::ClaimSignature;
use std::cell::Cell;

const FLEET: &str = "3e7a9c41-5d2b-4f86-b0c3-9a1e6d4f2b58";
const OTHER_FLEET: &str = "c2b5e8f1-7a3d-4c69-9e04-1f6b3a8d5c27";

fn node(name: &str) -> Arc<Store> {
    let store = Arc::new(Store::open_memory(name, Arc::new(Plain)).unwrap());
    store.bind_fleet(FLEET).unwrap();
    store
}

/// Give `store` a member key, which is also its node key: sealing signs claims and envelopes.
fn keyed(store: &Store) -> Arc<MemberKey> {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.load_member_key(Some(key.clone())).unwrap();
    key
}

fn note(store: &Store, subject: &str, fields: BTreeMap<String, Value>) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "example.note".into(),
            actor: None,
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

fn naming(hash: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([("hash".into(), Value::String(hash.into()))])
}

fn unsealed(claim: &ClaimRecord) -> principals::Unsealed<'_> {
    principals::Unsealed {
        subject: &claim.subject,
        kind: &claim.kind,
        actor: claim.actor.as_deref(),
        body: &claim.body,
        accepted_at_unix_ms: claim.accepted_at_unix_ms,
    }
}

fn claims(connection: &Connection) -> Vec<ClaimRecord> {
    connection
        .prepare(
            "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
             FROM claims ORDER BY store_index",
        )
        .unwrap()
        .query_map([], claim_from_row)
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// Run `change` once, after sealing prepares its next chunk and before it takes the writer.
fn before_the_loan(change: impl FnOnce(&Store) + 'static) {
    SEAL_BEFORE_LOAN.with(|hook| *hook.borrow_mut() = Some(Box::new(change)));
}

/// Seal everything written so far, and return how many chunks were prepared again.
fn seal(store: &Store) -> usize {
    SEALS_PREPARED_AGAIN.with(|count| count.set(0));
    store.seal_local_batches().unwrap();
    SEALS_PREPARED_AGAIN.with(Cell::get)
}

/// Run the sealing passes as they ran before preparation, chunk by chunk, in a transaction
/// that is rolled back, and return what it stored, read with `read`.
fn sealed_by_the_old_passes<T>(store: &Store, read: impl FnOnce(&Connection) -> T) -> T {
    let target = max_batch_rowid(&store.readers.get()).unwrap();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let mut seeded_through = store.seeded_batch_rowid.load(Ordering::Acquire);
    while seeded_through < target {
        let through: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(rowid), ?2) FROM (
                    SELECT rowid FROM batches WHERE rowid>?1 AND rowid<=?2
                    ORDER BY rowid LIMIT ?3)",
                params![seeded_through, target, 64],
                |row| row.get(0),
            )
            .unwrap();
        legacy_seed_replica_envelopes_signed_tx(
            &transaction,
            &store.origin,
            Some(seeded_through),
            Some(through),
            Some(&|claim: &principals::Unsealed<'_>| store.sign_unsealed(claim)),
        )
        .unwrap();
        legacy_sign_own_envelopes_range_tx(
            store,
            &transaction,
            Some(seeded_through),
            Some(through),
        )
        .unwrap();
        seeded_through = through;
    }
    read(&*transaction)
}

/// Every row sealing writes, in key order, without the time a signature was stored.
fn sealed_rows(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "SELECT writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms, payload,
                batch_id, relay, receipt_state, received_at_unix_ms
         FROM replica_envelopes ORDER BY writer, sequence, envelope_hash",
        "SELECT record_ref, writer, sequence, envelope_hash, position, raw, state, claim_id,
                subject_hint, kind_hint, updated_at_unix_ms
         FROM replica_records ORDER BY record_ref",
        "SELECT writer, sequence, envelope_hash, member_key, signature
         FROM replica_envelope_signatures ORDER BY writer, sequence, envelope_hash, member_key",
        "SELECT claim_id, signer, key, nonce, signature FROM claim_signatures ORDER BY claim_id",
        "SELECT claim_id FROM claim_verdict_fresh ORDER BY claim_id",
    ]
    .iter()
    .map(|sql| {
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
    .collect()
}

/// Each stored claim signature without its nonce and signature bytes, which every signing
/// makes afresh.
fn signers(connection: &Connection) -> Vec<(String, ClaimSignature)> {
    let mut statement = connection
        .prepare("SELECT claim_id, signature FROM claim_signatures ORDER BY claim_id")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| {
            let (claim_id, text) = row.unwrap();
            let mut signature: ClaimSignature = serde_json::from_str(&text).unwrap();
            signature.nonce.clear();
            signature.signature.clear();
            (claim_id, signature)
        })
        .collect()
}

/// The decoded payload of each envelope, by its claims' subjects.
fn payloads(connection: &Connection) -> BTreeMap<String, ReplicaEnvelopePayload> {
    let mut statement = connection
        .prepare("SELECT payload FROM replica_envelopes ORDER BY writer, sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|payload| {
            let payload: ReplicaEnvelopePayload =
                ciborium::from_reader(payload.unwrap().as_slice()).unwrap();
            let subjects = payload
                .batch
                .claims
                .iter()
                .map(|claim| claim.subject.as_str())
                .collect::<Vec<_>>()
                .join(",");
            (subjects, payload)
        })
        .collect()
}

/// Check that every envelope this node sealed carries one signature, by `key` for `fleet_id`,
/// and return how many there are.
fn envelopes_signed_by(connection: &Connection, key: &MemberKey, fleet_id: &str) -> usize {
    let envelopes: usize = connection
        .query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut statement = connection
        .prepare(
            "SELECT writer, sequence, envelope_hash, member_key, signature
             FROM replica_envelope_signatures",
        )
        .unwrap();
    let signatures = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for (writer, sequence, hash, member_key, signature) in &signatures {
        assert_eq!(member_key, key.public());
        assert!(
            verify_signature(
                member_key,
                &envelope_signature_message(fleet_id, writer, *sequence, hash),
                signature
            ),
            "the signature of {writer}/{sequence} is for fleet {fleet_id}"
        );
    }
    assert_eq!(signatures.len(), envelopes);
    envelopes
}

#[test]
fn prepared_sealing_stores_exactly_what_the_old_sealing_stores() {
    let store = node("aspen");
    keyed(&store);
    let stored = store.put_blob(b"bytes a note names").unwrap();
    let missing = "ab".repeat(32);
    // More than one chunk of 64 batches.
    for index in 0..70 {
        note(&store, &format!("note/plain-{index}"), BTreeMap::new());
    }
    note(&store, "note/names-a-stored-blob", naming(&stored));
    note(&store, "note/names-a-missing-blob", naming(&missing));
    // A claim signature carries a fresh nonce; sign each claim once, before both passes, so
    // both seal the same bytes.
    let written = claims(&store.readers.get());
    {
        let connection = store.connection.write();
        for claim in &written {
            let signature = store.sign_unsealed(&unsealed(claim)).unwrap();
            principals::store_claim_signature_tx(&connection, &claim.id, &signature).unwrap();
        }
    }
    let old = sealed_by_the_old_passes(&store, sealed_rows);
    assert_eq!(
        seal(&store),
        0,
        "nothing changed between preparation and the writer"
    );
    let target = max_batch_rowid(&store.readers.get()).unwrap();
    assert_eq!(store.seeded_batch_rowid.load(Ordering::Acquire), target);
    let new = sealed_rows(&store.readers.get());
    assert_eq!(new[0].len(), 72, "one envelope per batch");
    assert_eq!(new[2].len(), 72, "one member signature per envelope");
    assert_eq!(new, old);
    let payloads = payloads(&store.readers.get());
    assert!(
        payloads["note/names-a-stored-blob"]
            .blobs
            .contains_key(&stored)
    );
    assert!(payloads["note/names-a-missing-blob"].blobs.is_empty());
}

#[test]
fn a_claim_signed_while_preparing_is_signed_as_the_old_sealing_signs_it() {
    let store = node("birch");
    let key = keyed(&store);
    for index in 0..3 {
        note(&store, &format!("note/unsigned-{index}"), BTreeMap::new());
    }
    let old = sealed_by_the_old_passes(&store, signers);
    assert_eq!(old.len(), 3);
    assert_eq!(seal(&store), 0);
    let reader = store.readers.get();
    assert_eq!(signers(&reader), old);
    for (subject, payload) in payloads(&reader) {
        let claim = &payload.batch.claims[0];
        let stored: String = reader
            .query_row(
                "SELECT signature FROM claim_signatures WHERE claim_id=?1",
                [&claim.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            payload.claim_signatures.get(&claim.id),
            Some(&serde_json::from_str::<ClaimSignature>(&stored).unwrap()),
            "{subject} carries the signature stored for it"
        );
    }
    assert_eq!(envelopes_signed_by(&reader, &key, FLEET), 3);
}

#[test]
fn a_named_blob_stored_or_dropped_after_preparation_is_sealed_as_the_store_holds_it() {
    for stored_late in [true, false] {
        let store = node("cedar");
        keyed(&store);
        let bytes = b"bytes that arrive or go".to_vec();
        let hash = hex::encode(Sha256::digest(&bytes));
        if !stored_late {
            // The note below promotes the upload into the shared table.
            store.put_blob(&bytes).unwrap();
        }
        note(&store, "note/names-a-blob", naming(&hash));
        let named = hash.clone();
        before_the_loan(move |store| {
            if stored_late {
                // Shared blobs arrive only with a claim: here a second note promotes it.
                let hash = store.put_blob(&bytes).unwrap();
                note(store, "note/stores-the-blob", naming(&hash));
            } else {
                store
                    .connection
                    .write()
                    .execute("DELETE FROM blobs WHERE hash=?1", [&named])
                    .unwrap();
            }
        });
        assert_eq!(seal(&store), 1, "stored_late={stored_late}");
        let payloads = payloads(&store.readers.get());
        let carried = payloads["note/names-a-blob"].blobs.get(&hash);
        if stored_late {
            assert_eq!(
                carried.map(Vec::as_slice),
                Some(&b"bytes that arrive or go"[..])
            );
        } else {
            assert!(carried.is_none());
        }
    }
}

#[test]
fn a_claim_signature_stored_after_preparation_is_the_one_sealed() {
    let store = node("dogwood");
    keyed(&store);
    let claim = note(&store, "note/signed-late", BTreeMap::new());
    // Signing again makes another nonce, so this differs from what preparation signs.
    let late = store.sign_unsealed(&unsealed(&claim)).unwrap();
    let (claim_id, signature) = (claim.id.clone(), late.clone());
    before_the_loan(move |store| {
        principals::store_claim_signature_tx(&store.connection.write(), &claim_id, &signature)
            .unwrap();
    });
    assert_eq!(seal(&store), 1);
    let reader = store.readers.get();
    let payloads = payloads(&reader);
    assert_eq!(
        payloads["note/signed-late"].claim_signatures.get(&claim.id),
        Some(&late)
    );
    let stored: String = reader
        .query_row(
            "SELECT signature FROM claim_signatures WHERE claim_id=?1",
            [&claim.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<ClaimSignature>(&stored).unwrap(),
        late
    );
}

#[test]
fn a_member_key_fleet_id_or_keyring_changed_after_preparation_is_signed_again() {
    for change in ["member key", "fleet id", "keyring"] {
        let store = node("elm");
        let first = keyed(&store);
        note(&store, "note/signed-envelope", BTreeMap::new());
        let second = Arc::new(MemberKey::generate().unwrap().0);
        let replacement = second.clone();
        before_the_loan(move |store| match change {
            "member key" => store.load_member_key(Some(replacement)).unwrap(),
            "fleet id" => {
                store
                    .connection
                    .write()
                    .execute(
                        "UPDATE meta SET value=?1 WHERE key='fleet_id'",
                        [OTHER_FLEET],
                    )
                    .unwrap();
            }
            _ => store.keyring.set_node(store.keyring.node()),
        });
        assert_eq!(seal(&store), 1, "{change}");
        let (key, fleet_id) = match change {
            "member key" => (&second, FLEET),
            "fleet id" => (&first, OTHER_FLEET),
            _ => (&first, FLEET),
        };
        assert_eq!(
            envelopes_signed_by(&store.readers.get(), key, fleet_id),
            1,
            "{change}"
        );
    }
}

#[test]
fn a_batch_sealed_by_another_pass_after_preparation_is_not_sealed_again() {
    let store = node("fir");
    let key = keyed(&store);
    for index in 0..3 {
        note(
            &store,
            &format!("note/sealed-elsewhere-{index}"),
            BTreeMap::new(),
        );
    }
    before_the_loan(|store| {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        seed_replica_envelopes_tx(&transaction, &store.origin, None).unwrap();
        transaction.commit().unwrap();
    });
    assert_eq!(seal(&store), 1);
    let reader = store.readers.get();
    assert_eq!(payloads(&reader).len(), 3, "one envelope per batch");
    // The other pass did not sign envelopes; sealing signs the envelopes it finds unsigned.
    assert_eq!(envelopes_signed_by(&reader, &key, FLEET), 3);
    assert_eq!(
        store.seeded_batch_rowid.load(Ordering::Acquire),
        max_batch_rowid(&reader).unwrap()
    );
}

#[test]
fn preparation_stops_at_its_byte_budget_and_prepares_a_larger_batch_alone() {
    let store = node("ginkgo");
    keyed(&store);
    for index in 0..3 {
        note(&store, &format!("note/budget-{index}"), BTreeMap::new());
    }
    let target = max_batch_rowid(&store.readers.get()).unwrap();
    let all = store.prepare_seal(target, usize::MAX).unwrap().unwrap();
    assert_eq!((all.batches.len(), all.through), (3, target));
    let alone = store.prepare_seal(target, 1).unwrap().unwrap();
    assert_eq!(
        alone.batches.len(),
        1,
        "a batch larger than the budget is prepared alone"
    );
    assert_eq!(alone.through, alone.batches[0].batch.rowid);
    let budget = alone.batches[0].payload.len() + 1;
    let two = store.prepare_seal(target, budget).unwrap().unwrap();
    assert_eq!(two.batches.len(), 2, "the second batch reaches the budget");
    assert_eq!(two.through, two.batches[1].batch.rowid);
    assert!(two.through < target);
    assert_eq!(seal(&store), 0);
    assert_eq!(payloads(&store.readers.get()).len(), 3);
}

// The sealing functions as they were before preparation, verbatim from main at 5bf3105b9
// (`seed_replica_envelopes_signed_tx`, and `Store::sign_own_envelopes_range_tx` with `self` as
// `store`).

#[allow(clippy::type_complexity)]
fn legacy_seed_replica_envelopes_signed_tx(
    transaction: &Transaction<'_>,
    relay: &str,
    after_rowid: Option<i64>,
    through_rowid: Option<i64>,
    sign: Option<&dyn Fn(&principals::Unsealed<'_>) -> Option<crate::principal::ClaimSignature>>,
) -> Result<()> {
    let order = if after_rowid.is_some() {
        "batches.rowid"
    } else {
        "origin, replica_sequence, id"
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms
         FROM batches
         WHERE batches.rowid>=?1 AND batches.rowid<=?2
           AND NOT EXISTS (
             SELECT 1 FROM replica_envelopes WHERE replica_envelopes.batch_id=batches.id
         )
         ORDER BY {order}"
    ))?;
    let headers = statement
        .query_map(
            params![
                after_rowid.map_or(i64::MIN, |rowid| rowid.saturating_add(1)),
                through_rowid.unwrap_or(i64::MAX)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (id, writer, sequence, previous_hash, legacy_hash, accepted_at) in headers {
        let mut claim_statement = transaction.prepare(
            "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
             FROM claims WHERE batch_id=?1 ORDER BY store_index",
        )?;
        let claims = claim_statement
            .query_map([&id], claim_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(claim_statement);
        let mut blobs = BTreeMap::new();
        collect_referenced_blobs(transaction, &claims, &mut blobs)?;
        let mut claim_signatures = BTreeMap::new();
        for claim in &claims {
            let existing = transaction
                .prepare_cached("SELECT signature FROM claim_signatures WHERE claim_id=?1")?
                .query_row([&claim.id], |row| row.get::<_, String>(0))
                .optional()?
                .and_then(|text| serde_json::from_str(&text).ok());
            let signature = match (existing, sign) {
                (Some(signature), _) => Some(signature),
                (None, Some(sign)) => sign(&principals::Unsealed {
                    subject: &claim.subject,
                    kind: &claim.kind,
                    actor: claim.actor.as_deref(),
                    body: &claim.body,
                    accepted_at_unix_ms: claim.accepted_at_unix_ms,
                }),
                (None, None) => None,
            };
            if let Some(signature) = signature {
                principals::store_claim_signature_tx(transaction, &claim.id, &signature)?;
                claim_signatures.insert(claim.id.clone(), signature);
            }
        }
        let batch = ReplicaBatch {
            id: id.clone(),
            origin: writer.clone(),
            replica_sequence: sequence,
            previous_hash: previous_hash.clone(),
            hash: legacy_hash,
            accepted_at_unix_ms: accepted_at.parse().unwrap_or_default(),
            claims: claims.clone(),
        };
        let mut payload = Vec::new();
        ciborium::into_writer(
            &ReplicaEnvelopePayload {
                batch,
                blobs,
                claim_signatures,
            },
            &mut payload,
        )?;
        let envelope_hash = replica_envelope_hash(
            &writer,
            sequence,
            previous_hash.as_deref(),
            accepted_at.parse().unwrap_or_default(),
            &payload,
        );
        transaction.execute(
            "INSERT OR IGNORE INTO replica_envelopes(
                 writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms,
                 payload, batch_id, relay, receipt_state, received_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'validated', ?5)",
            params![
                writer,
                sequence,
                envelope_hash,
                previous_hash,
                accepted_at,
                payload,
                id,
                relay
            ],
        )?;
        for (position, claim) in claims.iter().enumerate() {
            let mut raw = Vec::new();
            ciborium::into_writer(claim, &mut raw)?;
            transaction.execute(
                "INSERT OR IGNORE INTO replica_records(
                     record_ref, writer, sequence, envelope_hash, position, raw, state,
                     claim_id, subject_hint, kind_hint, updated_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'valid', ?7, ?8, ?9, ?10)",
                params![
                    replica_record_ref(&writer, sequence, &envelope_hash, position as u64),
                    writer,
                    sequence,
                    envelope_hash,
                    position as u64,
                    raw,
                    claim.id,
                    claim.subject,
                    claim.kind,
                    accepted_at,
                ],
            )?;
        }
    }
    Ok(())
}

fn legacy_sign_own_envelopes_range_tx(
    store: &Store,
    transaction: &Transaction<'_>,
    after_batch_rowid: Option<i64>,
    through_batch_rowid: Option<i64>,
) -> Result<usize> {
    let _timing = time_stage(&store.replication_timers.signing);
    let Some(key) = store
        .member_key
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
    else {
        return Ok(0);
    };
    let Some(fleet_id) = fleet_meta(transaction, "fleet_id")? else {
        return Ok(0);
    };
    let from = if after_batch_rowid.is_some() {
        "FROM batches CROSS JOIN replica_envelopes AS envelopes
         ON envelopes.batch_id=batches.id WHERE batches.rowid>?2 AND batches.rowid<=?4 AND envelopes.writer=?1"
    } else {
        "FROM replica_envelopes AS envelopes WHERE envelopes.writer=?1 AND envelopes.rowid>?2 AND envelopes.rowid<=?4"
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT envelopes.sequence, envelopes.envelope_hash {from}
           AND NOT EXISTS (
             SELECT 1 FROM replica_envelope_signatures AS signatures
             WHERE signatures.writer=envelopes.writer
               AND signatures.sequence=envelopes.sequence
               AND signatures.envelope_hash=envelopes.envelope_hash
               AND signatures.member_key=?3
           )
         ORDER BY envelopes.sequence"
    ))?;
    let unsigned = statement
        .query_map(
            params![
                store.origin,
                after_batch_rowid.unwrap_or(i64::MIN),
                key.public(),
                through_batch_rowid.unwrap_or(i64::MAX),
            ],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let now = now_ms().to_string();
    for (sequence, envelope_hash) in &unsigned {
        let signature = key.sign(&crate::fleet::envelope_signature_message(
            &fleet_id,
            &store.origin,
            *sequence,
            envelope_hash,
        ));
        transaction.execute(
            "INSERT OR IGNORE INTO replica_envelope_signatures(
                 writer, sequence, envelope_hash, member_key, signature, stored_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                store.origin,
                sequence,
                envelope_hash,
                key.public(),
                signature,
                now
            ],
        )?;
    }
    Ok(unsigned.len())
}
