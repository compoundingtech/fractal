use super::*;
use crate::claim::{ClaimInput, ReplicaEnvelopeId};
use crate::replication::ReplicationInventory;
use crate::store::runtime::Plain;
use std::sync::Arc;

const FLEET: &str = "5e0b7c1a-2d3f-4e5a-9b6c-7d8e9fa0b1c2";

fn member(origin: &str) -> Store {
    let store = Store::open_memory(origin, Arc::new(Plain)).unwrap();
    store.bind_fleet(FLEET).unwrap();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store
}

fn note(store: &Store, subject: &str) -> String {
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "example.note".into(),
            actor: None,
            fields: Default::default(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .id
}

fn envelope_of(store: &Store, claim: &str) -> ReplicaEnvelopeId {
    store.seal_local_batches().unwrap();
    store
        .readers
        .get()
        .query_row(
            "SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash
             FROM claims JOIN replica_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
             WHERE claims.id=?1",
            [claim],
            |row| Ok(ReplicaEnvelopeId { writer: row.get(0)?, sequence: row.get(1)?, hash: row.get(2)? }),
        )
        .unwrap()
}

/// Retire a claim ID in the ledger, as a deletion here or on a member this one learned it from.
fn retire_claim(store: &Store, id: &str) {
    store
        .connection
        .write()
        .execute(&format!(/* sqlite-large-value: allow-for-now lv-hd9cb3435dce3 owner=smallclaims::store::retired::authority_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({})", identity_sql("?1")), [id])
        .unwrap();
}

fn retire_envelope(store: &Store, envelope: &ReplicaEnvelopeId) {
    store
        .connection
        .write()
        .execute(
            &format!(
                /* sqlite-large-value: allow-for-now lv-h6d608f0086b4 owner=smallclaims::store::retired::authority_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO retired_envelopes(writer, sequence, envelope_hash)
                 VALUES (?1, ?2, {})",
                identity_sql("?3")
            ),
            params![envelope.writer, envelope.sequence, envelope.hash],
        )
        .unwrap();
}

fn push_everything(from: &Store, to: &Store) {
    // What a peer sends that ignores the receiver's inventory, as an older build answering a
    // claim walk or a first sync does: every envelope it holds.
    let exchange = from.export_replication_exchange(FLEET, &ReplicationInventory::default()).unwrap();
    to.receive_replication_exchange(&from.origin, FLEET, &exchange).unwrap();
    to.validate_replication_backlog().unwrap();
}

fn holds(store: &Store, claim: &str) -> bool {
    store.claim_by_id(claim).unwrap().is_some()
}

#[test]
fn the_ledger_is_the_authority_only_while_trimming_locally_and_after_the_import() {
    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    let id = format!("{:064x}", 7);
    retire_claim(&store, &id);
    let refused = |store: &Store| claim_refused(&store.readers.get(), &id).unwrap();
    assert!(!refused(&store), "not yet the authority");

    let imported = store.retired_identity_import_done().unwrap();
    store.trim_locally(true).unwrap();
    assert_eq!(authoritative(&store.readers.get()).unwrap(), imported);
    // Finishing the import makes it the authority, judged in the same transaction.
    store.import_retired_identities_paced(|_| {}).unwrap();
    assert!(authoritative(&store.readers.get()).unwrap());
    assert!(refused(&store));

    store.trim_locally(false).unwrap();
    assert!(!authoritative(&store.readers.get()).unwrap());
    assert!(!refused(&store), "an ordinary member reads its checkpoint tables again");
    assert!(claim_retired(&store.readers.get(), &id).unwrap(), "the ledger keeps the identity");
}

#[test]
fn becoming_the_authority_drops_what_never_was_a_deletion_and_keeps_the_rest() {
    let store = member("alder");
    let id = |n: u64| format!("{n:064x}");
    let hash = |n: u64| format!("{:064x}", n + 1000);
    store
        .connection
        .write()
        .execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-hd188a331bef9 owner=smallclaims::store::retired::authority_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(id, cut_unix_ms, state, updated_at_unix_ms)
                 VALUES('checkpoint/trimmed', 1, 'trimmed', 1),
                       ('checkpoint/recording', 2, 'recording', 1),
                       ('checkpoint/adopted', 3, 'trimmed', 1);
             INSERT INTO checkpoint_claims(id, writer, sequence, envelope_hash, subject, kind,
                     predecessors, accepted_at_unix_ms, checkpoint)
                 VALUES('{c1}', 'oak', 1, '{h1}', 'note/one', 'example.note', '[]', 1, 'checkpoint/trimmed'),
                       ('{c2}', 'oak', 2, '{h2}', 'note/two', 'example.note', '[]', 1, 'checkpoint/recording'),
                       ('{c3}', 'oak', 3, '{h3}', 'note/three', 'example.note', '[]', 1, 'checkpoint/trimmed'),
                       ('{c5}', 'oak', 5, '{h5}', 'note/five', 'example.note', '[]', 1, 'checkpoint/trimmed');
             INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                 VALUES('oak', 1, '{h1}', 1, 'checkpoint/trimmed'),
                       ('oak', 2, '{h2}', 1, 'checkpoint/recording'),
                       ('oak', 3, '{h3}', 1, 'checkpoint/trimmed'),
                       ('oak', 5, '{h5}', 1, 'checkpoint/trimmed');
             -- Adoption kept a manifest without the third, and recorded the fifth again under
             -- the checkpoint it adopted.
             DELETE FROM checkpoint_claims WHERE id IN ('{c3}', '{c5}');
             DELETE FROM checkpoint_envelopes WHERE sequence IN (3, 5);
             INSERT INTO checkpoint_claims(id, writer, sequence, envelope_hash, subject, kind,
                     predecessors, accepted_at_unix_ms, checkpoint)
                 VALUES('{c5}', 'oak', 5, '{h5}', 'note/five', 'example.note', '[]', 1, 'checkpoint/adopted');
             INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                 VALUES('oak', 5, '{h5}', 1, 'checkpoint/adopted');",
            c1 = id(1), c2 = id(2), c3 = id(3), c5 = id(5),
            h1 = hash(1), h2 = hash(2), h3 = hash(3), h5 = hash(5),
        ))
        .unwrap();
    // Learned from a peer: no checkpoint row at all.
    retire_claim(&store, &id(4));
    retire_envelope(&store, &ReplicaEnvelopeId { writer: "oak".into(), sequence: 4, hash: hash(4) });

    store.trim_locally(true).unwrap();
    let connection = store.readers.get();
    for (n, deleted) in [(1, true), (2, false), (3, false), (4, true), (5, true)] {
        assert_eq!(claim_retired(&connection, &id(n)).unwrap(), deleted, "claim {n}");
        assert_eq!(envelope_retired(&connection, "oak", n, &hash(n)).unwrap(), deleted, "envelope {n}");
    }
    let labels: i64 = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM retired_claim_withdrawals)
                    + (SELECT COUNT(*) FROM retired_envelope_withdrawals)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(labels, 0, "the withdrawal labels are spent");
}

#[test]
fn a_member_that_trims_locally_refuses_what_it_deleted_from_a_peer_that_still_holds_it() {
    let holder = member("birch");
    let deleted = note(&holder, "note/deleted");
    let claim_only = note(&holder, "note/claim-only");
    let kept = note(&holder, "note/kept");
    let deleted_envelope = envelope_of(&holder, &deleted);

    let ordinary = member("alder");
    retire_claim(&ordinary, &deleted);
    retire_envelope(&ordinary, &deleted_envelope);
    retire_claim(&ordinary, &claim_only);
    push_everything(&holder, &ordinary);
    assert!(holds(&ordinary, &deleted) && holds(&ordinary, &claim_only), "control: not the authority");

    let local = member("alder");
    local.trim_locally(true).unwrap();
    retire_claim(&local, &deleted);
    retire_envelope(&local, &deleted_envelope);
    // Only the claim: its envelope is not retired, as when a trim kept the rest of a batch.
    retire_claim(&local, &claim_only);
    push_everything(&holder, &local);
    assert!(!holds(&local, &deleted), "a retired envelope is refused");
    assert!(!holds(&local, &claim_only), "a retired claim is refused from an envelope it admits");
    assert!(holds(&local, &kept));
    // Again, now that the envelope it admitted is held.
    push_everything(&holder, &local);
    assert!(!holds(&local, &deleted) && !holds(&local, &claim_only));

    // An ordinary exchange never asks for it: the inventory lists it as held without payload.
    let summary = local.export_replication_summary(FLEET).unwrap();
    let answer = holder.export_replication_exchange(FLEET, &summary.inventory).unwrap();
    assert!(
        answer.envelopes.iter().all(|envelope| envelope.sequence != deleted_envelope.sequence
            || envelope.writer != deleted_envelope.writer),
        "the holder sends nothing the local member retired"
    );
}

#[test]
fn a_member_that_trims_locally_serves_nothing_it_retired_even_while_it_holds_a_copy() {
    let local = member("alder");
    let retired = note(&local, "note/retired");
    let kept = note(&local, "note/kept");
    let retired_envelope = envelope_of(&local, &retired);
    let kept_envelope = envelope_of(&local, &kept);
    local.trim_locally(true).unwrap();
    retire_claim(&local, &retired);
    retire_envelope(&local, &retired_envelope);

    let served = local.replica_envelopes(vec![retired_envelope.clone(), kept_envelope.clone()]).unwrap();
    assert_eq!(served.iter().map(|envelope| envelope.sequence).collect::<Vec<_>>(), [kept_envelope.sequence]);
    assert_eq!(local.envelopes_carrying_claims(&[retired.clone(), kept.clone()]).unwrap(), [kept_envelope]);

    let fresh = member("birch");
    push_everything(&local, &fresh);
    assert!(!holds(&fresh, &retired), "a peer is never sent it");
    assert!(holds(&fresh, &kept));
}
