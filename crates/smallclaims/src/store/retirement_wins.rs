//! A retirement wins (#6878): a claim ID a peer retired deletes the copy this member holds.
//!
//! Members that trim locally retire the claims their cuts drop, and the retired IDs travel in the
//! claim-range walk (`claim_ranges`, the retired namespace): a member records each retired ID a
//! peer holds and it lacks (`retire_learned_claims`). Without this, a member that already held
//! such a claim kept it until its own cut dropped it, so members agreed on their live claims only
//! up to what some member retired.
//!
//! A learned retirement of a claim held here waits in `peer_retired_claims`. One of a kind local
//! trim drops (`LOCAL_TRIM_KINDS`) that no person wrote is deleted by the trim's own guarded
//! chunk path (`trim_checkpoint`): its tombstone is recorded under a cut of its own, then deleted
//! in paced chunks that each check the graph, and a chunk that would change it stops the trim and
//! keeps the claim, as a local cut's does (`local-trim-stopped`). The deletion keeps the ID in
//! this member's retired set. Any other claim is kept and counted: no member deletes a person's
//! claim, or a kind its own trim keeps, on a peer's word. Only an exchange this member admits
//! from an authenticated peer teaches it a retired ID. A held claim that is still the newest of
//! its subject and kind here waits until the newer one a peer's cut kept arrives.
use super::*;
use super::checkpoint::ClaimTombstone;
use super::checkpoint_agreement::CheckpointAction;

/// Whether a member deletes the claims its peers retired. Off until the convergence of mixed
/// trims is proven on running members; it does not depend on releasing envelopes.
pub const RETIREMENT_WINS: bool = false;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hd0a8c2a3ba86 owner=smallclaims::store::retirement_wins TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS peer_retired_claims (
    id TEXT PRIMARY KEY,
    peer TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','kept','unheld')),
    learned_at_unix_ms INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS peer_retired_claims_state ON peer_retired_claims(state, id);
";

/// The most learned retirements one step deletes.
const STEP_CLAIMS: usize = 10_000;

/// The claims a peer's retirement names, of `ids`: a held one `pending` deletion or `kept`, and
/// with `fetch`, one never held here `unheld`, to be asked for so that it is admitted and then
/// retired with its tombstone, as a holder's is. Returns the unheld ones.
pub(crate) fn note_learned_retirements_tx(connection: &Connection, peer: &str, ids: &[String], fetch: bool) -> Result<Vec<String>> {
    let mut held = connection.prepare_cached(
        "SELECT kind, actor FROM claims WHERE id=?1",
    )?;
    let mut insert = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h509d9a962117 owner=smallclaims::store::retirement_wins TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO peer_retired_claims(id, peer, state, learned_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let now = i64::try_from(now_ms())?;
    let mut unheld = Vec::new();
    for id in ids {
        let Some((kind, actor)) = held
            .query_row(/* sqlite-large-value: allow-for-now lv-h555a785f81b8 owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
            .optional()?
        else {
            if fetch && !checkpoint::claim_tombstoned(connection, id)? && insert.execute(/* sqlite-large-value: allow-for-now lv-h81a19713ee79 owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ params![id, peer, "unheld", now])? > 0 {
                unheld.push(id.clone());
            }
            continue;
        };
        let state = if deletable_on_a_peers_word(&kind, actor.as_deref()) { "pending" } else { "kept" };
        insert.execute(/* sqlite-large-value: allow-for-now lv-hc898072723cb owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ params![id, peer, state, now])?;
    }
    Ok(unheld)
}

/// Whether claim `id` is retired here only on a peer's word and never held: admission takes it
/// when a peer supplies it, so this member's trim makes its tombstone.
pub fn retired_unheld(connection: &Connection, id: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT EXISTS(SELECT 1 FROM peer_retired_claims WHERE id=?1 AND state='unheld')")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h2b1ccaa542b2 owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ [id], |row| row.get(0))?)
}

/// Whether a peer's retirement may delete a held claim of `kind` written by `actor`: one local
/// trim drops, that no person wrote.
pub fn deletable_on_a_peers_word(kind: &str, actor: Option<&str>) -> bool {
    checkpoint_trim::LOCAL_TRIM_KINDS.contains(&kind) && !actor.is_some_and(|actor| actor.starts_with("person/"))
}

/// The claims waiting for deletion that this member holds: their tombstones and claim rows. A
/// signed claim whose batch is younger than the freshness window's forget bound (after ?2, from
/// ?3 on) is not among them yet: the deletion would refuse it (`checkpoint::young_signed_claim_tx`),
/// and left in the page it would hold back the claims behind it.
const PENDING_SQL: &str = "SELECT claims.id, claims.subject, claims.kind, claims.actor, claims.predecessors,
        claims.accepted_at_unix_ms, claims.batch_id, batches.origin, batches.replica_sequence, claims.body
     FROM peer_retired_claims AS learned INDEXED BY peer_retired_claims_state
     JOIN claims ON claims.id=learned.id
     JOIN batches ON batches.id=claims.batch_id
     WHERE learned.state='pending'
       AND NOT (CAST(batches.accepted_at_unix_ms AS INTEGER) > ?2 AND CAST(batches.accepted_at_unix_ms AS INTEGER) >= ?3
                AND EXISTS(SELECT 1 FROM claim_signatures AS signed WHERE signed.claim_id=claims.id))
     ORDER BY learned.id LIMIT ?1";

impl Store {
    /// Whether this member deletes what its peers retired: [`RETIREMENT_WINS`], or what a test
    /// set.
    pub fn retirement_wins(&self) -> bool {
        self.retirement_wins.load(Ordering::Relaxed)
    }

    /// Turn deleting what peers retired on or off, for a test.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_retirement_wins(&self, on: bool) {
        self.retirement_wins.store(on, Ordering::Relaxed);
    }

    /// How many claims peers retired that this member never held and no peer has supplied: it
    /// lacks their tombstones.
    pub fn peer_retired_claims_unheld(&self) -> Result<u64> {
        Ok(self.readers.get().query_row(
            "SELECT COUNT(*) FROM peer_retired_claims WHERE state='unheld'",
            [],
            |row| row.get(0),
        )?)
    }

    /// How many held claims a peer retired that this member keeps: a person's, or of a kind its
    /// own trim keeps.
    pub fn peer_retired_claims_kept(&self) -> Result<u64> {
        Ok(self.readers.get().query_row(
            "SELECT COUNT(*) FROM peer_retired_claims WHERE state='kept'",
            [],
            |row| row.get(0),
        )?)
    }

    /// Delete the held claims peers retired, through the trim's guarded chunk path, under a cut
    /// of their own. Nothing runs while a trim is halted or under way, or past the day's cap. A
    /// claim still the newest of its subject and kind here waits.
    pub(crate) fn trim_peer_retired(&self, actions: &mut Vec<CheckpointAction>) -> Result<()> {
        if !self.retirement_wins() || self.checkpoint_halted()? {
            return Ok(());
        }
        // A claim a peer supplied after its retirement was learned is held now: it waits for
        // deletion, or is kept, as one held all along. One this member trimmed itself since is
        // done.
        {
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            let supplied = transaction
                .prepare_cached(
                    "SELECT learned.id, claims.kind, claims.actor FROM peer_retired_claims AS learned
                     JOIN claims ON claims.id=learned.id WHERE learned.state='unheld'",
                )?
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, kind, actor) in supplied {
                let state = if deletable_on_a_peers_word(&kind, actor.as_deref()) { "pending" } else { "kept" };
                transaction.execute(/* sqlite-large-value: allow-for-now lv-h2e9891a08bde owner=smallclaims::store::retirement_wins TODO=bound-or-reference-values-and-reclamation */ "UPDATE peer_retired_claims SET state=?2 WHERE id=?1", params![id, state])?;
            }
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h7f55b523e25e owner=smallclaims::store::retirement_wins TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM peer_retired_claims WHERE state='unheld'
                   AND EXISTS(SELECT 1 FROM checkpoint_claims WHERE checkpoint_claims.id=peer_retired_claims.id)",
                [],
            )?;
            transaction.commit()?;
        }
        let trimming: bool = self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM checkpoints WHERE state='trimming')",
            [],
            |row| row.get(0),
        )?;
        let left = self.local_trim_left_today()?;
        if trimming || left == 0 {
            return Ok(());
        }
        // The forget bound, by the clock the deletion judges it with: a later step takes what waits.
        let (youngest_forgettable, first_held) = {
            use crate::principal::{FORGET_SIGNED_AFTER_MS, FRESHNESS_FROM_UNIX_MS, FRESHNESS_WINDOW_MS};
            let now = super::checkpoint::forget_clock(&self.connection.write());
            (
                i64::try_from(now.saturating_sub(FORGET_SIGNED_AFTER_MS)).unwrap_or(i64::MAX),
                i64::try_from(FRESHNESS_FROM_UNIX_MS.saturating_sub(FRESHNESS_WINDOW_MS)).unwrap_or(i64::MAX),
            )
        };
        let rows = {
            let connection = self.readers.get();
            let mut envelope = connection.prepare_cached(
                "SELECT writer, sequence, envelope_hash FROM envelope_rows WHERE batch_id=?1 LIMIT 1",
            )?;
            let rows = connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h60e0bfc24ee7 owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ PENDING_SQL)?
                .query_map(params![usize::try_from(left).unwrap_or(usize::MAX).min(STEP_CLAIMS), youngest_forgettable, first_held], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, u64>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut tombstones = Vec::with_capacity(rows.len());
            for (id, subject, kind, actor, predecessors, accepted, batch, origin, sequence, body) in rows {
                let body: Value = serde_json::from_str(&body)?;
                let operation = operation_parts(&body);
                let (writer, sequence, envelope_hash) = envelope
                    .query_row(/* sqlite-large-value: allow-for-now lv-h326b9cbb525b owner=smallclaims::store::retirement_wins TODO=trace-statement-and-value-representation */ [&batch], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .optional()?
                    .unwrap_or((origin, sequence, String::new()));
                tombstones.push(ClaimTombstone {
                    id,
                    writer,
                    sequence,
                    envelope_hash,
                    subject,
                    kind,
                    actor,
                    predecessors: serde_json::from_str(&predecessors)?,
                    operation_id: operation.map(|(id, _)| id.to_owned()),
                    request_digest: operation.map(|(_, digest)| digest.to_owned()),
                    accepted_at_unix_ms: accepted.parse().unwrap_or_default(),
                });
            }
            tombstones
        };
        // The newer claim a peer's cut kept may not be here yet; deleting the newest of a subject
        // and kind would change what its readers answer, so it waits for that one.
        let mut claims = Vec::with_capacity(rows.len());
        for tombstone in rows {
            let newest = self.latest_claim(&tombstone.subject, Some(&tombstone.kind))?;
            if newest.is_some_and(|newest| newest.id != tombstone.id) {
                claims.push(tombstone);
            }
        }
        if !claims.is_empty() {
            // Named by what it deletes, so the same retirements make the same cut on every run.
            let digest = crate::hash::canonical_hash(&claims.iter().map(|claim| &claim.id).collect::<Vec<_>>())?;
            let checkpoint = format!("checkpoint/retired-by-peers/{}", &digest[..16.min(digest.len())]);
            // A cut of time zero: the cut this member's own trim compares is never moved by it.
            self.trim_checkpoint(&checkpoint, 0, &digest, &[], &claims, actions)?;
        }
        // What is gone now, by this deletion or any other, no longer waits.
        self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h48a82e903143 owner=smallclaims::store::retirement_wins TODO=prepare-body-and-predecessor-values */ "DELETE FROM peer_retired_claims WHERE state='pending'
               AND NOT EXISTS(SELECT 1 FROM claims WHERE claims.id=peer_retired_claims.id)",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;

    /// The waiting retirements are read by their state's index, each claim and batch by its key.
    #[test]
    fn the_pending_retirements_seek_and_scan_nothing() {
        let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
        envelope_release::assert_plan_seeks(&store.connection.write(), PENDING_SQL);
    }

    #[test]
    fn only_a_trimmed_kind_no_person_wrote_goes_on_a_peers_word() {
        assert!(deletable_on_a_peers_word("observer.observed", None));
        assert!(deletable_on_a_peers_word("harness.observed", Some("agent/sample")));
        assert!(!deletable_on_a_peers_word("observer.observed", Some("person/example")));
        assert!(!deletable_on_a_peers_word("message.sent", None));
    }
}
