//! Admission prepares each envelope before it takes the writer. These controls admit real
//! envelopes, with claims, blobs, bad bytes and claims a runtime does not know, through
//! admission and through a frozen copy of the admission function as it was before preparation
//! (main at 7be50273c), and compare every stored row. Then they change the store between
//! preparation and the writer: a row received again, a signature or blob that arrives, a record
//! already repaired, a classification that reads the store.
use super::checkpoint::{DropPlan, SealedSet};
use super::runtime::{IncrementalProjection, LegacyDigestTable, Plain};
use super::*;
use crate::error::Error;
use crate::claim::{EnvelopePayload, ReplicaEnvelopeSignature};
use crate::fleet::MemberKey;
use std::sync::mpsc;
use std::time::Duration;

const FLEET: &str = "8d3f6a2e-1b7c-4e95-a0d4-6c2b9e8f1a37";

fn node(name: &str) -> Arc<Store> {
    node_with(name, Arc::new(Plain))
}

fn node_with(name: &str, runtime: Arc<dyn Runtime>) -> Arc<Store> {
    let store = Arc::new(Store::open_memory(name, runtime).unwrap());
    store.bind_fleet(FLEET).unwrap();
    store
}

fn note(store: &Store, subject: &str, fields: BTreeMap<String, Value>) {
    claim_of(store, subject, "example.note", fields);
}

fn claim_of(store: &Store, subject: &str, kind: &str, fields: BTreeMap<String, Value>) {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: None,
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// Notes from `source`, some naming a blob, exported for `target`.
fn populated(source: &Store, target: &Store, count: usize) -> ReplicationExchange {
    for index in 0..count {
        let mut fields = BTreeMap::new();
        if index % 3 == 0 {
            let hash = source
                .put_blob(format!("attachment {index}").as_bytes())
                .unwrap();
            fields.insert("hash".into(), Value::String(hash));
        }
        note(source, &format!("note/prepared/{index}"), fields);
    }
    source
        .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
        .unwrap()
}

/// Rewrite a valid envelope's payload and its hash: the envelope still matches its bytes, so only
/// what `edit` broke inside fails.
fn forged(
    envelope: &ReplicaEnvelope,
    edit: impl FnOnce(&mut ReplicaEnvelopePayload),
) -> ReplicaEnvelope {
    let mut payload: ReplicaEnvelopePayload =
        ciborium::from_reader(envelope.payload.bytes().unwrap()).unwrap();
    edit(&mut payload);
    let mut bytes = Vec::new();
    ciborium::into_writer(&payload, &mut bytes).unwrap();
    let mut forged = envelope.clone();
    forged.hash = replica_envelope_hash(
        &envelope.writer,
        envelope.sequence,
        envelope.previous_hash.as_deref(),
        envelope.accepted_at_unix_ms,
        &bytes,
    );
    forged.payload = EnvelopePayload::from(bytes);
    forged
}

/// Every row admission stored from birch, without times. Each node's own rows differ.
fn admitted_rows(store: &Store) -> Vec<String> {
    let connection = store.readers.get();
    let mut rows = Vec::new();
    for query in [
        "SELECT 'claim', id, batch_id, subject, kind, body, predecessors, accepted_at_unix_ms
         FROM claims WHERE origin='birch' ORDER BY id",
        "SELECT 'record', record_ref, state, COALESCE(claim_id,''), COALESCE(subject_hint,''),
                COALESCE(kind_hint,''), COALESCE(error_code,''), COALESCE(error_message,''), hex(raw)
         FROM replica_records WHERE writer='birch' ORDER BY record_ref",
        "SELECT 'blob', hash, size, hex(bytes), '', '', '', '' FROM blobs ORDER BY hash",
        "SELECT 'batch', id, origin, replica_sequence, COALESCE(previous_hash,''), hash,
                accepted_at_unix_ms, '' FROM batches WHERE origin='birch' ORDER BY id",
        "SELECT 'envelope', writer, sequence, envelope_hash, receipt_state,
                COALESCE(batch_id,''), COALESCE(validation_error,''), ''
         FROM replica_envelopes WHERE writer='birch' ORDER BY writer, sequence, envelope_hash",
    ] {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hc890695a5fb2 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ query).unwrap();
        let columns = statement.column_count();
        rows.extend(
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| {
                            Ok(match row.get_ref(index)? {
                                rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                                rusqlite::types::ValueRef::Text(text) => {
                                    String::from_utf8_lossy(text).into_owned()
                                }
                                other => format!("{other:?}"),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map(|values| values.join("|"))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap(),
        );
    }
    rows
}

/// The oracle: `validate_and_admit_envelope_tx` exactly as it was on main at 7be50273c, before
/// admission prepared envelopes, copied verbatim apart from its name. It decodes, verifies and
/// writes in one pass and shares no code with `prepared_envelope`.
#[allow(clippy::too_many_lines)]
fn legacy_validate_and_admit_envelope_tx(
    transaction: &Connection,
    envelope: &ReplicaEnvelope,
    runtime: &dyn Runtime,
    outcome: &mut ReplicationAdmission,
) -> Result<(), St3Error> {
    let started = std::time::Instant::now();
    let payload_bytes = envelope.payload.bytes().map_err(|error| {
        St3Error::new(
            "invalid-envelope-payload",
            format!("the envelope payload is not valid base64: {error}"),
        )
    })?;
    let expected_hash = replica_envelope_hash(
        &envelope.writer,
        envelope.sequence,
        envelope.previous_hash.as_deref(),
        envelope.accepted_at_unix_ms,
        payload_bytes,
    );
    if expected_hash != envelope.hash {
        return Err(St3Error::new(
            "envelope-hash-mismatch",
            "the envelope hash does not match its payload",
        ));
    }
    let payload: ReplicaEnvelopePayload =
        ciborium::from_reader(payload_bytes).map_err(|error| {
            St3Error::new(
                "invalid-envelope-payload",
                format!("the envelope payload is not valid CBOR: {error}"),
            )
        })?;
    let batch = &payload.batch;
    if batch.origin != envelope.writer
        || batch.replica_sequence != envelope.sequence
        || batch.previous_hash != envelope.previous_hash
        || batch.accepted_at_unix_ms != envelope.accepted_at_unix_ms
    {
        return Err(St3Error::new(
            "envelope-batch-mismatch",
            "the envelope and its batch identify different writes",
        ));
    }
    verify_replica_batch_header(batch)?;
    outcome.verify += started.elapsed();
    let now = now_ms().to_string();
    let mut degraded = false;
    for (offset, (hash, bytes)) in payload.blobs.iter().enumerate() {
        let position = batch.claims.len() as u64 + offset as u64;
        let record_ref = replica_record_ref(
            &envelope.writer,
            envelope.sequence,
            &envelope.hash,
            position,
        );
        let valid = hex::encode(Sha256::digest(bytes)) == *hash;
        let previous_state = transaction
            .prepare_cached("SELECT state FROM replica_records WHERE record_ref=?1")
            .map_err(internal)?
            .query_row(/* sqlite-large-value: allow-for-now lv-h251cf196ca8f owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ [&record_ref], |row| row.get::<_, String>(0))
            .optional()
            .map_err(internal)?;
        if valid {
            transaction
                .prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h467546f5abc5 owner=smallclaims::store::prepared_envelope_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blobs(hash, bytes, size) VALUES (?1, ?2, ?3)",
                )
                .map_err(internal)?
                .execute(/* sqlite-large-value: allow-for-now lv-h3f00eae39626 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![hash, bytes, bytes.len() as u64])
                .map_err(internal)?;
            transaction
                .prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h2e4257ac0054 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                         record_ref, writer, sequence, envelope_hash, position, raw, state,
                         subject_hint, kind_hint, updated_at_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'valid', ?7, 'blob', ?8)
                     ON CONFLICT(record_ref) DO UPDATE SET state=CASE WHEN state='repaired' THEN state ELSE 'valid' END,
                        subject_hint=excluded.subject_hint,
                            kind_hint='blob',
                            error_code=CASE WHEN state='repaired' THEN error_code ELSE NULL END,
                            error_message=CASE WHEN state='repaired' THEN error_message ELSE NULL END,
                        updated_at_unix_ms=excluded.updated_at_unix_ms",
                )
                .map_err(internal)?
                .execute(/* sqlite-large-value: allow-for-now lv-he50b4185610a owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![
                        record_ref,
                        envelope.writer,
                        envelope.sequence,
                        envelope.hash,
                        position,
                        bytes,
                        format!("blob/{hash}"),
                        now_ms().to_string(),
                    ])
                .map_err(internal)?;
            outcome.valid += 1;
            outcome.changed |= previous_state.as_deref() != Some("valid");
        } else {
            degraded = true;
            transaction
                .prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h3bda83416645 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                         record_ref, writer, sequence, envelope_hash, position, raw, state,
                         subject_hint, kind_hint, error_code, error_message, updated_at_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'invalid', ?7, 'blob',
                               'blob-hash-mismatch', ?8, ?9)
                     ON CONFLICT(record_ref) DO UPDATE SET state=CASE WHEN state='repaired' THEN state ELSE 'invalid' END,
                        subject_hint=excluded.subject_hint, kind_hint='blob', error_code=excluded.error_code,
                        error_message=excluded.error_message, updated_at_unix_ms=excluded.updated_at_unix_ms",
                )
                .map_err(internal)?
                .execute(/* sqlite-large-value: allow-for-now lv-hc8afba984753 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![
                        record_ref,
                        envelope.writer,
                        envelope.sequence,
                        envelope.hash,
                        position,
                        bytes,
                        format!("blob/{hash}"),
                        format!("replicated blob `{hash}` failed verification"),
                        now_ms().to_string(),
                    ])
                .map_err(internal)?;
            outcome.invalid += 1;
        }
    }
    let batch_rowid = transaction
        .query_row(
            "SELECT rowid FROM batches WHERE id=?1",
            [&batch.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(internal)?
        .map_or_else(|| next_batch_rowid(transaction).map_err(internal), Ok)?;
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h77d7b285efb4 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms, rowid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(internal)?
        .execute(/* sqlite-large-value: allow-for-now lv-h9d21056b828a owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![
                batch.id,
                batch.origin,
                batch.replica_sequence,
                batch.previous_hash,
                batch.hash,
                batch.accepted_at_unix_ms.to_string(),
                batch_rowid,
            ])
        .map_err(internal)?;
    for (position, claim) in batch.claims.iter().enumerate() {
        let position = position as u64;
        let record_ref = replica_record_ref(
            &envelope.writer,
            envelope.sequence,
            &envelope.hash,
            position,
        );
        let mut raw = Vec::new();
        ciborium::into_writer(claim, &mut raw).map_err(internal)?;
        let previous_state = transaction
            .prepare_cached("SELECT state FROM replica_records WHERE record_ref=?1")
            .map_err(internal)?
            .query_row(/* sqlite-large-value: allow-for-now lv-h5047e43e1ca2 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ [&record_ref], |row| row.get::<_, String>(0))
            .optional()
            .map_err(internal)?;
        let started = std::time::Instant::now();
        let classification = validate_replicated_claim(transaction, batch, claim, runtime);
        outcome.verify += started.elapsed();
        match classification {
            Ok(ReplicatedClaimAdmission::Valid) => {
                // A checkpoint dropped this claim here, and another envelope carries it again.
                // A node that still held it would ignore the copy as already stored.
                let inserted = if checkpoint::claim_tombstoned(transaction, &claim.id)
                    .map_err(internal)?
                {
                    0
                } else {
                    transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-hefa396f113ae owner=smallclaims::store::prepared_envelope_tests TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .map_err(internal)?
                    .execute(/* sqlite-large-value: allow-for-now lv-hb2db0d1b6b32 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![
                        claim.id,
                        claim.batch_id,
                        claim.subject,
                        claim.kind,
                        claim.origin,
                        claim.actor,
                        canonical_json_text(&claim.body).map_err(internal)?,
                        serde_json::to_string(&claim.predecessors).map_err(internal)?,
                        claim.accepted_at_unix_ms.to_string(),
                    ])
                    .map_err(internal)?
                };
                if let Some(signature) = payload.claim_signatures.get(&claim.id) {
                    principals::store_claim_signature_tx(transaction, &claim.id, signature)
                        .map_err(internal)?;
                }
                transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-h5b54e53d110c owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                             record_ref, writer, sequence, envelope_hash, position, raw, state,
                             claim_id, subject_hint, kind_hint, error_code, error_message, updated_at_unix_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'valid', ?7, ?8, ?9, NULL, NULL, ?10)
                         ON CONFLICT(record_ref) DO UPDATE SET state=CASE WHEN state='repaired' THEN state ELSE 'valid' END,
                            claim_id=excluded.claim_id,
                            subject_hint=excluded.subject_hint, kind_hint=excluded.kind_hint,
                            error_code=CASE WHEN state='repaired' THEN error_code ELSE NULL END,
                            error_message=CASE WHEN state='repaired' THEN error_message ELSE NULL END,
                            updated_at_unix_ms=excluded.updated_at_unix_ms",
                    )
                    .map_err(internal)?
                    .execute(/* sqlite-large-value: allow-for-now lv-he0f1a9ba8d67 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![record_ref, envelope.writer, envelope.sequence, envelope.hash, position, raw, claim.id, claim.subject, claim.kind, now])
                    .map_err(internal)?;
                outcome.valid += 1;
                outcome.changed |= inserted != 0 || previous_state.as_deref() != Some("valid");
            }
            Ok(
                classification @ (ReplicatedClaimAdmission::UnknownKind
                | ReplicatedClaimAdmission::UnknownField),
            ) => {
                let (error_code, error_message) = match classification {
                    ReplicatedClaimAdmission::UnknownKind => (
                        "unknown-claim-kind",
                        "this st build does not know the claim kind",
                    ),
                    ReplicatedClaimAdmission::UnknownField => (
                        "unknown-claim-field",
                        "this st build does not know every field on the claim kind",
                    ),
                    ReplicatedClaimAdmission::Valid => unreachable!(),
                };
                transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-h28ce9ec17c3e owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                             record_ref, writer, sequence, envelope_hash, position, raw, state,
                             claim_id, subject_hint, kind_hint, error_code, error_message, updated_at_unix_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'unknown', ?7, ?8, ?9, ?10, ?11, ?12)
                         ON CONFLICT(record_ref) DO UPDATE SET state=CASE WHEN state='repaired' THEN state ELSE 'unknown' END,
                            error_code=CASE WHEN state='repaired' THEN error_code ELSE excluded.error_code END,
                            error_message=CASE WHEN state='repaired' THEN error_message ELSE excluded.error_message END,
                            updated_at_unix_ms=excluded.updated_at_unix_ms",
                    )
                    .map_err(internal)?
                    .execute(/* sqlite-large-value: allow-for-now lv-he7da7c216d5d owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![record_ref, envelope.writer, envelope.sequence, envelope.hash, position, raw, claim.id, claim.subject, claim.kind, error_code, error_message, now])
                    .map_err(internal)?;
                outcome.unknown += 1;
            }
            Err(error) => {
                degraded = true;
                transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-hd29771f298b0 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                             record_ref, writer, sequence, envelope_hash, position, raw, state,
                             claim_id, subject_hint, kind_hint, error_code, error_message, updated_at_unix_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'invalid', ?7, ?8, ?9, ?10, ?11, ?12)
                         ON CONFLICT(record_ref) DO UPDATE SET state=CASE WHEN state='repaired' THEN state ELSE 'invalid' END,
                            error_code=excluded.error_code, error_message=excluded.error_message,
                            updated_at_unix_ms=excluded.updated_at_unix_ms",
                    )
                    .map_err(internal)?
                    .execute(/* sqlite-large-value: allow-for-now lv-h1fe09bdc7663 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![record_ref, envelope.writer, envelope.sequence, envelope.hash, position, raw, claim.id, claim.subject, claim.kind, error.code, error.message, now])
                    .map_err(internal)?;
                outcome.invalid += 1;
            }
        }
    }
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h999edd954b96 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state=?4, validation_error=NULL, batch_id=?5
             WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
        )
        .map_err(internal)?
        .execute(/* sqlite-large-value: allow-for-now lv-h12c052f5eba9 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ params![
            envelope.writer,
            envelope.sequence,
            envelope.hash,
            if degraded { "degraded" } else { "validated" },
            batch.id,
        ])
        .map_err(internal)?;
    Ok(())
}

/// Admit every pending envelope with the oracle, each in its own savepoint as admission does.
fn admit_with_the_old_function(store: &Store) {
    let envelopes = {
        let connection = store.readers.get();
        let mut statement = connection
            .prepare(
                "SELECT writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms, payload
                 FROM replica_envelopes WHERE receipt_state='pending'
                 ORDER BY writer, sequence, envelope_hash",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok(ReplicaEnvelope {
                    writer: row.get(0)?,
                    sequence: row.get(1)?,
                    hash: row.get(2)?,
                    previous_hash: row.get(3)?,
                    accepted_at_unix_ms: row.get::<_, String>(4)?.parse().unwrap(),
                    payload: row.get(5)?,
                    member_key: None,
                    signature: None,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let mut outcome = ReplicationAdmission::default();
    let mut connection = store.connection.write();
    let mut transaction = connection.transaction().unwrap();
    for envelope in &envelopes {
        let savepoint = transaction.savepoint().unwrap();
        match legacy_validate_and_admit_envelope_tx(
            &savepoint,
            envelope,
            &*store.runtime,
            &mut outcome,
        ) {
            Ok(()) => savepoint.commit().unwrap(),
            Err(error) => {
                drop(savepoint);
                record_invalid_replica_envelope(&transaction, envelope, &error).unwrap();
            }
        }
    }
    transaction.commit().unwrap();
}

#[test]
fn prepared_admission_stores_exactly_what_the_old_function_stores() {
    let source = node("birch");
    let prepared = node("cedar");
    let single = node("larch");
    let mut exchange = populated(&source, &prepared, 24);
    assert!(exchange.envelopes.len() > ADMISSION_CHUNK_ENVELOPES, "spans several loans");
    // Bad bytes take every failure path: an envelope whose bytes do not match its hash, and a
    // valid envelope carrying a claim whose id is not its content.
    exchange.envelopes[1].payload = EnvelopePayload::from(b"not the envelope".to_vec());
    exchange.envelopes[2] = forged(&exchange.envelopes[2], |payload| {
        payload.batch.claims[0].body = json!({"fields": {"forged": true}});
    });
    // And a valid envelope whose blob's bytes do not match its hash.
    let with_blob = (3..exchange.envelopes.len())
        .find(|index| {
            let payload: ReplicaEnvelopePayload =
                ciborium::from_reader(exchange.envelopes[*index].payload.bytes().unwrap())
                    .unwrap();
            !payload.blobs.is_empty()
        })
        .expect("a later envelope carries a blob");
    exchange.envelopes[with_blob] = forged(&exchange.envelopes[with_blob], |payload| {
        for bytes in payload.blobs.values_mut() {
            bytes.push(b'!');
        }
    });
    for target in [&prepared, &single] {
        target
            .receive_replication_exchange("birch", FLEET, &exchange)
            .unwrap();
    }
    let outcome = prepared.validate_replication_backlog().unwrap();
    admit_with_the_old_function(&single);
    assert_eq!(admitted_rows(&prepared), admitted_rows(&single));
    assert!(outcome.valid > 0 && outcome.invalid >= 3, "{outcome:?}");
    let records = prepared.replica_records(true).unwrap();
    for code in [
        "envelope-hash-mismatch",
        "claim-hash-mismatch",
        "blob-hash-mismatch",
    ] {
        assert!(
            records
                .iter()
                .any(|record| record.error_code.as_deref() == Some(code)),
            "no {code} record"
        );
    }
    let blobs: usize = prepared
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM blobs", [], |row| row.get(0))
        .unwrap();
    assert!(blobs > 0, "the page carried blobs");
}

/// Run `validate_replication_backlog` on its own thread, stopping after it has prepared the
/// first chunk and before it takes the writer, until `between` has run.
fn admit_with_a_change_before_the_loan(
    target: &Arc<Store>,
    between: impl FnOnce(),
) -> ReplicationAdmission {
    let (prepared, preparing) = mpsc::sync_channel(1);
    let (resume, resumed) = mpsc::sync_channel(1);
    let admitting = target.clone();
    let admission = std::thread::spawn(move || {
        BEFORE_ADMISSION_LOAN.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                prepared.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(10)).unwrap();
            }));
        });
        admitting.validate_replication_backlog().unwrap()
    });
    preparing.recv_timeout(Duration::from_secs(5)).unwrap();
    between();
    resume.send(()).unwrap();
    admission.join().unwrap()
}

/// An envelope whose row changes between preparation and the loan is left pending, and the
/// receipt that changed it admits it: every receipt is followed by its own admission pass
/// (`sync::backend` receive, backup restore), which waits on the admission lock for this one.
/// Nothing here calls admission a second time by hand.
#[test]
fn an_envelope_received_again_after_its_preparation_is_admitted_by_that_receipt() {
    let source = node("birch");
    let target = node("cedar");
    let exchange = populated(&source, &target, 1);
    target
        .receive_replication_exchange("birch", FLEET, &exchange)
        .unwrap();
    let envelope = exchange.envelopes[0].clone();
    let mut receiving = None;
    let outcome = admit_with_a_change_before_the_loan(&target, || {
        {
            let mut writer = target.connection.write();
            let transaction = writer.transaction().unwrap();
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-hf044b4ead875 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "DELETE FROM replica_envelopes
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                    params![envelope.writer, envelope.sequence, envelope.hash],
                )
                .unwrap();
            transaction.commit().unwrap();
        }
        // A later receipt, in a later millisecond, of the same bytes, then its admission pass,
        // as the sync backend runs them.
        std::thread::sleep(Duration::from_millis(3));
        let (received, receipt) = mpsc::sync_channel(1);
        let (store, exchange) = (target.clone(), exchange.clone());
        receiving = Some(std::thread::spawn(move || {
            store
                .receive_replication_exchange("birch", FLEET, &exchange)
                .unwrap();
            received.send(()).unwrap();
            store.validate_replication_backlog().unwrap()
        }));
        receipt.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    assert_eq!((outcome.valid, outcome.invalid), (0, 0), "the stale preparation admitted nothing");
    let later = receiving.unwrap().join().unwrap();
    assert!(later.valid > 0, "{later:?}");
    let state: String = target
        .readers
        .get()
        .query_row(
            "SELECT receipt_state FROM replica_envelopes WHERE writer='birch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "validated");
}

/// A signature is read under the writer, not prepared: one that lands between preparation and
/// the loan admits the envelope in the same pass.
#[test]
fn a_signature_that_arrives_after_preparation_admits_in_the_same_pass() {
    let source = node("birch");
    let target = node("cedar");
    let (key, _) = MemberKey::generate().unwrap();
    let key = Arc::new(key);
    target.set_member_key(Some(key.clone())).unwrap();
    target.pin_fleet_anchor(key.public()).unwrap();
    target
        .admit_fleet_anchor(FLEET, key.public(), "listening")
        .unwrap();
    target
        .append_claim(&ClaimInput {
            subject: "host/birch".into(),
            kind: "fleet.member-admitted".into(),
            actor: None,
            fields: BTreeMap::from([
                ("fleet_id".into(), Value::String(FLEET.into())),
                ("member_key".into(), Value::String(key.public().into())),
                ("via".into(), Value::String("invite".into())),
                ("mode".into(), Value::String("listening".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    let exchange = populated(&source, &target, 1);
    let envelope = exchange.envelopes[0].clone();
    assert!(matches!(
        target
            .fleet_membership()
            .unwrap()
            .window("birch", envelope.sequence),
        crate::fleet::Window::Keyed(_)
    ));
    target
        .receive_replication_exchange("birch", FLEET, &exchange)
        .unwrap();
    let outcome = admit_with_a_change_before_the_loan(&target, || {
        let mut signed = exchange.clone();
        signed.envelopes.clear();
        signed.signatures.push(ReplicaEnvelopeSignature {
            writer: envelope.writer.clone(),
            sequence: envelope.sequence,
            hash: envelope.hash.clone(),
            member_key: key.public().into(),
            signature: key.sign(&crate::fleet::envelope_signature_message(
                FLEET,
                &envelope.writer,
                envelope.sequence,
                &envelope.hash,
            )),
        });
        target
            .receive_replication_exchange("birch", FLEET, &signed)
            .unwrap();
    });
    assert_eq!((outcome.valid > 0, outcome.held), (true, 0), "{outcome:?}");
}

#[test]
fn admission_keeps_a_record_repaired_before_it() {
    let source = node("birch");
    let target = node("cedar");
    let exchange = populated(&source, &target, 2);
    target
        .receive_replication_exchange("birch", FLEET, &exchange)
        .unwrap();
    let envelope = &exchange.envelopes[0];
    let record_ref = replica_record_ref(&envelope.writer, envelope.sequence, &envelope.hash, 0);
    let outcome = admit_with_a_change_before_the_loan(&target, || {
        let mut writer = target.connection.write();
        let transaction = writer.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hfc3f88eaf9a9 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(
                     record_ref, writer, sequence, envelope_hash, position, raw, state,
                     error_code, error_message, updated_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, 0, x'', 'repaired', 'claim-hash-mismatch', 'repaired by hand', '0')",
                params![record_ref, envelope.writer, envelope.sequence, envelope.hash],
            )
            .unwrap();
        transaction.commit().unwrap();
    });
    assert!(outcome.valid > 0);
    let (state, error_code): (String, Option<String>) = target
        .readers
        .get()
        .query_row(
            "SELECT state, error_code FROM replica_records WHERE record_ref=?1",
            [&record_ref],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), error_code.as_deref()),
        ("repaired", Some("claim-hash-mismatch"))
    );
}

/// Snapshot every column, including timestamps and wire bytes.
fn admission_snapshot(connection: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut result = Vec::new();
    for query in [
        "SELECT * FROM replica_records ORDER BY record_ref",
        "SELECT * FROM replica_envelopes ORDER BY writer, sequence, envelope_hash",
    ] {
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h37cf17a6da08 owner=smallclaims::store::prepared_envelope_tests TODO=trace-statement-and-value-representation */ query).unwrap();
        let columns = statement.column_count();
        result.extend(
            statement
                .query_map([], |row| {
                    (0..columns).map(|index| row.get(index)).collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap(),
        );
    }
    result
}

#[test]
fn admission_retry_preserves_rows_and_skips_sqlite_changes() {
    // Unknown and invalid claims do not touch claim_positions, so the entire admission
    // has zero changes on retry. A valid claim has one unrelated position UPSERT.
    for (kind, position_changes, invalid_blob) in [
        ("example.unknown", 0, false),
        ("example.extra", 0, false),
        ("example.bad", 0, false),
        ("example.note", 1, false),
        ("example.unknown", 0, true),
    ] {
        let source = node("person/robin");
        let target = node_with("person/recipient", ByKind::new(true));
        let hash = source.put_blob(b"attachment").unwrap();
        claim_of(
            &source,
            "agent/example-retry",
            kind,
            BTreeMap::from([("hash".into(), Value::String(hash))]),
        );
        let mut exchange = source
            .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
            .unwrap();
        if invalid_blob {
            exchange.envelopes[0] = forged(&exchange.envelopes[0], |payload| {
                for bytes in payload.blobs.values_mut() {
                    bytes.push(b'!');
                }
            });
        }
        target
            .receive_replication_exchange("person/robin", FLEET, &exchange)
            .unwrap();
        target.validate_replication_backlog().unwrap();
        let connection = target.connection.write();
        // Use an unmistakably older timestamp; no sleeps or clock-resolution assumptions.
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-hbff1a9ddcb96 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET updated_at_unix_ms='0'", [])
            .unwrap();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hf22585109b10 owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE admission_writes(table_name TEXT);
             CREATE TEMP TRIGGER record_admission_write AFTER UPDATE ON main.replica_records
             BEGIN INSERT INTO admission_writes VALUES ('record'); END;
             CREATE TEMP TRIGGER envelope_admission_write AFTER UPDATE ON main.replica_envelopes
             BEGIN INSERT INTO admission_writes VALUES ('envelope'); END;",
        ).unwrap();
        let before = admission_snapshot(&connection);
        let changes = connection.total_changes();
        for envelope in &exchange.envelopes {
            validate_and_admit_envelope_tx(
                &connection, envelope, &*target.runtime, &mut ReplicationAdmission::default(),
            ).unwrap();
        }
        // No envelope/record statement or its triggers changed anything. Only the
        // existing canonical-position statement can contribute to this delta.
        assert_eq!(connection.total_changes() - changes, position_changes, "{kind}");
        assert_eq!(admission_snapshot(&connection), before, "{kind}");
        let writes: u64 = connection
            .query_row("SELECT count(*) FROM admission_writes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(writes, 0, "{kind}");
    }
}

#[test]
fn admission_retry_applies_semantic_changes_and_preserves_repairs() {
    let source = node("person/robin");
    let target = node_with("person/recipient", ByKind::new(false));
    claim_of(&source, "agent/example-gated", "example.gated", BTreeMap::new());
    let exchange = source
        .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
        .unwrap();
    target.receive_replication_exchange("person/robin", FLEET, &exchange).unwrap();
    target.validate_replication_backlog().unwrap();
    let connection = target.connection.write();
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h83b30c1ad48e owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO meta(key,value) VALUES ('gate','open');
         UPDATE replica_records SET updated_at_unix_ms='0';
         UPDATE replica_envelopes SET receipt_state='degraded',validation_error='stale',batch_id=NULL;",
    ).unwrap();
    let envelope = &exchange.envelopes[0];
    validate_and_admit_envelope_tx(
        &connection, envelope, &*target.runtime, &mut ReplicationAdmission::default(),
    ).unwrap();
    let record: (String, Option<String>, Option<String>, String) = connection.query_row(
        "SELECT state,error_code,error_message,updated_at_unix_ms FROM replica_records",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!((&*record.0, record.1, record.2), ("valid", None, None));
    assert_ne!(record.3, "0");
    let receipt: (String, Option<String>, Option<String>) = connection.query_row(
        "SELECT receipt_state,validation_error,batch_id FROM replica_envelopes",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!((&*receipt.0, receipt.1), ("validated", None));
    assert!(receipt.2.is_some());
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h8d47cd9402ea owner=smallclaims::store::prepared_envelope_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_records SET state='repaired',error_code='original-error',
             error_message='repaired by hand',updated_at_unix_ms='0';",
    ).unwrap();
    let before = admission_snapshot(&connection);
    validate_and_admit_envelope_tx(
        &connection, envelope, &*target.runtime, &mut ReplicationAdmission::default(),
    ).unwrap();
    assert_eq!(admission_snapshot(&connection), before);
}

/// Blob presence is read under the writer, not prepared: a claim whose blob is missing when its
/// envelope is prepared is admitted when the blob is stored before the loan, and refused when it
/// is not.
#[test]
fn a_blob_stored_after_preparation_admits_the_claim_that_names_it() {
    for stored_late in [true, false] {
        let source = node("birch");
        let target = node("cedar");
        let mut exchange = populated(&source, &target, 1);
        let mut carried = Vec::new();
        exchange.envelopes[0] = forged(&exchange.envelopes[0], |payload| {
            carried = std::mem::take(&mut payload.blobs).into_values().collect();
        });
        assert_eq!(carried.len(), 1, "the note named one blob");
        target
            .receive_replication_exchange("birch", FLEET, &exchange)
            .unwrap();
        let outcome = admit_with_a_change_before_the_loan(&target, || {
            if stored_late {
                // Shared blobs arrive only with a claim: an upload stays local until a claim
                // naming it promotes it, here a local note.
                let hash = target.put_blob(&carried[0]).unwrap();
                note(
                    &target,
                    "note/holds-the-blob",
                    BTreeMap::from([("hash".into(), Value::String(hash.clone()))]),
                );
                let shared: bool = target
                    .readers
                    .get()
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM blobs WHERE hash=?1)",
                        [&hash],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert!(shared, "the blob is in the shared table before the loan");
            }
        });
        let state: String = target
            .readers
            .get()
            .query_row(
                "SELECT state FROM replica_records WHERE writer='birch' AND position=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if stored_late {
            assert_eq!((state.as_str(), outcome.valid), ("valid", 1), "{outcome:?}");
        } else {
            assert_eq!(state, "invalid", "{outcome:?}");
            assert!(
                target
                    .replica_records(true)
                    .unwrap()
                    .iter()
                    .any(|record| record.error_code.as_deref() == Some("missing-replicated-blob"))
            );
        }
    }
}

/// A runtime that classifies by kind: `example.unknown` is unknown, `example.extra` carries an
/// unknown field, `example.bad` is invalid, and `example.gated` is valid only once the store's
/// `gate` meta row exists, a classification that reads the store. Everything else is Plain's.
/// With `unbound`, it classifies every kind but `example.gated` without the store.
struct ByKind {
    unbound: bool,
    with_store: AtomicUsize,
    without_store: AtomicUsize,
}

impl ByKind {
    fn new(unbound: bool) -> Arc<Self> {
        Arc::new(Self {
            unbound,
            with_store: AtomicUsize::new(0),
            without_store: AtomicUsize::new(0),
        })
    }

    fn by_kind(claim: &ClaimRecord) -> Result<ReplicatedClaimAdmission, Error> {
        match claim.kind.as_str() {
            "example.unknown" => Ok(ReplicatedClaimAdmission::UnknownKind),
            "example.extra" => Ok(ReplicatedClaimAdmission::UnknownField),
            "example.bad" => Err(Error::new(
                "invalid-replicated-claim",
                format!("replicated claim `{}` has an invalid body", claim.id),
            )),
            _ => Ok(ReplicatedClaimAdmission::Valid),
        }
    }
}

impl Runtime for ByKind {
    fn migrate_schema(&self, connection: &Connection) -> Result<()> {
        Plain.migrate_schema(connection)
    }

    fn create_schema(&self, connection: &Connection) -> Result<()> {
        Plain.create_schema(connection)
    }

    fn open_projections(&self, transaction: &Transaction<'_>, shared_memory: bool) -> Result<()> {
        Plain.open_projections(transaction, shared_memory)
    }

    fn schema_digest(&self) -> String {
        Plain.schema_digest()
    }

    fn classify_replicated_claim(
        &self,
        connection: &Connection,
        _batch: &ReplicaBatch,
        claim: &ClaimRecord,
    ) -> Result<ReplicatedClaimAdmission, Error> {
        self.with_store.fetch_add(1, Ordering::Relaxed);
        if claim.kind == "example.gated" {
            let open: bool = connection
                .query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key='gate')", [], |row| {
                    row.get(0)
                })
                .map_err(internal)?;
            return Ok(if open {
                ReplicatedClaimAdmission::Valid
            } else {
                ReplicatedClaimAdmission::UnknownKind
            });
        }
        Self::by_kind(claim)
    }

    fn classify_replicated_claim_unbound(
        &self,
        _batch: &ReplicaBatch,
        claim: &ClaimRecord,
    ) -> Option<Result<ReplicatedClaimAdmission, Error>> {
        if !self.unbound || claim.kind == "example.gated" {
            return None;
        }
        self.without_store.fetch_add(1, Ordering::Relaxed);
        Some(Self::by_kind(claim))
    }

    fn append_claim(&self, store: &Store, input: &ClaimInput) -> Result<(ClaimRecord, bool), Error> {
        Plain.append_claim(store, input)
    }

    fn apply_repair_tx(
        &self,
        transaction: &Transaction<'_>,
        repaired: &str,
        replacement: &str,
    ) -> Result<()> {
        Plain.apply_repair_tx(transaction, repaired, replacement)
    }

    fn append_claim_tx(
        &self,
        transaction: &Transaction<'_>,
        origin: &str,
        subject: &str,
        kind: &str,
        actor: Option<&str>,
        body: &Value,
        predecessors: &[String],
        forced_batch: Option<&str>,
    ) -> Result<ClaimRecord> {
        Plain.append_claim_tx(
            transaction,
            origin,
            subject,
            kind,
            actor,
            body,
            predecessors,
            forced_batch,
        )
    }

    fn project_incremental(
        &self,
        transaction: &Transaction<'_>,
        origin: &str,
        through: u64,
    ) -> Result<IncrementalProjection, Error> {
        Plain.project_incremental(transaction, origin, through)
    }

    fn replay_from_nothing(&self, transaction: &Transaction<'_>) -> Result<(), Error> {
        Plain.replay_from_nothing(transaction)
    }

    fn after_projection(&self, transaction: &Transaction<'_>) -> Result<(), Error> {
        Plain.after_projection(transaction)
    }

    fn forget_views(&self) {}

    fn digest_tables(&self) -> &'static [(&'static str, &'static [&'static str])] {
        Plain.digest_tables()
    }

    fn legacy_digest_tables(&self) -> &'static [LegacyDigestTable] {
        Plain.legacy_digest_tables()
    }

    fn checkpoint_rules_digest(&self) -> String {
        Plain.checkpoint_rules_digest()
    }

    fn plan_checkpoint_drops(&self, sealed: &SealedSet) -> DropPlan {
        Plain.plan_checkpoint_drops(sealed)
    }

    fn clear_checkpoint_projections(&self, transaction: &Transaction<'_>) -> Result<()> {
        Plain.clear_checkpoint_projections(transaction)
    }

    fn replay_checkpoint_projections(&self, transaction: &Transaction<'_>) -> Result<()> {
        Plain.replay_checkpoint_projections(transaction)
    }

    fn checkpoint_subject_answers(
        &self,
        connection: &Connection,
        subject: &str,
        cut: u128,
    ) -> Result<Value> {
        Plain.checkpoint_subject_answers(connection, subject, cut)
    }
}

/// Claims of every classification, from a Plain writer that knows them all.
fn classified(source: &Store, target: &Store) -> ReplicationExchange {
    for (index, kind) in [
        "example.note",
        "example.unknown",
        "example.extra",
        "example.bad",
    ]
    .iter()
    .cycle()
    .take(12)
    .enumerate()
    {
        claim_of(source, &format!("note/classified/{index}"), kind, BTreeMap::new());
    }
    source
        .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
        .unwrap()
}

/// A runtime that classifies without the store gets the same stored rows as the old function,
/// which classified with it, and as the same runtime classifying under the writer: valid,
/// unknown kind, unknown field and invalid claims alike.
#[test]
fn an_opted_in_runtime_classifies_before_the_writer_as_the_old_function_does() {
    let source = node("birch");
    let unbound = ByKind::new(true);
    let bound = ByKind::new(false);
    let oracle = ByKind::new(true);
    let before = node_with("cedar", unbound.clone());
    let under = node_with("larch", bound.clone());
    let old = node_with("rowan", oracle.clone());
    let exchange = classified(&source, &before);
    for target in [&before, &under, &old] {
        target
            .receive_replication_exchange("birch", FLEET, &exchange)
            .unwrap();
    }
    before.validate_replication_backlog().unwrap();
    under.validate_replication_backlog().unwrap();
    admit_with_the_old_function(&old);
    let rows = admitted_rows(&old);
    assert_eq!(admitted_rows(&before), rows);
    assert_eq!(admitted_rows(&under), rows);
    let codes = old
        .replica_records(true)
        .unwrap()
        .into_iter()
        .filter_map(|record| record.error_code)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from([
            "invalid-replicated-claim".to_owned(),
            "unknown-claim-field".to_owned(),
            "unknown-claim-kind".to_owned(),
        ])
    );
    // The opted-in runtime never classified with the store; the other always did.
    assert!(unbound.without_store.load(Ordering::Relaxed) >= 12);
    assert_eq!(unbound.with_store.load(Ordering::Relaxed), 0);
    assert_eq!(bound.without_store.load(Ordering::Relaxed), 0);
    assert!(bound.with_store.load(Ordering::Relaxed) >= 12);
}

/// A classification that reads the store runs under the writer, against the store as the loan
/// finds it, whether or not the runtime opts in for other kinds.
#[test]
fn a_classification_that_reads_the_store_sees_the_store_under_the_writer() {
    for unbound in [true, false] {
        for opened in [true, false] {
            let source = node("birch");
            let target = node_with("cedar", ByKind::new(unbound));
            claim_of(&source, "note/gated", "example.gated", BTreeMap::new());
            let exchange = source
                .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
                .unwrap();
            target
                .receive_replication_exchange("birch", FLEET, &exchange)
                .unwrap();
            admit_with_a_change_before_the_loan(&target, || {
                if opened {
                    // A raw meta row: the store state this runtime's classification reads.
                    let mut writer = target.connection.write();
                    let transaction = writer.transaction().unwrap();
                    transaction
                        .execute(/* sqlite-large-value: allow-for-now lv-h475b9c435b3b owner=smallclaims::store::prepared_envelope_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES ('gate', 'open')", [])
                        .unwrap();
                    transaction.commit().unwrap();
                }
            });
            let state: String = target
                .readers
                .get()
                .query_row(
                    "SELECT state FROM replica_records
                     WHERE writer='birch' AND kind_hint='example.gated'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                state,
                if opened { "valid" } else { "unknown" },
                "unbound {unbound}, opened {opened}"
            );
        }
    }
}

/// Exchange from `source` until `target` has received every envelope, without admitting any:
/// large envelopes leave in pages of their own.
fn receive_everything(source: &Store, target: &Store) {
    for _ in 0..32 {
        let exchange = source
            .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            return;
        }
        target
            .receive_replication_exchange("birch", FLEET, &exchange)
            .unwrap();
    }
    panic!("the source kept sending envelopes");
}

/// Preparation ahead of the loans stops at `ADMISSION_PREPARED_BYTES` of payload: three envelopes
/// of 6 MiB are prepared two at a time, and one of 17 MiB alone. Either way admission stores what
/// the old function stores.
#[test]
fn preparation_stops_at_the_byte_budget_and_prepares_a_larger_envelope_alone() {
    const MIB: usize = 1024 * 1024;
    for (sizes, first) in [(vec![6 * MIB; 3], 2), (vec![17 * MIB, MIB], 1)] {
        let source = node("birch");
        let prepared = node("cedar");
        let old = node("larch");
        for (index, size) in sizes.iter().enumerate() {
            let hash = source.put_blob(&vec![index as u8; *size]).unwrap();
            note(
                &source,
                &format!("note/large/{index}"),
                BTreeMap::from([("hash".into(), Value::String(hash))]),
            );
        }
        receive_everything(&source, &prepared);
        receive_everything(&source, &old);
        PREPARED_AHEAD.with(|sets| sets.borrow_mut().clear());
        let outcome = prepared.validate_replication_backlog().unwrap();
        let sets = PREPARED_AHEAD.with(|sets| sets.borrow().clone());
        assert_eq!(sets.first(), Some(&first), "prepared ahead of each loan: {sets:?}");
        assert!(sets.iter().all(|count| *count <= first), "{sets:?}");
        assert_eq!(outcome.invalid, 0, "{outcome:?}");
        admit_with_the_old_function(&old);
        assert_eq!(admitted_rows(&prepared), admitted_rows(&old));
    }
}
