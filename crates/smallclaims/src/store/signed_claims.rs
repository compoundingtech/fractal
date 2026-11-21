//! Serving and admitting the claims of released envelopes (`envelope_release`) as signed claims.
//!
//! A member that released an envelope still lists its identity, without payload, so a peer that
//! lacks it cannot get its bytes here. A peer that lists [`SIGNED_CLAIMS`] in its inventory gets
//! a [`SignedEnvelope`] instead: the envelope's identity with its writer's envelope signature,
//! its batch header, and each claim of it this member holds, with its position, its writer's
//! claim signature, the principal signature it carried and the blobs it names.
//!
//! The receiver checks everything it would check of the envelope except the payload hash, which
//! needs the payload: the envelope signature binds the identity, by a key membership admits for
//! that writer and sequence; the batch header recomputes the batch id; each claim id is its
//! content's hash, its writer signature verifies by an admitted key, and the runtime classifies
//! it valid; each blob is its hash. It then stores the claims as admission does and keeps the
//! identity as released here too, so its inventory lists it and it never asks again. A released
//! identity a member received has no rowid of its own: it takes one below zero, so every capture
//! counts it as sealed, and the receive moves the capture epoch.
//!
//! A peer that does not list the capability is never sent one. How many released envelopes it
//! lacks is counted in the replication status and logged once for that peer.
use super::*;
use crate::replication::{SIGNED_CLAIMS, SignedBatchHeader, SignedClaim, SignedEnvelope};

/// The most signed envelopes one exchange carries, and the most a receive admits.
pub const SIGNED_ENVELOPE_LIMIT: usize = 256;
/// The most payloadless identities one exchange compares to find the released ones a peer
/// lacks. Each costs a lookup; what lies past it is found by a later exchange.
const RELEASED_CANDIDATE_LIMIT: usize = 65_536;

/// A released envelope's identity, its writer's envelope signature, and its batch header.
const HEADER_SQL: &str = "SELECT released.previous_hash, released.accepted_at_unix_ms, released.member_key,
        released.envelope_signature, batches.id, batches.origin, batches.replica_sequence,
        batches.previous_hash, batches.hash, batches.accepted_at_unix_ms
     FROM released_envelopes AS released JOIN batches ON batches.id=released.batch_id
     WHERE released.writer=?1 AND released.sequence=?2 AND released.envelope_hash=?3";

/// The claims of batch `?1` held here, with their positions, in position order.
static CLAIMS_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT {CLAIM_COLUMNS}, positions.position
         FROM claims INDEXED BY claims_batch_index
         JOIN claim_positions AS positions ON positions.claim_id=claims.id
         WHERE claims.batch_id=?1 ORDER BY positions.position"
    )
});

/// Claim `?1`'s writer signature, by the envelope's key `?2` when it has one.
const WRITER_SIGNATURE_SQL: &str = "SELECT member_key, signature FROM claim_member_signatures
     WHERE claim_id=?1 ORDER BY member_key<>?2, member_key LIMIT 1";

/// A received released identity's rowid: one below the lowest, and below zero.
const RECEIVED_ROWID_SQL: &str = "SELECT MIN(0, COALESCE(MIN(original_rowid), 0)) - 1 FROM released_envelopes";

/// What signed-claim exchanges found since this process started.
#[derive(Debug, Default)]
pub struct Tally {
    /// For each peer without [`SIGNED_CLAIMS`], the released envelopes it lacked at its last
    /// exchange.
    pub unservable: BTreeMap<String, u64>,
    logged: BTreeSet<String>,
    /// Signed envelopes refused because something in them did not verify.
    pub refused: u64,
}

/// A blob's bytes, by its hash. Every reader of a blob for signed claims reads it here.
pub fn blob_bytes(connection: &Connection, hash: &str) -> Result<Option<Vec<u8>>> {
    Ok(connection
        .prepare_cached("SELECT bytes FROM blobs WHERE hash=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-he37575a9da8f owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ [hash], |row| row.get(0))
        .optional()?)
}

impl Store {
    /// The released envelopes `remote` lacks, as far as its inventory shows: every one in a
    /// range it does not hold, and in a range that differs, every one its complete listing of
    /// that range leaves out.
    pub fn released_missing(&self, snapshot: &ReplicationSnapshot, remote: &ReplicationInventory) -> Result<Vec<ReplicaEnvelopeId>> {
        let inventory = &snapshot.inventory;
        let mut candidates = Vec::new();
        if !remote.digest.is_empty() && remote.digest == inventory.digest {
            return Ok(candidates);
        }
        if !remote.buckets.is_empty() {
            let theirs = remote
                .buckets
                .iter()
                .map(|bucket| ((bucket.writer.as_str(), bucket.start), bucket))
                .collect::<BTreeMap<_, _>>();
            let mut listed = BTreeMap::<(&str, u64), Vec<&ReplicaEnvelopeId>>::new();
            for identity in &remote.envelopes {
                listed
                    .entry((identity.writer.as_str(), replication_bucket_start(identity.sequence)))
                    .or_default()
                    .push(identity);
            }
            for bucket in &snapshot.buckets {
                if candidates.len() >= RELEASED_CANDIDATE_LIMIT {
                    break;
                }
                let key = (bucket.writer.as_str(), bucket.start);
                let local = inventory.range(&bucket.writer, bucket.start);
                let known = match theirs.get(&key) {
                    None => None,
                    Some(their) if their.digest == bucket.digest => continue,
                    Some(their) => {
                        // Only a complete listing of the range proves what the peer lacks.
                        let Some(known) = listed.get_mut(&key) else { continue };
                        known.sort_unstable();
                        known.dedup();
                        if known.len() as u64 != their.count
                            || replication_bucket_digest(known.iter().copied()) != their.digest
                        {
                            continue;
                        }
                        Some(&*known)
                    }
                };
                candidates.extend(
                    local
                        .iter()
                        .zip(inventory.identities(local))
                        .filter(|(envelope, identity)| {
                            !inventory.has_payload(envelope)
                                && known.is_none_or(|known| known.binary_search(&identity).is_err())
                        })
                        .map(|(_, identity)| identity),
                );
            }
        } else {
            let complete = remote.digest.is_empty() || remote.digest == replication_inventory_digest(&remote.envelopes);
            if !complete {
                return Ok(candidates);
            }
            let known = remote.envelopes.iter().collect::<BTreeSet<_>>();
            candidates.extend(
                inventory
                    .envelopes
                    .iter()
                    .filter(|envelope| !inventory.has_payload(envelope))
                    .map(|envelope| inventory.identity(envelope))
                    .filter(|identity| !known.contains(identity))
                    .take(RELEASED_CANDIDATE_LIMIT),
            );
        }
        candidates.truncate(RELEASED_CANDIDATE_LIMIT);
        let connection = self.readers.get();
        let mut released = Vec::new();
        for identity in candidates {
            if envelope_release::envelope_released(&connection, &identity.writer, identity.sequence, &identity.hash)? {
                released.push(identity);
            }
        }
        Ok(released)
    }

    /// What an exchange answering `remote` carries of the envelopes this member released and
    /// `remote` lacks: their signed envelopes for a peer that lists [`SIGNED_CLAIMS`], or for one
    /// that does not, how many there are.
    pub(crate) fn signed_envelopes_answering(
        &self,
        snapshot: &ReplicationSnapshot,
        remote: &ReplicationInventory,
    ) -> Result<(Vec<SignedEnvelope>, u64)> {
        let _span = crate::profile::span("replication.signed_envelopes");
        let lacking = self.released_missing(snapshot, remote)?;
        if lacking.is_empty() {
            return Ok((Vec::new(), 0));
        }
        if remote.can(SIGNED_CLAIMS) {
            Ok((self.signed_envelopes(&lacking)?, 0))
        } else {
            Ok((Vec::new(), lacking.len() as u64))
        }
    }

    /// The signed envelopes of those of `ids` this member released, with the claims of each it
    /// holds. An envelope or claim this member retired is not served, and an envelope one of
    /// whose claims has no writer signature or names a blob not held here is left out.
    pub fn signed_envelopes(&self, ids: &[ReplicaEnvelopeId]) -> Result<Vec<SignedEnvelope>> {
        let connection = self.readers.get();
        let authoritative = retired::authoritative(&connection)?;
        let mut header = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-heddd98173c4f owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ HEADER_SQL)?;
        let mut claims = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h39f24e5159cd owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ &CLAIMS_SQL)?;
        let mut writer_signature = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h4fbcb77a7406 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ WRITER_SIGNATURE_SQL)?;
        let mut found = Vec::new();
        'envelopes: for id in ids.iter().take(SIGNED_ENVELOPE_LIMIT) {
            if authoritative && retired::envelope_retired(&connection, &id.writer, id.sequence, &id.hash)? {
                continue;
            }
            let Some((previous_hash, accepted, member_key, signature, batch)) = header
                .query_row(/* sqlite-large-value: allow-for-now lv-hf5c5f80a3c6c owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![id.writer, id.sequence, id.hash], |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        SignedBatchHeader {
                            id: row.get(4)?,
                            origin: row.get(5)?,
                            replica_sequence: row.get(6)?,
                            previous_hash: row.get(7)?,
                            hash: row.get(8)?,
                            accepted_at_unix_ms: row.get::<_, String>(9)?.parse().unwrap_or_default(),
                        },
                    ))
                })
                .optional()?
            else {
                continue;
            };
            let held = claims
                .query_map([&batch.id], |row| Ok((claim_from_row(row)?, row.get::<_, u64>(10)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut signed = Vec::with_capacity(held.len());
            let mut blobs = BTreeMap::new();
            for (claim, position) in held {
                if authoritative && retired::claim_retired(&connection, &claim.id)? {
                    continue;
                }
                let Some((claim_key, claim_signature)) = writer_signature
                    .query_row(/* sqlite-large-value: allow-for-now lv-hda03731ef031 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![claim.id, member_key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                    .optional()?
                else {
                    continue 'envelopes;
                };
                let mut hashes = BTreeSet::new();
                collect_hash_fields(&claim.body, &mut hashes);
                for hash in hashes {
                    let Some(bytes) = blob_bytes(&connection, &hash)? else { continue 'envelopes };
                    blobs.insert(hash, crate::claim::EnvelopePayload::from(bytes));
                }
                signed.push(SignedClaim {
                    principal_signature: principals::envelope_signature(&connection, &claim.id)?,
                    claim,
                    position,
                    member_key: claim_key,
                    signature: claim_signature,
                });
            }
            found.push(SignedEnvelope {
                writer: id.writer.clone(),
                sequence: id.sequence,
                hash: id.hash.clone(),
                previous_hash,
                accepted_at_unix_ms: accepted.parse().unwrap_or_default(),
                member_key,
                signature,
                batch,
                claims: signed,
                blobs,
            });
        }
        Ok(found)
    }

    /// Check and admit signed envelopes a peer sent. Returns how many it admitted and how many
    /// it refused; one this member already holds or refuses as tombstoned is neither, nor is
    /// one whose writer membership has no key for yet.
    pub fn receive_signed_envelopes(
        &self,
        fleet_id: &str,
        shard: Option<&str>,
        signed: &[SignedEnvelope],
    ) -> Result<(usize, usize)> {
        if signed.is_empty() {
            return Ok((0, 0));
        }
        let membership = self.fleet_membership_sealed()?;
        let mut refused = 0;
        let mut verified = Vec::new();
        for envelope in signed.iter().take(SIGNED_ENVELOPE_LIMIT) {
            // Membership may not know the writer's key yet: the claim that keys it can travel
            // in the same exchange, as an envelope admitted after this. Such an envelope is sent
            // again by a later exchange, since this member still lacks it.
            if matches!(membership.window(&envelope.writer, envelope.sequence), crate::fleet::Window::Legacy) {
                continue;
            }
            match verify(envelope, fleet_id, shard, &membership) {
                Ok(batch) => verified.push((envelope, batch)),
                Err(reason) => {
                    eprintln!(
                        "st3: refused signed envelope {} {} {}: {reason}",
                        envelope.writer, envelope.sequence, envelope.hash
                    );
                    refused += 1;
                }
            }
        }
        let mut admitted = Vec::new();
        let changes;
        {
            let mut connection = self.connection.write_background();
            let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let before = saved_inventory::changes(&transaction)?;
            let keep_valid = record_bytes::valid_records_kept(&transaction)?;
            for (envelope, batch) in &verified {
                transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h51fc09002710 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ "SAVEPOINT signed_envelope")?;
                match admit_signed_tx(&transaction, envelope, batch, keep_valid, &*self.runtime) {
                    Ok(true) => {
                        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-heb2fcee409d1 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ "RELEASE signed_envelope")?;
                        admitted.push(ReplicaEnvelopeId {
                            writer: envelope.writer.clone(),
                            sequence: envelope.sequence,
                            hash: envelope.hash.clone(),
                        });
                    }
                    Ok(false) => transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hc598d1d74fc1 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ "RELEASE signed_envelope")?,
                    Err(error) => {
                        transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hf9b9b499a0b7 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ "ROLLBACK TO signed_envelope; RELEASE signed_envelope")?;
                        eprintln!(
                            "st3: refused signed envelope {} {} {}: {}",
                            envelope.writer, envelope.sequence, envelope.hash, error.message
                        );
                        refused += 1;
                    }
                }
            }
            if !admitted.is_empty() {
                // A capture under way never saw these claims; it starts again.
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hcdc26b2deabf owner=smallclaims::store::signed_claims TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoint_capture_epoch SET value=value+1 WHERE id=1", [])?;
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-h8709c4273d89 owner=smallclaims::store::signed_claims TODO=chunk-or-reference-exact-wire-values */ "INSERT OR REPLACE INTO meta(key,value) VALUES('replication_admitted_index',?1)",
                    [current_index_tx(&transaction)?.to_string()],
                )?;
            }
            changes = (before, saved_inventory::changes(&transaction)?);
            transaction.commit()?;
        }
        if refused > 0 {
            crate::read_budget::lock_sync(&self.signed_claims)?.refused += refused as u64;
        }
        if !admitted.is_empty() {
            self.defer_replication_projection();
            self.rules_stale.store(true, Ordering::Release);
            self.verdicts_due.store(true, Ordering::Release);
            self.list_received_released(&admitted, changes)?;
        }
        Ok((admitted.len(), refused))
    }

    /// Add identities received as released to the cached replication snapshot, without their
    /// payloads, as a load from nothing lists them, and digest again only the ranges they fall
    /// in. The inventory generation does not move, so the next snapshot builds on this one. The
    /// receive counted one change for the saved inventory per identity (`changes`: before and
    /// after); a snapshot that took in every change before it takes these in too.
    fn list_received_released(&self, identities: &[ReplicaEnvelopeId], changes: (i64, i64)) -> Result<()> {
        {
            let _building = crate::read_budget::lock_sync(&self.replication_snapshot_build)?;
            let mut cached = crate::read_budget::lock_sync(&self.replication_snapshot)?;
            if let Some(previous) = cached.take() {
                let mut snapshot = Arc::try_unwrap(previous).unwrap_or_else(|shared| (*shared).clone());
                let mut touched = BTreeSet::new();
                for identity in identities {
                    touched.insert((identity.writer.clone(), replication_bucket_start(identity.sequence)));
                    snapshot.inventory.insert(identity.clone());
                    snapshot.inventory.mark_payloadless(identity);
                }
                for (writer, start) in &touched {
                    let bucket = snapshot.inventory.bucket(snapshot.inventory.range(writer, *start));
                    match snapshot.buckets.binary_search_by(|existing| {
                        (existing.writer.as_str(), existing.start).cmp(&(writer.as_str(), *start))
                    }) {
                        Ok(position) => snapshot.buckets[position] = bucket,
                        Err(position) => snapshot.buckets.insert(position, bucket),
                    }
                }
                let resume_from = touched
                    .iter()
                    .map(|(writer, start)| {
                        snapshot.buckets.partition_point(|existing| {
                            (existing.writer.as_str(), existing.start) < (writer.as_str(), *start)
                        })
                    })
                    .min()
                    .unwrap_or(snapshot.buckets.len());
                let ReplicationSnapshot { inventory, buckets, digest_prefixes, .. } = &mut snapshot;
                inventory.resume_digest(buckets, digest_prefixes, resume_from);
                snapshot.authority_digest = snapshot.inventory.digest.clone();
                if snapshot.inventory_changes == Some(changes.0) {
                    snapshot.inventory_changes = Some(changes.1);
                }
                *cached = Some(Arc::new(snapshot));
            }
        }
        self.replica_inventory_changed();
        Ok(())
    }

    /// Record how many released envelopes `peer`, which does not take signed claims, lacks, and
    /// log it the first time it lacks any.
    pub fn note_unservable_released(&self, peer: &str, count: u64) -> Result<()> {
        let mut tally = crate::read_budget::lock_sync(&self.signed_claims)?;
        if count == 0 {
            tally.unservable.remove(peer);
            return Ok(());
        }
        tally.unservable.insert(peer.to_owned(), count);
        if tally.logged.insert(peer.to_owned()) {
            eprintln!(
                "st3: {peer} does not take {SIGNED_CLAIMS} and lacks {count} envelopes this member released; it cannot get them here"
            );
        }
        Ok(())
    }

    /// What [`Tally`] holds now, for the replication status.
    pub fn signed_claim_tally(&self) -> Result<(BTreeMap<String, u64>, u64)> {
        let tally = crate::read_budget::lock_sync(&self.signed_claims)?;
        Ok((tally.unservable.clone(), tally.refused))
    }
}

/// Everything about `envelope` that needs no store: its envelope signature by a key membership
/// admits for its writer and sequence, its batch header, and each claim's batch, writer, content
/// hash and writer signature, and each blob's hash. Returns the batch its claims make.
fn verify(
    envelope: &SignedEnvelope,
    fleet_id: &str,
    shard: Option<&str>,
    membership: &crate::fleet::Membership,
) -> std::result::Result<ReplicaBatch, String> {
    let keys = match membership.window(&envelope.writer, envelope.sequence) {
        crate::fleet::Window::Keyed(keys) => keys,
        crate::fleet::Window::Legacy => return Err("its writer has no key for this sequence".into()),
        crate::fleet::Window::Fenced => return Err("no incarnation of its writer holds this sequence".into()),
    };
    let message = crate::fleet::envelope_signature_message_in(
        shard, fleet_id, &envelope.writer, envelope.sequence, &envelope.hash,
    );
    if !keys.contains(&envelope.member_key) || !crate::fleet::verify_signature(&envelope.member_key, &message, &envelope.signature) {
        return Err("its envelope signature is not by a key membership admits".into());
    }
    let header = &envelope.batch;
    let batch = ReplicaBatch {
        id: header.id.clone(),
        origin: header.origin.clone(),
        replica_sequence: header.replica_sequence,
        previous_hash: header.previous_hash.clone(),
        hash: header.hash.clone(),
        accepted_at_unix_ms: header.accepted_at_unix_ms,
        claims: envelope.claims.iter().map(|signed| signed.claim.clone()).collect(),
    };
    if batch.origin != envelope.writer
        || batch.replica_sequence != envelope.sequence
        || batch.previous_hash != envelope.previous_hash
        || batch.accepted_at_unix_ms != envelope.accepted_at_unix_ms
    {
        return Err("the envelope and its batch identify different writes".into());
    }
    crate::hash::verify_replica_batch_header(&batch).map_err(|error| error.message)?;
    let mut positions = BTreeSet::new();
    for signed in &envelope.claims {
        let claim = &signed.claim;
        if claim.batch_id != batch.id || claim.origin != batch.origin {
            return Err(format!("claim `{}` names another batch or writer", claim.id));
        }
        if !crate::hash::claim_id_is_content_hash(claim).map_err(|error| error.to_string())? {
            return Err(format!("claim `{}` is not its content's hash", claim.id));
        }
        if envelope_release::kept_kind(claim) {
            return Err(format!("claim `{}` is of a kind whose envelope is never released", claim.id));
        }
        let message = crate::fleet::keys::claim_signature_message(fleet_id, &envelope.writer, envelope.sequence, &claim.id);
        if !keys.contains(&signed.member_key) || !crate::fleet::verify_signature(&signed.member_key, &message, &signed.signature) {
            return Err(format!("claim `{}` has no writer signature by a key membership admits", claim.id));
        }
        if !positions.insert(signed.position) {
            return Err(format!("two claims hold position {}", signed.position));
        }
    }
    for (hash, bytes) in &envelope.blobs {
        let bytes = bytes.bytes().map_err(|error| format!("blob `{hash}` is not valid base64: {error}"))?;
        if hex::encode(Sha256::digest(bytes)) != *hash {
            return Err(format!("blob `{hash}` does not match its hash"));
        }
    }
    Ok(batch)
}

/// Admit one verified signed envelope: its blobs, its batch, each claim as envelope admission
/// stores a valid one, the writer signatures, and the identity as released here. False when
/// this member holds the envelope, or refuses it as one it tombstoned, retired or released. An
/// error refuses it whole: a claim the runtime does not classify valid cannot wait for a
/// payload, since there is none to read again.
fn admit_signed_tx(
    transaction: &Connection,
    envelope: &SignedEnvelope,
    batch: &ReplicaBatch,
    keep_valid: bool,
    runtime: &dyn Runtime,
) -> std::result::Result<bool, FractalError> {
    let held: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_envelopes WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)",
            params![envelope.writer, envelope.sequence, envelope.hash],
            |row| row.get(0),
        )
        .map_err(internal)?;
    if held || retired::envelope_refused(transaction, &envelope.writer, envelope.sequence, &envelope.hash).map_err(internal)? {
        return Ok(false);
    }
    for (hash, bytes) in &envelope.blobs {
        let bytes = bytes.bytes().map_err(internal)?;
        transaction
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h095c38a86e2c owner=smallclaims::store::signed_claims TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blobs(hash, bytes, size) VALUES (?1, ?2, ?3)")
            .map_err(internal)?
            .execute(/* sqlite-large-value: allow-for-now lv-heb188853547e owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![hash, bytes, bytes.len() as u64])
            .map_err(internal)?;
    }
    let batch_rowid = transaction
        .query_row("SELECT rowid FROM batches WHERE id=?1", [&batch.id], |row| row.get::<_, i64>(0))
        .optional()
        .map_err(internal)?
        .map_or_else(|| next_batch_rowid(transaction).map_err(internal), Ok)?;
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-hbdfba9fcf8a7 owner=smallclaims::store::signed_claims TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms, rowid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(internal)?
        .execute(/* sqlite-large-value: allow-for-now lv-h6ea19cd84702 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![
            batch.id, batch.origin, batch.replica_sequence, batch.previous_hash, batch.hash,
            batch.accepted_at_unix_ms.to_string(), batch_rowid,
        ])
        .map_err(internal)?;
    let now = now_ms().to_string();
    for (claim, signed) in batch.claims.iter().zip(&envelope.claims) {
        match prepared_envelope::classify_claim_tx(transaction, batch, claim, runtime)? {
            ReplicatedClaimAdmission::Valid => {}
            ReplicatedClaimAdmission::UnknownKind | ReplicatedClaimAdmission::UnknownField => {
                return Err(FractalError::new(
                    "signed-claim-not-known",
                    format!("this build does not know every field of claim `{}`", claim.id),
                ));
            }
        }
        if retired::claim_refused(transaction, &claim.id).map_err(internal)? {
            continue;
        }
        canonical::record_position_tx(transaction, &claim.id, signed.position).map_err(internal)?;
        transaction
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-h6ec591188c85 owner=smallclaims::store::signed_claims TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(internal)?
            .execute(/* sqlite-large-value: allow-for-now lv-h6695f067a887 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![
                claim.id, claim.batch_id, claim.subject, claim.kind, claim.origin, claim.actor,
                canonical_json_text(&claim.body).map_err(internal)?,
                serde_json::to_string(&claim.predecessors).map_err(internal)?,
                claim.accepted_at_unix_ms.to_string(),
            ])
            .map_err(internal)?;
        if let Some(signature) = &signed.principal_signature {
            principals::store_carried_signature_tx(transaction, &claim.id, signature, &envelope.writer)
                .map_err(internal)?;
        }
        member_signatures::insert_signature_tx(
            transaction,
            &member_signatures::ClaimMemberSignature {
                claim_id: claim.id.clone(),
                writer: envelope.writer.clone(),
                sequence: envelope.sequence,
                member_key: signed.member_key.clone(),
                signature: signed.signature.clone(),
            },
        )
        .map_err(internal)?;
        if keep_valid {
            transaction
                .prepare_cached(
                    /* sqlite-large-value: allow-for-now lv-h6d72a0efd20e owner=smallclaims::store::signed_claims TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO replica_records(
                         record_ref, writer, sequence, envelope_hash, position, raw, state,
                         claim_id, subject_hint, kind_hint, updated_at_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'valid', ?7, ?8, ?9, ?10)",
                )
                .map_err(internal)?
                .execute(/* sqlite-large-value: allow-for-now lv-h234478a24e5e owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![
                    crate::hash::replica_record_ref(&envelope.writer, envelope.sequence, &envelope.hash, signed.position),
                    envelope.writer, envelope.sequence, envelope.hash, signed.position,
                    record_bytes::VALID_RECORD_RAW, claim.id, claim.subject, claim.kind, now,
                ])
                .map_err(internal)?;
        }
    }
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h15c0380ff98b owner=smallclaims::store::signed_claims TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO released_envelopes(writer, sequence, envelope_hash, previous_hash,
                 accepted_at_unix_ms, batch_id, original_rowid, released_at_unix_ms, member_key,
                 envelope_signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(internal)?
        .execute(/* sqlite-large-value: allow-for-now lv-hd0a50d98497a owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ params![
            envelope.writer, envelope.sequence, envelope.hash, envelope.previous_hash,
            envelope.accepted_at_unix_ms.to_string(), batch.id,
            transaction.query_row(/* sqlite-large-value: allow-for-now lv-ha52d97cb16b5 owner=smallclaims::store::signed_claims TODO=trace-statement-and-value-representation */ RECEIVED_ROWID_SQL, [], |row| row.get::<_, i64>(0)).map_err(internal)?,
            i64::try_from(now_ms()).map_err(internal)?, envelope.member_key, envelope.signature,
        ])
        .map_err(internal)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;

    /// What serving and admitting signed envelopes reads, per envelope and claim, seeks its index.
    #[test]
    fn what_signed_claims_read_seeks_and_scans_nothing() {
        let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
        let connection = store.connection.write();
        for sql in [HEADER_SQL, &CLAIMS_SQL, WRITER_SIGNATURE_SQL, RECEIVED_ROWID_SQL] {
            envelope_release::assert_plan_seeks(&connection, sql);
        }
    }
}
