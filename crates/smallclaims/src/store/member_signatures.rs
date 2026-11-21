//! Each claim carries its writer's own signature, so a claim authenticates without the envelope
//! that carried it, and a member can release envelope history and still serve and verify every
//! claim it holds. The writer's member key signs `st3-claim-v1`, the fleet, the writer, the batch's
//! sequence and the claim id (`fleet::keys::claim_signature_message`). The claim id already
//! hashes the claim's content and its batch, so the signature binds both.
//!
//! A writer signs its own claims in a paced background pass, the new ones and those it held
//! before this build (`sign_own_claims`). Signatures travel beside the claims in an exchange's
//! `claim_member_signatures`. A member keeps one only after it verifies with a key the membership
//! window admits for that writer and sequence, and only for a claim whose batch names that writer
//! and sequence. The claim id does not name its writer, so a signature for a claim not held yet
//! waits in `claim_member_signature_pending` until the claim arrives
//! (`promote_claim_member_signatures`); a member's key can sign only its own claims. An older
//! peer ignores the field.
use super::*;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h88bdbfc1cdaf owner=smallclaims::store::member_signatures TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS claim_member_signatures (
    id INTEGER PRIMARY KEY,
    claim_id TEXT NOT NULL,
    member_key TEXT NOT NULL,
    signature TEXT NOT NULL,
    UNIQUE(claim_id, member_key)
);
CREATE TABLE IF NOT EXISTS claim_member_signature_pending (
    claim_id TEXT NOT NULL,
    writer TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    member_key TEXT NOT NULL,
    signature TEXT NOT NULL,
    received_at_unix_ms INTEGER NOT NULL,
    peer TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(claim_id, member_key)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS claim_member_signature_pending_peer ON claim_member_signature_pending(peer);
";

const SIGN_CURSOR: &str = "claim_member_signature_cursor";
const SIGN_PAGE: usize = 512;
const PROMOTE_PAGE: usize = 512;
const PENDING_CURSOR: &str = "claim_member_signature_pending_cursor";
/// The most signatures one peer may have waiting here, and the most in all. A member writes a
/// few thousand claims a day, so a real backlog stays far below these; past them a signature is
/// refused and counted (`REFUSED`), never by evicting one that waits.
pub const PENDING_PER_PEER: usize = 50_000;
pub const PENDING_TOTAL: usize = 200_000;
static CAPS: std::sync::RwLock<(usize, usize)> = std::sync::RwLock::new((PENDING_PER_PEER, PENDING_TOTAL));

/// Set the per-peer and total caps on waiting signatures for this process, in place of
/// [`PENDING_PER_PEER`] and [`PENDING_TOTAL`]; for tests that reach a cap without writing
/// hundreds of thousands of rows.
pub fn limit_waiting_claim_signatures(per_peer: usize, total: usize) {
    *CAPS.write().unwrap_or_else(PoisonError::into_inner) = (per_peer, total);
}
/// How many waiting signatures this member refused for the caps, ever.
const REFUSED: &str = "claim_member_signatures_refused";
/// The same count, as telemetry.
static REFUSED_COUNTER: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> = std::sync::LazyLock::new(|| {
    opentelemetry::global::meter("st3")
        .u64_counter("st.replication.claim_signatures.refused")
        .with_description("Waiting claim signatures refused for the per-peer or total cap")
        .build()
});
/// A signature for a claim that never arrives is dropped after this long.
const PENDING_KEEP_MS: u128 = 30 * 24 * 60 * 60 * 1000;

/// One claim's writer signature, as an exchange carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimMemberSignature {
    pub claim_id: String,
    pub writer: String,
    pub sequence: u64,
    pub member_key: String,
    pub signature: String,
}

/// What one page of [`Store::sign_own_claims`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OwnClaimSigning {
    pub scanned: usize,
    pub signed: usize,
    pub done: bool,
}

impl Store {
    /// Sign one page of this writer's own claims that have no writer signature yet, in claim
    /// order. The page is read and signed outside the writer and stored in one short write; the
    /// cursor commits with it. A store without a member key or fleet signs nothing.
    pub fn sign_own_claims(&self) -> Result<OwnClaimSigning> {
        let Some(key) = self.member_key.read().unwrap_or_else(PoisonError::into_inner).clone() else {
            return Ok(OwnClaimSigning { done: true, ..Default::default() });
        };
        let (fleet_id, after, page) = {
            let connection = self.readers.get();
            let Some(fleet_id) = self.signing_fleet_id(&connection)? else {
                return Ok(OwnClaimSigning { done: true, ..Default::default() });
            };
            let after: i64 = connection
                .query_row("SELECT value FROM meta WHERE key=?1", [SIGN_CURSOR], |row| row.get::<_, String>(0))
                .optional()?
                .map(|value| value.parse::<i64>())
                .transpose()?
                .unwrap_or(0);
            let page = connection
                .prepare_cached(
                    "SELECT claims.store_index, claims.id, batches.replica_sequence,
                            EXISTS(SELECT 1 FROM claim_member_signatures WHERE claim_id=claims.id AND member_key=?4)
                     FROM claims JOIN batches ON batches.id=claims.batch_id
                     WHERE claims.store_index>?1 AND claims.origin=?2
                     ORDER BY claims.store_index LIMIT ?3",
                )?
                .query_map(params![after, self.origin, SIGN_PAGE, key.public()], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, bool>(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            (fleet_id, after, page)
        };
        let mut report = OwnClaimSigning { scanned: page.len(), done: page.is_empty(), ..Default::default() };
        let through = page.last().map_or(after, |row| row.0);
        let signatures: Vec<ClaimMemberSignature> = page
            .into_iter()
            .filter(|(_, _, _, signed)| !signed)
            .map(|(_, claim_id, sequence, _)| ClaimMemberSignature {
                signature: key.sign(&crate::fleet::keys::claim_signature_message(
                    &fleet_id, &self.origin, sequence, &claim_id,
                )),
                member_key: key.public().to_owned(),
                writer: self.origin.clone(),
                sequence,
                claim_id,
            })
            .collect();
        report.signed = signatures.len();
        if report.scanned == 0 {
            return Ok(report);
        }
        self.connection
            .batched(|tx| -> Result<()> {
                for signature in &signatures {
                    insert_signature_tx(tx, signature)?;
                }
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hf875f5474570 owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![SIGN_CURSOR, through.to_string()],
                )?;
                Ok(())
            })
            .map_err(anyhow::Error::msg)??;
        Ok(report)
    }

    /// Keep the writer signatures a peer sent that verify with a key the membership window
    /// admits for that writer and sequence, each for the claim it names only if that claim's batch
    /// is that writer's at that sequence. One for a claim not held here yet waits until it is.
    /// Returns how many it kept; one that does not verify, or names another writer's claim, is
    /// dropped.
    pub fn receive_claim_member_signatures(&self, peer: &str, signatures: &[ClaimMemberSignature]) -> Result<usize> {
        if signatures.is_empty() {
            return Ok(0);
        }
        let (fleet_id, membership) = {
            let connection = self.readers.get();
            let Some(fleet_id) = self.signing_fleet_id(&connection)? else { return Ok(0) };
            (fleet_id, fleet_membership_tx(&connection)?)
        };
        // A signature waits only under a key this member's membership has known: the anchor or
        // any incarnation's. A made-up key is dropped, not parked.
        let known: std::collections::BTreeSet<&str> = membership
            .anchor()
            .into_iter()
            .chain(membership.incarnations().map(|incarnation| incarnation.member_key.as_str()))
            .collect();
        // A signature counts only for a key membership admits for its writer at that sequence.
        // Membership may not know the writer's key yet: the claim that keys it can travel in the
        // same exchange as the signatures. Such a signature waits, and is judged again once the
        // claim has arrived (`promote_claim_member_signatures`); a fenced sequence is dropped.
        let verified: Vec<(&ClaimMemberSignature, bool)> = signatures
            .iter()
            .filter_map(|signature| {
                let admitted = match membership.window(&signature.writer, signature.sequence) {
                    crate::fleet::Window::Keyed(keys) => keys.contains(&signature.member_key),
                    crate::fleet::Window::Legacy => false,
                    crate::fleet::Window::Fenced => return None,
                };
                crate::fleet::verify_signature(
                        &signature.member_key,
                        &crate::fleet::keys::claim_signature_message(
                            &fleet_id, &signature.writer, signature.sequence, &signature.claim_id,
                        ),
                        &signature.signature,
                    )
                    .then_some((signature, admitted))
            })
            .collect();
        if verified.is_empty() {
            return Ok(0);
        }
        let now = i64::try_from(now_ms())?;
        let (per_peer_cap, total_cap) = *CAPS.read().unwrap_or_else(PoisonError::into_inner);
        let mut refused = 0;
        // Every signature of one exchange is decided in one write: an exchange carries the
        // signatures of the envelopes it sends, at most its envelope limit's claims.
        let kept = self.connection
            .batched(|tx| -> Result<usize> {
                let mut kept = 0;
                let mut from_peer: usize = tx.query_row(
                    "SELECT COUNT(*) FROM claim_member_signature_pending WHERE peer=?1",
                    [peer],
                    |row| row.get(0),
                )?;
                let mut total: usize =
                    tx.query_row("SELECT COUNT(*) FROM claim_member_signature_pending", [], |row| row.get(0))?;
                for (signature, admitted) in verified {
                    match claim_writer_tx(tx, &signature.claim_id)? {
                        Some((writer, sequence)) if admitted => {
                            if writer == signature.writer && sequence == signature.sequence {
                                kept += insert_signature_tx(tx, signature)?;
                            }
                        }
                        // Another writer's claim, and nothing can admit it later.
                        Some(_) if !admitted => {}
                        _ if !known.contains(signature.member_key.as_str()) => {}
                        _ if from_peer >= per_peer_cap || total >= total_cap => refused += 1,
                        _ => {
                            from_peer += 1;
                            total += 1;
                            tx.execute(
                                /* sqlite-large-value: allow-for-now lv-h9f2bdd0b1cc1 owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_member_signature_pending(
                                     claim_id, writer, sequence, member_key, signature, received_at_unix_ms, peer)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                                params![signature.claim_id, signature.writer, signature.sequence,
                                    signature.member_key, signature.signature, now, peer],
                            )?;
                        }
                    }
                }
                if refused > 0 {
                    tx.execute(
                        /* sqlite-large-value: allow-for-now lv-hf52799426bd6 owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)
                         ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+CAST(excluded.value AS INTEGER) AS TEXT)",
                        params![REFUSED, refused.to_string()],
                    )?;
                }
                Ok(kept)
            })
            .map_err(anyhow::Error::msg)??;
        if refused > 0 {
            REFUSED_COUNTER.add(refused as u64, &[]);
            eprintln!(
                "st3: refused {refused} waiting claim signatures from {peer}: past {per_peer_cap} from one peer or {total_cap} in all"
            );
        }
        Ok(kept)
    }

    /// How many waiting signatures this member has refused for the caps.
    pub fn claim_member_signatures_refused(&self) -> Result<u64> {
        Ok(self
            .readers
            .get()
            .query_row("SELECT value FROM meta WHERE key=?1", [REFUSED], |row| row.get::<_, String>(0))
            .optional()?
            .map(|value| value.parse::<u64>())
            .transpose()?
            .unwrap_or(0))
    }

    /// Keep the waiting signatures whose claims have arrived and whose batch names the signing
    /// writer and sequence, drop those whose claim names another writer or sequence, and drop
    /// those older than a month whose claim never came. One bounded page; returns how many it
    /// settled.
    pub fn promote_claim_member_signatures(&self) -> Result<usize> {
        // Nothing waits on an idle member: look before taking the writer.
        let waiting: bool = self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM claim_member_signature_pending)",
            [],
            |row| row.get(0),
        )?;
        if !waiting {
            return Ok(0);
        }
        let now = i64::try_from(now_ms())?;
        let keep = i64::try_from(PENDING_KEEP_MS)?;
        let membership = fleet_membership_tx(&self.readers.get())?;
        self.connection
            .batched(|tx| -> Result<usize> {
                // A cursor moves through the waiting rows and wraps, so rows whose claims have not
                // arrived do not hide the rows behind them.
                let after: (String, String) = tx
                    .query_row("SELECT value FROM meta WHERE key=?1", [PENDING_CURSOR], |row| row.get::<_, String>(0))
                    .optional()?
                    .and_then(|value| value.split_once('\n').map(|(a, b)| (a.to_owned(), b.to_owned())))
                    .unwrap_or_default();
                let page = tx
                    .prepare_cached(
                        "SELECT claim_id, writer, sequence, member_key, signature, received_at_unix_ms
                         FROM claim_member_signature_pending WHERE (claim_id, member_key) > (?2, ?3)
                         ORDER BY claim_id, member_key LIMIT ?1",
                    )?
                    .query_map(params![PROMOTE_PAGE, after.0, after.1], |row| {
                        Ok((
                            ClaimMemberSignature {
                                claim_id: row.get(0)?,
                                writer: row.get(1)?,
                                sequence: row.get(2)?,
                                member_key: row.get(3)?,
                                signature: row.get(4)?,
                            },
                            row.get::<_, i64>(5)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut settled = 0;
                for (signature, received) in &page {
                    let decided = match claim_writer_tx(tx, &signature.claim_id)? {
                        // The claim has arrived, and with it its writer's admission if there is
                        // one: keep the signature only if that writer's key is the one admitted.
                        Some((writer, sequence)) => {
                            let admitted = matches!(
                                membership.window(&signature.writer, signature.sequence),
                                crate::fleet::Window::Keyed(keys) if keys.contains(&signature.member_key)
                            );
                            if admitted && writer == signature.writer && sequence == signature.sequence {
                                insert_signature_tx(tx, signature)?;
                            }
                            true
                        }
                        None => now - received > keep,
                    };
                    if decided {
                        tx.execute(
                            /* sqlite-large-value: allow-for-now lv-h82059a59bb28 owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_member_signature_pending WHERE claim_id=?1 AND member_key=?2",
                            params![signature.claim_id, signature.member_key],
                        )?;
                        settled += 1;
                    }
                }
                let next = page.last().map_or_else(String::new, |(last, _)| format!("{}\n{}", last.claim_id, last.member_key));
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hc4a49862daba owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![PENDING_CURSOR, next],
                )?;
                Ok(settled)
            })
            .map_err(anyhow::Error::msg)?
    }

    /// The writer signatures of every claim the envelopes `envelopes` carry, found by each
    /// envelope's batch, for an exchange that sends those envelopes.
    pub fn claim_member_signatures_for_envelopes(
        &self,
        envelopes: &[crate::ReplicaEnvelopeId],
    ) -> Result<Vec<ClaimMemberSignature>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT signatures.claim_id, batches.origin, batches.replica_sequence,
                    signatures.member_key, signatures.signature
             FROM replica_envelopes AS envelopes
             JOIN batches ON batches.id=envelopes.batch_id
             JOIN claims INDEXED BY claims_batch_index ON claims.batch_id=envelopes.batch_id
             JOIN claim_member_signatures AS signatures ON signatures.claim_id=claims.id
             WHERE envelopes.writer=?1 AND envelopes.sequence=?2 AND envelopes.envelope_hash=?3",
        )?;
        let mut found = Vec::new();
        for envelope in envelopes {
            let rows = statement.query_map(params![envelope.writer, envelope.sequence, envelope.hash], |row| {
                Ok(ClaimMemberSignature {
                    claim_id: row.get(0)?,
                    writer: row.get(1)?,
                    sequence: row.get(2)?,
                    member_key: row.get(3)?,
                    signature: row.get(4)?,
                })
            })?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }

    /// Every writer signature this store holds for `claim_ids`, as an exchange carries them.
    pub fn claim_member_signatures_for(&self, claim_ids: &[String]) -> Result<Vec<ClaimMemberSignature>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT signatures.claim_id, batches.origin, batches.replica_sequence,
                    signatures.member_key, signatures.signature
             FROM claim_member_signatures AS signatures
             JOIN claims ON claims.id=signatures.claim_id
             JOIN batches ON batches.id=claims.batch_id
             WHERE signatures.claim_id=?1",
        )?;
        let mut found = Vec::new();
        for claim_id in claim_ids {
            // Every key's signature: a writer whose key rotated signed under each.
            let rows = statement.query_map([claim_id], |row| {
                Ok(ClaimMemberSignature {
                    claim_id: row.get(0)?,
                    writer: row.get(1)?,
                    sequence: row.get(2)?,
                    member_key: row.get(3)?,
                    signature: row.get(4)?,
                })
            })?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }
}

/// The writer and sequence of the batch that holds claim `id`, when this store holds it.
fn claim_writer_tx(connection: &Connection, id: &str) -> Result<Option<(String, u64)>> {
    Ok(connection
        .prepare_cached(
            "SELECT batches.origin, batches.replica_sequence FROM claims
             JOIN batches ON batches.id=claims.batch_id WHERE claims.id=?1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h3044a4adf888 owner=smallclaims::store::member_signatures TODO=trace-statement-and-value-representation */ [id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?)
}

pub(crate) fn insert_signature_tx(connection: &Connection, signature: &ClaimMemberSignature) -> Result<usize> {
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-hcf3fa63cc69a owner=smallclaims::store::member_signatures TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_member_signatures(claim_id, member_key, signature) VALUES (?1, ?2, ?3)",
        params![signature.claim_id, signature.member_key, signature.signature],
    )?)
}
