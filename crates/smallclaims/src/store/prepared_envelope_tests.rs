//! Admission prepares each envelope before it takes the writer. These controls admit real
//! envelopes, with claims, blobs and bad bytes, both ways and compare every stored row, then
//! change the store between preparation and the writer: a row received again, a signature that
//! arrives, a record already repaired.
use super::runtime::Plain;
use super::*;
use crate::claim::{EnvelopePayload, ReplicaEnvelopeSignature};
use crate::fleet::MemberKey;
use std::sync::mpsc;
use std::time::Duration;

const FLEET: &str = "8d3f6a2e-1b7c-4e95-a0d4-6c2b9e8f1a37";

fn node(name: &str) -> Arc<Store> {
    let store = Arc::new(Store::open_memory(name, Arc::new(Plain)).unwrap());
    store.bind_fleet(FLEET).unwrap();
    store
}

fn note(store: &Store, subject: &str, fields: BTreeMap<String, Value>) {
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
        let mut statement = connection.prepare(query).unwrap();
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

/// Admit every pending envelope the way admission did before it prepared: decode, verify and
/// write each in one pass under the writer.
fn admit_in_one_pass(store: &Store) {
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
    let transaction = connection.transaction().unwrap();
    for envelope in &envelopes {
        let savepoint = transaction.savepoint().unwrap();
        match validate_and_admit_envelope_tx(&savepoint, envelope, &*store.runtime, &mut outcome)
        {
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
fn prepared_admission_stores_exactly_what_one_pass_stores() {
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
    admit_in_one_pass(&single);
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

#[test]
fn an_envelope_received_again_after_its_preparation_waits_for_a_later_pass() {
    let source = node("birch");
    let target = node("cedar");
    let exchange = populated(&source, &target, 1);
    target
        .receive_replication_exchange("birch", FLEET, &exchange)
        .unwrap();
    let envelope = exchange.envelopes[0].clone();
    let outcome = admit_with_a_change_before_the_loan(&target, || {
        {
            let mut writer = target.connection.write();
            let transaction = writer.transaction().unwrap();
            transaction
                .execute(
                    "DELETE FROM replica_envelopes
                     WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3",
                    params![envelope.writer, envelope.sequence, envelope.hash],
                )
                .unwrap();
            transaction.commit().unwrap();
        }
        // A later receipt, in a later millisecond, of the same bytes.
        std::thread::sleep(Duration::from_millis(3));
        target
            .receive_replication_exchange("birch", FLEET, &exchange)
            .unwrap();
    });
    assert_eq!((outcome.valid, outcome.invalid), (0, 0));
    let state = |target: &Store| -> String {
        target
            .readers
            .get()
            .query_row(
                "SELECT receipt_state FROM replica_envelopes WHERE writer='birch'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(state(&target), "pending", "the stale preparation admitted nothing");
    assert!(target.validate_replication_backlog().unwrap().valid > 0);
    assert_eq!(state(&target), "validated");
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
                "INSERT INTO replica_records(
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
