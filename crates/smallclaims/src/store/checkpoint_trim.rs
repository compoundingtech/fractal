//! Trimming: deleting what the newest stable checkpoint drops, and adopting a checkpoint this
//! node did not verify from a peer's manifest. Sections 6 and 8 of
//! `doc/fleet/smalltalk/checkpoint-design`.
//!
//! Tombstones go first, in chunks, with the checkpoint marked `recording`; once every one is
//! recorded it is marked `trimming`. Deletions follow in chunks, each its own transaction. A
//! sealed set never reads the tombstones of a checkpoint still `recording`, so a crash part way
//! through recording plans the same drop again and records the rest; a crash after it deletes
//! what is left. At every point the inventory lists the same identities, since a tombstone and
//! the envelope it stands for have one identity, so peers cannot tell.

use super::checkpoint::{
    CheckpointManifest, ClaimTombstone, EnvelopeTombstone, checkpoint_name, delete_dropped_rows_tx,
    record_checkpoint_tombstones_tx, verify_checkpoint_manifest,
};
use super::checkpoint_agreement::{
    Certificate, CheckpointAction, CheckpointClaim, certificates, chosen_certificate,
    stable_checkpoints,
};
use super::*;
use crate::replication::InventoryCheckpoint;

/// The kinds a member that trims locally drops, each by its audited rule in
/// `crates/fractal-schema/retention.toml` and with a test that every reader of it answers the
/// same from the full and the trimmed log. A kind joins only with such a test. `resource.observed`
/// keeps what its readers walk back through, such as a pull request's observations back to the
/// merge queue (rules version 23). `harness.observed` joined once its rule kept the credential
/// report where a seat's needs-login run begins (see #4237).
pub const LOCAL_TRIM_KINDS: &[&str] = &["observer.observed", "harness.observed", "resource.observed"];

/// The kinds a local cut's capture also stages, because judging a claim of `LOCAL_TRIM_KINDS`
/// reads them: the status-history rule folds a seat's harness observations with its runtime
/// resets and harness diagnostics, and keeps a transition a reset or a restored prompt depends
/// on. They are staged as context and protected like every other kind, never dropped.
pub const LOCAL_TRIM_CONTEXT_KINDS: &[&str] = &["runtime.observed", "harness.diagnostic"];

/// What a local cut's capture stages: the kinds it drops and the kinds judging them reads.
pub fn local_trim_capture_kinds() -> Vec<&'static str> {
    LOCAL_TRIM_KINDS.iter().chain(LOCAL_TRIM_CONTEXT_KINDS).copied().collect()
}

/// At most this many envelopes deleted per transaction.
pub const TRIM_CHUNK_ENVELOPES: usize = 2_000;

/// The claims a member that trims locally deletes in one day unless `[trim] daily_claims` says
/// otherwise: about 1.7 h of paced deletion at the 16.6 claims a second measured on a 53 GB
/// member, whose first cut dropped 390,000 claims.
pub const LOCAL_TRIM_DAILY_CLAIMS: u64 = 100_000;

/// `meta`: the day a member that trims locally last deleted on, and how many claims it deleted
/// that day, as `DAY CLAIMS`. Each chunk updates it in the transaction that deletes.
pub const LOCAL_TRIM_DAY: &str = "local_trim_day";

/// Stop starting new trim steps after this much work. A single step or COMMIT may take longer;
/// this is not a hard bound on writer hold or foreground request latency.
pub const TRIM_CHUNK_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);

// Releasing a writer loan alone does not reserve any disk time: a quiet foreground queue
// lets a large trim reacquire it immediately, continuously rewriting table/index/WAL pages.
// Rest outside the loan for at least 100 ms and four times the preceding writer hold
// (including COMMIT), capped at five seconds per rest. In the uncapped regime this reserves
// four times the acquired-loan work for other traffic. A slow statement can still exceed the
// chunk budget; this pacing is not a hard statement or request-latency bound.
pub(super) const TRIM_MIN_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// A trim chunk that holds the writer longer than this logs one line saying what it did.
pub const LOGGED_TRIM_CHUNK: std::time::Duration = std::time::Duration::from_secs(1);

/// Log one line for a trim chunk held over `LOGGED_TRIM_CHUNK`: its phase, the envelopes and
/// claims it took, the bytes of the claim bodies it deleted, and its hold split into the steps
/// and the commit, so a member attributes its own outliers.
fn log_if_slow_trim_chunk(
    phase: &str,
    held: std::time::Duration,
    commit: std::time::Duration,
    envelopes: usize,
    claims: usize,
    body_bytes: u64,
) {
    if held <= LOGGED_TRIM_CHUNK {
        return;
    }
    tracing::warn!(
        phase,
        held_ms = held.as_millis() as u64,
        step_ms = held.saturating_sub(commit).as_millis() as u64,
        commit_ms = commit.as_millis() as u64,
        envelopes,
        claims,
        body_bytes,
        "a trim chunk held the store's writer over 1 s",
    );
}

/// The bytes of the bodies of `claims` that this store still holds.
fn claim_body_bytes(transaction: &Transaction<'_>, claims: &[ClaimTombstone]) -> Result<u64> {
    claims.iter().try_fold(0, |bytes, claim| {
        let body: Option<i64> = transaction
            .query_row("SELECT LENGTH(body) FROM claims WHERE id=?1", params![claim.id], |row| row.get(0))
            .optional()?;
        Ok(bytes + body.map_or(0, |body| u64::try_from(body).unwrap_or(0)))
    })
}

/// A condition a test injects into a trim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrimFault {
    /// This many chunks of tombstones are recorded, and the checkpoint is still `recording`.
    AfterTombstoneChunk(usize),
    /// Tombstones are recorded and nothing is deleted yet.
    AfterTombstones,
    /// This many chunks are deleted.
    AfterChunk(usize),
    /// This many pages of a local cut's citation catch-up are committed.
    AfterCitationPages(usize),
    /// Every row is deleted and the checkpoint is not yet marked trimmed.
    BeforeFinish,
    /// The first deletion chunk starts with its time budget already spent.
    #[cfg(test)]
    ExpireChunkBudget,
    /// The first chunk bumps the graph generation without changing the graph.
    TouchGraph,
    /// The first chunk changes a graph table, as a trim must never do.
    ChangeGraph,
    /// A local cut's plan drops the newest claim of a subject and kind, as a planner defect
    /// would.
    DropNewest,
}

/// The newest stable checkpoint, when this node cannot trim it from its own plan. A peer that
/// trimmed it serves its manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifestNeed {
    pub checkpoint: String,
    pub cut_unix_ms: u128,
    pub drop_digest: String,
}

pub fn local_checkpoint(
    connection: &Connection,
    checkpoint: &str,
) -> Result<Option<(String, Option<i64>)>> {
    Ok(connection
        .query_row(
            "SELECT state, seal_rowid FROM checkpoints WHERE id=?1",
            [checkpoint],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// Whether this node has applied the certificate with `drop_digest` for the cut, or anything
/// newer, or set the cut aside. A node that applied the other side's certificate for the same
/// cut has not applied this one.
pub fn applied(connection: &Connection, cut_unix_ms: u128, drop_digest: &str) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM checkpoints
             WHERE (state='trimmed'
                    AND (cut_unix_ms > ?1 OR (cut_unix_ms = ?1 AND drop_digest = ?2)))
                OR (state IN ('set-aside', 'graph-changed') AND cut_unix_ms >= ?1)
             LIMIT 1",
            params![i64::try_from(cut_unix_ms)?, drop_digest],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Delete every tombstone that is not among `envelopes` and `claims`.
pub fn keep_only_tombstones_tx(
    transaction: &Transaction<'_>,
    envelopes: &[EnvelopeTombstone],
    claims: &[ClaimTombstone],
) -> Result<()> {
    transaction.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h41059b0e7a62 owner=smallclaims::store::checkpoint_trim TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE IF NOT EXISTS adopted_envelopes(
             writer TEXT NOT NULL, sequence INTEGER NOT NULL, envelope_hash TEXT NOT NULL,
             PRIMARY KEY (writer, sequence, envelope_hash));
         CREATE TEMP TABLE IF NOT EXISTS adopted_claims(id TEXT PRIMARY KEY);
         DELETE FROM temp.adopted_envelopes;
         DELETE FROM temp.adopted_claims;",
    )?;
    {
        let mut insert = transaction.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h606a2ea2c17e owner=smallclaims::store::checkpoint_trim TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO temp.adopted_envelopes(writer, sequence, envelope_hash)
             VALUES (?1, ?2, ?3)",
        )?;
        for envelope in envelopes {
            insert.execute(/* sqlite-large-value: allow-for-now lv-h602112ce8703 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![
                envelope.writer,
                envelope.sequence,
                envelope.envelope_hash
            ])?;
        }
        let mut insert = transaction
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hf56d2d99513a owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO temp.adopted_claims(id) VALUES (?1)")?;
        for claim in claims {
            insert.execute(/* sqlite-large-value: allow-for-now lv-ha5bb9a6450df owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [&claim.id])?;
        }
    }
    transaction.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hb829e6623715 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_envelopes WHERE NOT EXISTS (
             SELECT 1 FROM temp.adopted_envelopes AS adopted
             WHERE adopted.writer=checkpoint_envelopes.writer
               AND adopted.sequence=checkpoint_envelopes.sequence
               AND adopted.envelope_hash=checkpoint_envelopes.envelope_hash);
         DELETE FROM checkpoint_claims WHERE id NOT IN (SELECT id FROM temp.adopted_claims);
         DELETE FROM temp.adopted_envelopes;
         DELETE FROM temp.adopted_claims;",
    )?;
    Ok(())
}

/// How this node applies the newest stable checkpoint.
pub enum Application {
    /// It already did.
    Done,
    /// It verified the certificate every node applies, so it trims from its own plan.
    OwnPlan {
        checkpoint: String,
        certificate: Box<Certificate>,
        seal_rowid: i64,
    },
    /// It adopts the certificate's manifest from a peer.
    Manifest(CheckpointManifestNeed),
}

/// Tombstones a trim reads at most this many at a time.
const TOMBSTONE_READ_PAGE: usize = 256;

/// The tombstones a trim records: a manifest's in memory, or this node's own staged plan.
trait Tombstones {
    fn counts(&self) -> (usize, usize);
    fn envelope_page(&self, after: usize, limit: usize) -> Result<Vec<EnvelopeTombstone>>;
    fn claim_page(&self, after: usize, limit: usize) -> Result<Vec<ClaimTombstone>>;
    fn slices(&self) -> Option<(&[EnvelopeTombstone], &[ClaimTombstone])> {
        None
    }
}

impl Tombstones for (&[EnvelopeTombstone], &[ClaimTombstone]) {
    fn counts(&self) -> (usize, usize) {
        (self.0.len(), self.1.len())
    }

    fn envelope_page(&self, after: usize, limit: usize) -> Result<Vec<EnvelopeTombstone>> {
        Ok(self.0.iter().skip(after).take(limit).cloned().collect())
    }

    fn claim_page(&self, after: usize, limit: usize) -> Result<Vec<ClaimTombstone>> {
        Ok(self.1.iter().skip(after).take(limit).cloned().collect())
    }

    fn slices(&self) -> Option<(&[EnvelopeTombstone], &[ClaimTombstone])> {
        Some((self.0, self.1))
    }
}

impl Tombstones for (&super::checkpoint_staging::StagedSet, &super::checkpoint_staging::StagedPlan) {
    fn counts(&self) -> (usize, usize) {
        (self.1.dropped_envelopes, self.1.dropped_claims)
    }

    fn envelope_page(&self, after: usize, limit: usize) -> Result<Vec<EnvelopeTombstone>> {
        self.0.dropped_envelopes(after, limit)
    }

    fn claim_page(&self, after: usize, limit: usize) -> Result<Vec<ClaimTombstone>> {
        self.0.dropped_claims(after, limit)
    }
}

impl Store {
    /// Set the minimum chunk rest. Tests only; zero disables both the floor and multiplier.
    pub fn set_trim_pause(&self, pause: std::time::Duration) {
        self.trim_pause_micros.store(
            u64::try_from(pause.as_micros()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }

    fn trim_pause_duration(&self, held: std::time::Duration) -> std::time::Duration {
        let micros = self.trim_pause_micros.load(Ordering::Acquire);
        if micros == 0 {
            return std::time::Duration::ZERO;
        }
        held.saturating_mul(4)
            .max(std::time::Duration::from_micros(micros))
            .min(std::time::Duration::from_secs(5))
    }

    fn pause_trim_chunk(&self, held: std::time::Duration) {
        std::thread::sleep(self.trim_pause_duration(held));
    }

    /// Arm a fault in the next trim. Tests only.
    pub fn set_trim_fault(&self, fault: Option<TrimFault>) {
        *self
            .trim_fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = fault;
    }

    /// Delete this many envelopes per transaction. Tests only.
    pub fn set_trim_chunk_envelopes(&self, envelopes: usize) {
        self.trim_chunk_envelopes
            .store(envelopes.max(1), Ordering::Release);
    }

    /// Make deleting each envelope or claim take this much longer, as a slow store would. Tests
    /// only.
    pub fn set_trim_row_cost(&self, cost: std::time::Duration) {
        self.trim_row_cost_micros.store(
            u64::try_from(cost.as_micros()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }

    fn pause_for_trim_row_cost(&self) {
        let micros = self.trim_row_cost_micros.load(Ordering::Acquire);
        if micros > 0 {
            std::thread::sleep(std::time::Duration::from_micros(micros));
        }
    }

    /// Apply a graph fault a test armed, inside the first chunk's transaction.
    pub fn alter_graph_for_trim_fault(&self, transaction: &Transaction<'_>) -> Result<()> {
        let mut fault = self
            .trim_fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match *fault {
            Some(TrimFault::TouchGraph) => {
                transaction.execute(/* sqlite-large-value: allow-for-now lv-ha9cdb844c613 owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "UPDATE projection_digest_generation SET value=value+1", [])?;
            }
            Some(TrimFault::ChangeGraph) => {
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-hef95e4dbb508 owner=smallclaims::store::checkpoint_trim TODO=prepare-body-and-predecessor-values */ "INSERT INTO desired(subject, kind, revision, claim_id, body)
                     SELECT 'custom/trim-fault/changed', 'custom', '1', id, '{}'
                     FROM claims LIMIT 1",
                    [],
                )?;
            }
            _ => return Ok(()),
        }
        *fault = None;
        Ok(())
    }

    pub fn stop_for_trim_fault(&self, at: TrimFault) -> Result<()> {
        let mut fault = self
            .trim_fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *fault == Some(at) {
            *fault = None;
            anyhow::bail!("the trim stopped at {at:?} for a test");
        }
        Ok(())
    }

    pub fn newest_application(&self, claims: &[CheckpointClaim]) -> Result<Option<Application>> {
        let stable = stable_checkpoints(claims);
        let Some((cut, certificates)) = stable.into_iter().next_back() else {
            return Ok(None);
        };
        let certificate = chosen_certificate(&certificates)
            .expect("a stable checkpoint has a certificate")
            .clone();
        let checkpoint = checkpoint_name(cut);
        let connection = self.readers.get();
        if applied(&connection, cut, &certificate.terms.drop_digest)? {
            return Ok(Some(Application::Done));
        }
        if certificate.verifications.contains_key(&self.origin)
            && let Some((state, Some(seal_rowid))) = local_checkpoint(&connection, &checkpoint)?
            && state != "plan-changed"
        {
            return Ok(Some(Application::OwnPlan {
                checkpoint,
                certificate: Box::new(certificate),
                seal_rowid,
            }));
        }
        Ok(Some(Application::Manifest(CheckpointManifestNeed {
            checkpoint,
            cut_unix_ms: cut,
            drop_digest: certificate.terms.drop_digest,
        })))
    }

    /// Finish a trim a crash interrupted, then apply the newest stable checkpoint if this node
    /// has not: trim it from its own plan when it verified the certificate every node applies,
    /// or report that it needs that certificate's manifest. Until it is applied, this node must
    /// not verify a newer checkpoint, because every node trims the same checkpoints in the same
    /// order.
    pub fn apply_stable_checkpoints(
        &self,
        claims: &[CheckpointClaim],
        scratch: &Path,
        actions: &mut Vec<CheckpointAction>,
    ) -> Result<Option<CheckpointManifestNeed>> {
        self.runtime.checkpoint_preflight()?;
        self.finish_interrupted_trims(actions)?;
        match self.newest_application(claims)? {
            None | Some(Application::Done) => Ok(None),
            Some(Application::Manifest(need)) => Ok(Some(need)),
            Some(Application::OwnPlan {
                checkpoint,
                certificate,
                seal_rowid,
            }) => {
                let _completion = super::checkpoint_completion::Completion::outer();
                let cut = certificate.terms.cut_unix_ms;
                // The staged plan this node verified, when its store-index cut still holds: the
                // file's identity fixes the epoch, rules and executable, so a repair, a late
                // envelope or a rules change since makes it capture and plan again. A plan that
                // differs from the certificate every participant verified is never trimmed.
                let (staged, plan) = {
                    let _span = crate::profile::span("trim.capture_plan");
                    let staged = self.checkpoint_staged_set(cut, Some(seal_rowid), scratch)?;
                    let plan = self.plan_staged_checkpoint(&staged)?;
                    (staged, plan)
                };
                if plan.sealed_digest == certificate.terms.sealed_digest
                    && plan.drop_digest == certificate.terms.drop_digest
                {
                    self.trim_staged_checkpoint(&checkpoint, &staged, &plan, actions)?;
                    staged.remove()?;
                    return Ok(None);
                }
                staged.remove()?;
                // Its own rows no longer give the drop it verified, so it adopts the manifest
                // from a peer like a node that did not take part. Tombstones a trim interrupted
                // while recording go first: nothing reads them until the trim finishes, and
                // they must not outlive the plan they came from.
                self.forget_recording_tombstones(&checkpoint)?;
                self.set_checkpoint_state(&checkpoint, cut, "plan-changed")?;
                self.checkpoint_diagnostic(
                    "checkpoint-trim-plan-changed",
                    &format!(
                        "this node no longer plans the drop it verified for {checkpoint}; it \
                         adopts the manifest from a peer instead"
                    ),
                    &checkpoint,
                )?;
                Ok(Some(CheckpointManifestNeed {
                    checkpoint,
                    cut_unix_ms: cut,
                    drop_digest: certificate.terms.drop_digest,
                }))
            }
        }
    }

    /// Finish every trim a crash interrupted after its tombstones committed (`trimming`). A
    /// checkpoint still `recording` is not committed and stays as it is.
    pub(crate) fn finish_interrupted_trims(&self, actions: &mut Vec<CheckpointAction>) -> Result<()> {
        let interrupted = {
            let connection = self.readers.get();
            connection
                .prepare_cached("SELECT id FROM checkpoints WHERE state='trimming' ORDER BY id")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let _span = crate::profile::span("trim.resume");
        for checkpoint in interrupted {
            self.finish_trim(&checkpoint, actions)?;
        }
        Ok(())
    }

    /// Trim this member's own due cut, while it trims locally: capture the set before the cut,
    /// plan its drop with the same rules and guards a shared checkpoint uses, and trim it. There
    /// is no seal, verification or copy-proof. Two checks stand in for the proof: the plan is
    /// refused whole, before anything is recorded, if it would drop the newest claim of any
    /// subject and kind; and `finish_trim` compares the projection digests around every chunk
    /// it deletes and stops at the first change. The cut moves only forward, once a day; a
    /// stopped trim halts the next ones until it is resumed.
    pub fn trim_local_cut(
        &self,
        context: &super::checkpoint_agreement::CheckpointContext,
        actions: &mut Vec<CheckpointAction>,
    ) -> Result<()> {
        if self.checkpoint_halted()? {
            return Ok(());
        }
        let cut = super::checkpoint::newest_due_cut(context.now_unix_ms);
        let trimmed: Option<i64> = self.readers.get().query_row(
            // A cut set aside after a stop is done too; the next day's cut goes on.
            "SELECT MAX(cut_unix_ms) FROM checkpoints WHERE state IN ('trimming', 'trimmed', 'set-aside')",
            [],
            |row| row.get(0),
        )?;
        if cut == 0 || trimmed.is_some_and(|trimmed| u128::try_from(trimmed).unwrap_or(0) >= cut) {
            return self.remove_checkpoint_staging(&context.scratch);
        }
        // An earlier cut the daily cap stopped finishes first, and a cut that could delete
        // nothing today waits for tomorrow rather than record its tombstones now.
        let trimming: bool = self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM checkpoints WHERE state='trimming')",
            [],
            |row| row.get(0),
        )?;
        if trimming || self.local_trim_left_today()? == 0 {
            return Ok(());
        }
        let checkpoint = checkpoint_name(cut);
        // A shared checkpoint this member was still recording before it chose local trim was
        // never a deletion; its candidates must not outlive that plan.
        if local_checkpoint(&self.readers.get(), &checkpoint)?
            .is_some_and(|(state, _)| state == "recording")
        {
            self.forget_recording_tombstones(&checkpoint)?;
        }
        let _completion = super::checkpoint_completion::Completion::outer();
        // Only what judging the kinds it drops reads: the plan is the same as a full capture's.
        let staged = self.checkpoint_staged_set_scoped(cut, None, &context.scratch, Some(&local_trim_capture_kinds()))?;
        staged.protect_all_but(LOCAL_TRIM_KINDS)?;
        staged.split_envelopes()?;
        // The in-memory reference planner tests builds compare with knows no kind restriction.
        #[cfg(any(test, feature = "test-support"))]
        let plan = super::checkpoint_staging::without_reference_plan(|| self.plan_staged_checkpoint(&staged))?;
        #[cfg(not(any(test, feature = "test-support")))]
        let plan = self.plan_staged_checkpoint(&staged)?;
        {
            let mut fault = self.trim_fault.lock().unwrap_or_else(PoisonError::into_inner);
            if *fault == Some(TrimFault::DropNewest) {
                *fault = None;
                staged.drop_newest_for_test()?;
            }
        }
        if let Some(claim) = staged.dropped_newest_claim()? {
            staged.remove()?;
            self.set_checkpoint_state(&checkpoint, cut, "graph-changed")?;
            self.checkpoint_diagnostic(
                "local-trim-would-drop-newest",
                &format!(
                    "the plan for {checkpoint} drops claim {claim}, the newest of its subject and \
                     kind; nothing is deleted and no local trim runs until it is resumed"
                ),
                &checkpoint,
            )?;
            actions.push(CheckpointAction::TrimGraphChanged { checkpoint });
            return Ok(());
        }
        self.trim_staged_checkpoint(&checkpoint, &staged, &plan, actions)?;
        staged.remove()
    }

    /// The newest stable checkpoint this node needs a manifest for, if any. The replication
    /// worker asks when a peer advertises a checkpoint this node has not applied, and fetches
    /// the manifest from a peer that applied this one. A member that trims locally needs none.
    pub fn checkpoint_manifest_need(&self) -> Result<Option<CheckpointManifestNeed>> {
        self.runtime.checkpoint_preflight()?;
        if self.trims_locally() {
            return Ok(None);
        }
        let claims = self.checkpoint_claims()?;
        Ok(match self.newest_application(&claims)? {
            Some(Application::Manifest(need)) => Some(need),
            _ => None,
        })
    }

    /// The newest checkpoint this node has trimmed or adopted, as its inventory advertises it.
    pub fn trimmed_checkpoint(&self) -> Result<Option<InventoryCheckpoint>> {
        Ok(self
            .readers
            .get()
            .prepare_cached(
                "SELECT id, cut_unix_ms, drop_digest FROM checkpoints
                 WHERE state='trimmed' AND drop_digest IS NOT NULL
                 ORDER BY cut_unix_ms DESC LIMIT 1",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h138e37a621ed owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [], |row| {
                Ok(InventoryCheckpoint {
                    id: row.get(0)?,
                    cut_unix_ms: u128::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                    drop_digest: row.get(2)?,
                })
            })
            .optional()?)
    }

    /// Adopt a stable checkpoint from a peer's manifest. The whole manifest is checked against
    /// the drop digest every participant of the chosen certificate verified before anything
    /// from it is stored. Afterwards this node's tombstones are exactly the manifest's, so it
    /// goes on like every node that applied the same certificate.
    pub fn adopt_checkpoint(
        &self,
        manifest: &CheckpointManifest,
    ) -> Result<Vec<CheckpointAction>, St3Error> {
        self.runtime.checkpoint_preflight().map_err(internal)?;
        if self.trims_locally() {
            return Err(St3Error::new(
                "trim-is-local",
                "this member trims locally ([trim] local), so it adopts no checkpoint manifest",
            ));
        }
        self.adopt_checkpoint_checked(manifest, true)
    }

    /// Initialize checkpoint history in a fresh offline restore after admitting its claim log.
    /// The source may have learned a newer certificate without applying it yet; its recorded
    /// tombstones still belong to the older certified manifest. This never replaces existing
    /// checkpoint state, and verifies the manifest through the same certificate path as sync.
    pub /* sqlite-large-value-flow: allow-for-now flow-h3436182aecc1 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ fn restore_checkpoint_history(
        &self,
        manifest: &CheckpointManifest,
    ) -> Result<Vec<CheckpointAction>, St3Error> {
        self.runtime.checkpoint_preflight().map_err(internal)?;
        let occupied: bool = self
            .readers
            .get()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkpoints)
                 OR EXISTS(SELECT 1 FROM checkpoint_envelopes)
                 OR EXISTS(SELECT 1 FROM checkpoint_claims)",
                [],
                |row| row.get(0),
            )
            .map_err(internal)?;
        if occupied {
            return Err(St3Error::new(
                "checkpoint-history-exists",
                "checkpoint restore requires empty checkpoint state",
            ));
        }
        self.adopt_checkpoint_checked(manifest, false)
    }

    fn adopt_checkpoint_checked(
        &self,
        manifest: &CheckpointManifest,
        require_newest: bool,
    ) -> Result<Vec<CheckpointAction>, St3Error> {
        let _completion = super::checkpoint_completion::Completion::outer();
        self.runtime.checkpoint_preflight().map_err(internal)?;
        let claims = self.checkpoint_claims().map_err(internal)?;
        // Adoption replaces every tombstone this node holds, so only the newest stable
        // checkpoint may be adopted: an older manifest lacks the newer drops.
        if require_newest
            && stable_checkpoints(&claims)
                .keys()
                .next_back()
                .is_some_and(|newest| *newest > manifest.cut_unix_ms)
        {
            return Err(St3Error::new(
                "checkpoint-superseded",
                format!(
                    "`{}` is older than the newest stable checkpoint here; a node adopts only \
                     the newest",
                    manifest.checkpoint
                ),
            ));
        }
        let certificates = certificates(&claims, &manifest.checkpoint);
        let Some(certificate) = chosen_certificate(&certificates) else {
            return Err(St3Error::new(
                "checkpoint-not-stable",
                format!(
                    "`{}` is not stable on this node; it adopts a checkpoint once it holds \
                     every participant's verification",
                    manifest.checkpoint
                ),
            ));
        };
        if certificate.terms.cut_unix_ms != manifest.cut_unix_ms {
            return Err(St3Error::new(
                "checkpoint-manifest-invalid",
                "the manifest's cut is not the certified one",
            ));
        }
        verify_checkpoint_manifest(manifest, &certificate.terms.drop_digest)?;
        let mut actions = Vec::new();
        let done = applied(
            &self.readers.get(),
            manifest.cut_unix_ms,
            &certificate.terms.drop_digest,
        )
        .map_err(internal)?;
        if done {
            return Ok(actions);
        }
        self.trim_checkpoint(
            &manifest.checkpoint,
            manifest.cut_unix_ms,
            &certificate.terms.drop_digest,
            &manifest.envelopes,
            &manifest.claims,
            true,
            &mut actions,
        )
        .map_err(internal)?;
        Ok(actions)
    }

    /// A deliberate bug for the convergence suite: adopt a manifest without checking it
    /// against its certificate. Nothing else calls it.
    #[doc(hidden)]
    pub fn adopt_checkpoint_unverified_for_tests(
        &self,
        manifest: &CheckpointManifest,
    ) -> Result<Vec<CheckpointAction>> {
        self.runtime.checkpoint_preflight()?;
        let claims = self.checkpoint_claims()?;
        let certificates = certificates(&claims, &manifest.checkpoint);
        let certificate = chosen_certificate(&certificates)
            .ok_or_else(|| anyhow::anyhow!("{} is not stable here", manifest.checkpoint))?;
        let mut actions = Vec::new();
        self.trim_checkpoint(
            &manifest.checkpoint,
            manifest.cut_unix_ms,
            &certificate.terms.drop_digest,
            &manifest.envelopes,
            &manifest.claims,
            true,
            &mut actions,
        )?;
        Ok(actions)
    }

    /// A deliberate bug for the convergence suite: forget every tombstone, as a trim that
    /// deleted rows without keeping tombstones would. Returns how many it forgot. Nothing else
    /// calls it.
    #[doc(hidden)]
    pub fn forget_tombstones_for_tests(&self) -> Result<usize> {
        self.runtime.checkpoint_preflight()?;
        let forgotten = {
            let connection = self.connection.write();
            connection.execute(/* sqlite-large-value: allow-for-now lv-h5875e9ada089 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_envelopes", [])?
                + connection.execute(/* sqlite-large-value: allow-for-now lv-h093f16c5a2a5 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "DELETE FROM checkpoint_claims", [])?
        };
        self.replica_rows_changed();
        Ok(forgotten)
    }

    /// Delete the tombstones of `checkpoint` while it is `recording`, in chunks of
    /// `TRIM_CHUNK_BUDGET`, and return how many. Each chunk checks the state again, so a
    /// checkpoint whose trim went on records nothing lost. Nothing has been deleted for them yet,
    /// and sealed sets never read them, so forgetting them changes no inventory or plan.
    pub fn forget_recording_tombstones(&self, checkpoint: &str) -> Result<usize> {
        self.runtime.checkpoint_preflight()?;
        let mut forgotten = 0;
        loop {
            let mut connection = self.connection.write_background();
            let held_since = std::time::Instant::now();
            let transaction = connection.transaction()?;
            let recording: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM checkpoints WHERE id=?1 AND state='recording')",
                [checkpoint],
                |row| row.get(0),
            )?;
            if !recording {
                return Ok(forgotten);
            }
            let started = std::time::Instant::now();
            let mut deleted = 0;
            // A trim records after every earlier checkpoint's tombstones, so a recording
            // checkpoint's tombstones are the newest rows of each table: walk down from the
            // newest by rowid, with no scan for the checkpoint's name.
            'tables: for table in ["checkpoint_envelopes", "checkpoint_claims"] {
                while started.elapsed() < TRIM_CHUNK_BUDGET {
                    let newest = transaction
                        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h527d45a1ba30 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ &format!(
                            "SELECT rowid, checkpoint FROM {table} ORDER BY rowid DESC LIMIT 1"
                        ))?
                        .query_row(/* sqlite-large-value: allow-for-now lv-h556d5f52a3a9 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
                        .optional()?;
                    match newest {
                        Some((rowid, name)) if name == checkpoint => {
                            transaction
                                .execute(&format!(/* sqlite-large-value: allow-for-now lv-hb9e8af7e99a4 owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM {table} WHERE rowid=?1"), [rowid])?;
                            deleted += 1;
                        }
                        _ => continue 'tables,
                    }
                }
                break;
            }
            transaction.commit()?;
            let held = held_since.elapsed();
            drop(connection);
            forgotten += deleted;
            if deleted == 0 {
                return Ok(forgotten);
            }
            self.pause_trim_chunk(held);
        }
    }

    pub fn set_checkpoint_state(
        &self,
        checkpoint: &str,
        cut_unix_ms: u128,
        state: &str,
    ) -> Result<()> {
        self.runtime.checkpoint_preflight()?;
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h4852c7521fdc owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(id, cut_unix_ms, state, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET state=excluded.state,
                 updated_at_unix_ms=excluded.updated_at_unix_ms",
            params![
                checkpoint,
                i64::try_from(cut_unix_ms)?,
                state,
                i64::try_from(now_ms())?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn checkpoint_diagnostic(&self, code: &str, reason: &str, checkpoint: &str) -> Result<()> {
        self.append_claim(&ClaimInput {
            subject: format!("daemon/{}", self.origin),
            kind: "daemon.diagnostic".into(),
            actor: None,
            fields: BTreeMap::from([
                ("severity".into(), json!("error")),
                ("code".into(), json!(code)),
                ("reason".into(), json!(reason)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("{code}:{}:{checkpoint}", self.origin)),
        })
        .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
        Ok(())
    }

    /// Record every tombstone and mark the checkpoint `trimming` in one transaction, then
    /// delete. The clamp's floor comes with the row, so nothing this node writes afterwards is
    /// dated before the cut.
    ///
    /// With `exact`, as in adoption, `envelopes` and `claims` are every tombstone through the
    /// checkpoint, and any other tombstone this node holds goes. That happens only on a node
    /// that applied the other side's certificate for a cut both sides of a partition certified.
    /// Its identities leave the inventory, and peers that kept those envelopes send them again.
    #[allow(clippy::too_many_arguments)]
    pub fn trim_checkpoint(
        &self,
        checkpoint: &str,
        cut_unix_ms: u128,
        drop_digest: &str,
        envelopes: &[EnvelopeTombstone],
        claims: &[ClaimTombstone],
        exact: bool,
        actions: &mut Vec<CheckpointAction>,
    ) -> Result<()> {
        self.trim_checkpoint_from(checkpoint, cut_unix_ms, drop_digest, &(envelopes, claims), exact, actions)
    }

    /// `trim_checkpoint` from this node's own staged plan, read a page at a time.
    pub fn trim_staged_checkpoint(
        &self,
        checkpoint: &str,
        staged: &super::checkpoint_staging::StagedSet,
        plan: &super::checkpoint_staging::StagedPlan,
        actions: &mut Vec<CheckpointAction>,
    ) -> Result<()> {
        self.trim_checkpoint_from(checkpoint, plan.cut_unix_ms, &plan.drop_digest, &(staged, plan), false, actions)
    }

    /* sqlite-large-value-flow: allow-for-now flow-h39d714b2bc13 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ fn trim_checkpoint_from(
        &self,
        checkpoint: &str,
        cut_unix_ms: u128,
        drop_digest: &str,
        tombstones: &dyn Tombstones,
        exact: bool,
        actions: &mut Vec<CheckpointAction>,
    ) -> Result<()> {
        let _completion = super::checkpoint_completion::Completion::work();
        self.runtime.checkpoint_preflight()?;
        let mark = |transaction: &Transaction<'_>, state: &str| -> Result<()> {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h1a537769038f owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(
                     id, cut_unix_ms, state, drop_digest, detail, updated_at_unix_ms)
                 VALUES (?1, ?2, ?6, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET state=excluded.state,
                     drop_digest=excluded.drop_digest, detail=excluded.detail,
                     updated_at_unix_ms=excluded.updated_at_unix_ms",
                params![
                    checkpoint,
                    i64::try_from(cut_unix_ms)?,
                    drop_digest,
                    json!({"envelopes": tombstones.counts().0, "claims": tombstones.counts().1}).to_string(),
                    i64::try_from(now_ms())?,
                    state
                ],
            )?;
            Ok(())
        };
        // Recording, in either branch, through the `trimming` mark; finish_trim's spans follow.
        let record = crate::profile::span("trim.record");
        if exact {
            // Adoption replaces every other tombstone, so it stays one transaction.
            let (envelopes, claims) = tombstones
                .slices()
                .ok_or_else(|| anyhow::anyhow!("an exact trim needs its manifest in memory"))?;
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            keep_only_tombstones_tx(&transaction, envelopes, claims)?;
            record_checkpoint_tombstones_tx(&transaction, checkpoint, envelopes, claims)?;
            mark(&transaction, "trimming")?;
            transaction.commit()?;
        } else {
            {
                let mut connection = self.connection.write();
                let transaction = connection.transaction()?;
                mark(&transaction, "recording")?;
                if self.trims_locally() {
                    start_local_walk_tx(&transaction, checkpoint)?;
                }
                transaction.commit()?;
            }
            // Recording again changes nothing, so a restart records what is missing. The
            // tombstones are read a page at a time, outside the writer.
            let (envelope_count, claim_count) = tombstones.counts();
            let (mut envelope, mut claim, mut chunks) = (0, 0, 0);
            let mut envelope_page: std::collections::VecDeque<EnvelopeTombstone> = Default::default();
            let mut claim_page: std::collections::VecDeque<ClaimTombstone> = Default::default();
            while envelope < envelope_count || claim < claim_count {
                if envelope < envelope_count && envelope_page.is_empty() {
                    envelope_page.extend(tombstones.envelope_page(envelope, TOMBSTONE_READ_PAGE)?);
                } else if envelope >= envelope_count && claim_page.is_empty() {
                    claim_page.extend(tombstones.claim_page(claim, TOMBSTONE_READ_PAGE)?);
                }
                anyhow::ensure!(
                    !envelope_page.is_empty() || !claim_page.is_empty(),
                    "the tombstones of {checkpoint} ended before their count"
                );
                let mut connection = self.connection.write_background();
                let held_since = std::time::Instant::now();
                let transaction = connection.transaction()?;
                let started = std::time::Instant::now();
                let (envelope_before, claim_before) = (envelope, claim);
                while started.elapsed() < TRIM_CHUNK_BUDGET
                    && (!envelope_page.is_empty() || (envelope >= envelope_count && !claim_page.is_empty()))
                {
                    if let Some(next) = envelope_page.pop_front() {
                        record_checkpoint_tombstones_tx(&transaction, checkpoint, &[next], &[])?;
                        envelope += 1;
                    } else if let Some(next) = claim_page.pop_front() {
                        record_checkpoint_tombstones_tx(&transaction, checkpoint, &[], &[next])?;
                        claim += 1;
                    }
                    self.pause_for_trim_row_cost();
                }

                let committing = std::time::Instant::now();
                transaction.commit()?;
                let held = held_since.elapsed();
                log_if_slow_trim_chunk("record", held, committing.elapsed(), envelope - envelope_before, claim - claim_before, 0);
                drop(connection);
                chunks += 1;
                self.stop_for_trim_fault(TrimFault::AfterTombstoneChunk(chunks))?;
                self.pause_trim_chunk(held);
            }
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            mark(&transaction, "trimming")?;
            transaction.commit()?;
        }
        drop(record);
        self.replica_rows_changed();
        self.stop_for_trim_fault(TrimFault::AfterTombstones)?;
        self.finish_trim(checkpoint, actions)
    }

    /// A member that trims locally walks only `checkpoint`'s own tombstones: from where its walk
    /// stopped, or else from the first of them, found once on a reader outside the writer (a cut
    /// an older build recorded). A cut with none starts past the end and is done at once.
    fn local_walk(&self, checkpoint: &str) -> Result<Option<LocalWalk>> {
        if !self.trims_locally() {
            return Ok(None);
        }
        let connection = self.readers.get();
        let saved: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key=?1", [local_walk_key(checkpoint)], |row| row.get(0))
            .optional()?;
        let (envelope, claim) = match saved.as_deref().and_then(|saved| saved.split_once(' ')) {
            Some((envelope, claim)) => (envelope.parse()?, claim.parse()?),
            None => {
                let start = |table: &str| -> Result<i64> {
                    Ok(connection.query_row(
                        /* sqlite-large-value: allow-for-now lv-h5fcb25afca0e owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ &format!(
                            "SELECT COALESCE((SELECT MIN(rowid) - 1 FROM {table} WHERE checkpoint=?1),
                                             (SELECT MAX(rowid) FROM {table}), 0)"
                        ),
                        [checkpoint],
                        |row| row.get(0),
                    )?)
                };
                (start("checkpoint_envelopes")?, start("checkpoint_claims")?)
            }
        };
        Ok(Some(LocalWalk { checkpoint: checkpoint.to_owned(), envelope, claim }))
    }

    /// Delete every row a tombstone stands for, in chunks, then mark the checkpoint trimmed.
    /// Running it again after a crash deletes only what is left. A chunk deletes one envelope at
    /// a time and stops starting new steps after `TRIM_CHUNK_BUDGET`. Chunks use background
    /// writer admission and rest outside the loan between commits. Each chunk checks, inside its own
    /// transaction, that the graph did not change; the proof showed it cannot.
    pub /* sqlite-large-value-flow: allow-for-now flow-h3f9bf57da525 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ fn finish_trim(&self, checkpoint: &str, actions: &mut Vec<CheckpointAction>) -> Result<()> {
        self.finish_trim_paced(checkpoint, actions, std::thread::sleep)
    }

    /* sqlite-large-value-flow: allow-for-now flow-h4f2952c50e14 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ fn finish_trim_paced(
        &self,
        checkpoint: &str,
        actions: &mut Vec<CheckpointAction>,
        mut pause: impl FnMut(std::time::Duration),
    ) -> Result<()> {
        let _completion = super::checkpoint_completion::Completion::work();
        self.runtime.checkpoint_preflight()?;
        let mut chunks = 0;
        let mut deleted_envelopes = 0;
        let mut deleted_claims = 0;
        let mut cursor = TrimCursor { local: self.local_walk(checkpoint)?, ..TrimCursor::default() };
        // A local walk deletes at most the day's claims left; a shared checkpoint's trim is not
        // capped.
        let day = self.trim_day.load(Ordering::Relaxed);
        let mut left = cursor.local.is_some().then(|| self.local_trim_left_today()).transpose()?;
        let delete = crate::profile::span("trim.delete");
        loop {
            if left == Some(0) {
                // The cut stays `trimming`, and the next day's first step goes on from here. What
                // its deletes deferred is rebuilt now, so a deferral outlasts no day's run.
                self.rebuild_deferred_paced()?;
                actions.push(CheckpointAction::TrimDeferred {
                    checkpoint: checkpoint.to_owned(),
                    envelopes: deleted_envelopes,
                    claims: deleted_claims,
                });
                return Ok(());
            }
            let mut connection = self.connection.write_background();
            let held_since = std::time::Instant::now();
            let transaction = connection.transaction()?;
            let started = std::time::Instant::now();
            #[cfg(test)]
            let started = {
                let mut fault = self.trim_fault.lock().unwrap_or_else(PoisonError::into_inner);
                if *fault == Some(TrimFault::ExpireChunkBudget) {
                    *fault = None;
                    started - TRIM_CHUNK_BUDGET
                } else {
                    started
                }
            };
            let most = self.trim_chunk_envelopes.load(Ordering::Acquire);
            // Deleting claims touches no graph table, so the generation stays. Only if it moved
            // are the digests themselves compared, before and after, in this transaction.
            let generation = projection_digest::generation(&transaction)?;
            transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h4ccfc8269d7a owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ "SAVEPOINT trim_chunk")?;
            let mut envelopes = Vec::new();
            let mut claims = Vec::new();
            let mut body_bytes = 0;
            let mut steps = 0;
            // Only an empty lookup proves exhaustion. Even if transaction setup spent the
            // budget, make one lookup so a zero-step chunk cannot falsely finish the trim.
            while steps < most
                && (steps == 0 || started.elapsed() < TRIM_CHUNK_BUDGET)
                && left.is_none_or(|left| (claims.len() as u64) < left)
            {
                let Some((envelope, envelope_claims)) = next_trim_rows(&transaction, &mut cursor)? else {
                    break;
                };
                body_bytes += claim_body_bytes(&transaction, &envelope_claims)?;
                delete_dropped_rows_tx(&transaction, envelope.as_slice(), &envelope_claims)?;
                self.pause_for_trim_row_cost();
                envelopes.extend(envelope);
                claims.extend(envelope_claims);
                steps += 1;
            }
            if steps == 0 {
                break;
            }
            self.alter_graph_for_trim_fault(&transaction)?;
            let changed = if projection_digest::generation(&transaction)? == generation {
                false
            } else {
                let after = graph_digest(&transaction)?;
                transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hcad476fc11b6 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ "ROLLBACK TO trim_chunk")?;
                let before = graph_digest(&transaction)?;
                delete_dropped_rows_tx(&transaction, &envelopes, &claims)?;
                before != after || graph_digest(&transaction)? != before
            };
            transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h4f653c779b09 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ "RELEASE trim_chunk")?;
            if changed {
                transaction.rollback()?;
                drop(connection);
                self.set_checkpoint_state(checkpoint, 0, "graph-changed")?;
                self.checkpoint_diagnostic(
                    "checkpoint-trim-graph-changed",
                    &format!(
                        "deleting what {checkpoint} drops would change the graph; nothing more \
                         is deleted and no checkpoint is sealed until a person looks"
                    ),
                    checkpoint,
                )?;
                actions.push(CheckpointAction::TrimGraphChanged {
                    checkpoint: checkpoint.to_owned(),
                });
                return Ok(());
            }
            if let Some(walk) = &cursor.local {
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-h90aea7898bf3 owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?
                    .execute(/* sqlite-large-value: allow-for-now lv-hde0878901e21 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![local_walk_key(&walk.checkpoint), format!("{} {}", walk.envelope, walk.claim)])?;
                let today = deleted_on(&transaction, day)? + claims.len() as u64;
                transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-h4d909c3fd602 owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?
                    .execute(/* sqlite-large-value: allow-for-now lv-h5ece9b814a1a owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![LOCAL_TRIM_DAY, format!("{day} {today}")])?;
            }
            let committing = std::time::Instant::now();
            transaction.commit()?;
            let held = held_since.elapsed();
            let phase = if envelopes.is_empty() { "claims" } else { "envelopes" };
            log_if_slow_trim_chunk(phase, held, committing.elapsed(), envelopes.len(), claims.len(), body_bytes);
            drop(connection);
            // A cursor-only chunk inspected already-absent rows. All chunks that actually
            // delete keep the original invalidation; this does not coalesce mutations.
            if !envelopes.is_empty() || !claims.is_empty() {
                self.replica_rows_changed();
            }
            deleted_envelopes += envelopes.len();
            deleted_claims += claims.len();
            left = left.map(|left| left.saturating_sub(claims.len() as u64));
            chunks += 1;
            self.stop_for_trim_fault(TrimFault::AfterChunk(chunks))?;
            pause(self.trim_pause_duration(held));
        }
        drop(delete);
        self.stop_for_trim_fault(TrimFault::BeforeFinish)?;
        {
            let _span = crate::profile::span("trim.final_mark");
            let mut connection = self.connection.write();
            let transaction = connection.transaction()?;
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-ha26d7ebb680b owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoints SET state='trimmed', updated_at_unix_ms=?2 WHERE id=?1",
                params![checkpoint, i64::try_from(now_ms())?],
            )?;
            transaction.execute(/* sqlite-large-value: allow-for-now lv-h51f966c0da12 owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [local_walk_key(checkpoint)])?;
            // Every snapshot and page cursor taken before the trim expires, since the claims
            // they read may be gone.
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h247fda7620dc owner=smallclaims::store::checkpoint_trim TODO=prepare-body-and-predecessor-values */ "UPDATE sqlite_sequence SET seq=seq+1 WHERE name='claims'",
                [],
            )?;
            transaction.commit()?;
        }
        self.replica_rows_changed();
        self.rebuild_deferred_paced()?;
        actions.push(CheckpointAction::Trimmed {
            checkpoint: checkpoint.to_owned(),
            envelopes: deleted_envelopes,
            claims: deleted_claims,
        });
        Ok(())
    }
}

impl Store {
    /// Rebuild what this member's trims deferred (the runtime's derived state of the subjects
    /// they touched), a budget's worth per transaction with a trim's pacing, until none is left.
    /// It runs when a trim stops and at each checkpoint step, so a crash cannot strand it.
    pub(crate) fn rebuild_deferred_paced(&self) -> Result<()> {
        // Most steps have nothing deferred: one read, and no writer.
        if !self.runtime.deferred_pending(&self.readers.get())? {
            return Ok(());
        }
        let _span = crate::profile::span("trim.rebuild_deferred");
        loop {
            let mut connection = self.connection.write_background();
            let held_since = std::time::Instant::now();
            let transaction = connection.transaction()?;
            let more = self.runtime.rebuild_deferred(&transaction, TRIM_CHUNK_BUDGET)?;
            let committing = std::time::Instant::now();
            transaction.commit()?;
            let held = held_since.elapsed();
            log_if_slow_trim_chunk("rebuild", held, committing.elapsed(), 0, 0, 0);
            drop(connection);
            if !more {
                return Ok(());
            }
            self.pause_trim_chunk(held);
        }
    }

    /// The claims a local trim may still delete today (`trim_locally_at_most`).
    pub fn local_trim_left_today(&self) -> Result<u64> {
        let deleted = deleted_on(&self.readers.get(), self.trim_day.load(Ordering::Relaxed))?;
        Ok(self.local_trim_daily_claims().saturating_sub(deleted))
    }
}

/// The claims a local trim deleted on `day`, as `LOCAL_TRIM_DAY` records them.
fn deleted_on(connection: &Connection, day: u64) -> Result<u64> {
    let saved: Option<String> = connection
        .prepare_cached("SELECT value FROM meta WHERE key=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h70b39bb4733d owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [LOCAL_TRIM_DAY], |row| row.get(0))
        .optional()?;
    Ok(saved
        .as_deref()
        .and_then(|saved| saved.split_once(' '))
        .filter(|(saved_day, _)| saved_day.parse() == Ok(day))
        .and_then(|(_, claims)| claims.parse().ok())
        .unwrap_or(0))
}

/// How far the sweep of rows earlier trims left behind has gone: `after:CLAIM_ID`, or `done`.
pub const TRIMMED_CLAIM_ROWS_SWEPT: &str = "checkpoint_trimmed_claim_rows_swept";

/// How long one sweep starts new steps before it commits. A step or COMMIT may take longer.
pub const SWEEP_CHUNK_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);

impl Store {
    /// Delete the signature and verdict rows of every claim a checkpoint dropped that this store
    /// still holds. Builds before this one deleted a dropped claim but not these rows. It walks
    /// the claim tombstones once, in chunks that each commit after `SWEEP_CHUNK_BUDGET`, then
    /// records that it is done, so a later pass reads one row. Returns the rows deleted.
    pub fn sweep_trimmed_claim_rows(&self) -> Result<usize> {
        let progress = {
            let connection = self.readers.get();
            super::fleet_meta(&connection, TRIMMED_CLAIM_ROWS_SWEPT)?
        };
        if progress.as_deref() == Some("done") {
            return Ok(0);
        }
        let mut after = progress
            .as_deref()
            .and_then(|progress| progress.strip_prefix("after:"))
            .unwrap_or_default()
            .to_owned();
        let mut rows = 0;
        loop {
            let mut connection = self.connection.write_background();
            let held_since = std::time::Instant::now();
            let transaction = connection.transaction()?;
            let started = std::time::Instant::now();
            let mut finished = false;
            // One claim at a time: on a store larger than its page cache, each claim's deletes
            // read pages from disk, so the budget is checked after every claim.
            while started.elapsed() < SWEEP_CHUNK_BUDGET {
                let Some(id) = transaction
                    .prepare_cached("SELECT id FROM checkpoint_claims WHERE id > ?1 ORDER BY id LIMIT 1")?
                    .query_row(/* sqlite-large-value: allow-for-now lv-hed1f38ebe50a owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [&after], |row| row.get::<_, String>(0))
                    .optional()?
                else {
                    finished = true;
                    break;
                };
                // A claim held in two envelopes can be both kept and tombstoned.
                let held: bool = transaction
                    .prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?
                    .query_row(/* sqlite-large-value: allow-for-now lv-hef37db70f7d0 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [&id], |row| row.get(0))?;
                if !held {
                    rows += super::principals::forget_claim_tx(&transaction, &id)?;
                }
                after = id;
            }
            let progress = if finished { "done".to_owned() } else { format!("after:{after}") };
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h74f80d64ae6a owner=smallclaims::store::checkpoint_trim TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![TRIMMED_CLAIM_ROWS_SWEPT, progress],
            )?;
            transaction.commit()?;
            let held = held_since.elapsed();
            drop(connection);
            if finished {
                return Ok(rows);
            }
            self.pause_trim_chunk(held);
        }
    }
}

/// Where a trim goes on from: the last envelope tombstone it read, then the last claim tombstone
/// whose envelope was already gone. Each is read once, however many chunks the trim takes.
#[derive(Default)]
struct TrimCursor {
    envelope: (String, i64, String),
    envelopes_done: bool,
    claim: String,
    local: Option<LocalWalk>,
}

/// Where a member that trims locally has got in its own cut's tombstones. They were recorded
/// after every earlier checkpoint's, so the walk goes by rowid from the first of them and never
/// visits the older ones. It is kept in `meta` with each chunk, so a restart goes on from it.
#[derive(Debug, Default)]
struct LocalWalk {
    checkpoint: String,
    envelope: i64,
    claim: i64,
}

/// The `meta` key of a local cut's walk.
fn local_walk_key(checkpoint: &str) -> String {
    format!("local_trim_walk/{checkpoint}")
}

/// Record where a local cut's tombstones begin, before the first of them is recorded.
fn start_local_walk_tx(transaction: &Transaction<'_>, checkpoint: &str) -> Result<()> {
    transaction.execute(
        /* sqlite-large-value: allow-for-now lv-hd6e8a46ee987 owner=smallclaims::store::checkpoint_trim TODO=stage-generation-and-keyed-reclamation */ "INSERT OR IGNORE INTO meta(key, value) VALUES (?1,
             (SELECT COALESCE(MAX(rowid), 0) FROM checkpoint_envelopes) || ' ' ||
             (SELECT COALESCE(MAX(rowid), 0) FROM checkpoint_claims))",
        [local_walk_key(checkpoint)],
    )?;
    Ok(())
}

/// Inspect one indexed tombstone, advancing even when its row is already gone. An empty pair
/// means progress, not exhaustion; only None proves that both ordered scans have ended.
/// Keeping the existence probe outside the ordered seek prevents LIMIT 1 from scanning an
/// arbitrarily long run of already-trimmed tombstones while holding the writer.
fn next_trim_rows(
    transaction: &Transaction<'_>,
    cursor: &mut TrimCursor,
) -> Result<Option<(Option<EnvelopeTombstone>, Vec<ClaimTombstone>)>> {
    if !cursor.envelopes_done {
        let envelope = match &mut cursor.local {
            Some(walk) => next_local_envelope(transaction, walk)?,
            None => {
                let envelope = transaction
                    .prepare_cached(
                        "SELECT tombstone.writer, tombstone.sequence, tombstone.envelope_hash,
                                tombstone.accepted_at_unix_ms
                         FROM checkpoint_envelopes AS tombstone
                         WHERE (tombstone.writer, tombstone.sequence, tombstone.envelope_hash)
                               > (?1, ?2, ?3)
                         ORDER BY tombstone.writer, tombstone.sequence, tombstone.envelope_hash
                         LIMIT 1",
                    )?
                    .query_row(
                        /* sqlite-large-value: allow-for-now lv-h75e67f417bdf owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![cursor.envelope.0, cursor.envelope.1, cursor.envelope.2],
                        |row| {
                            Ok(EnvelopeTombstone {
                                writer: row.get(0)?,
                                sequence: row.get(1)?,
                                envelope_hash: row.get(2)?,
                                accepted_at_unix_ms: u128::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                            })
                        },
                    )
                    .optional()?;
                if let Some(envelope) = &envelope {
                    cursor.envelope = (
                        envelope.writer.clone(),
                        i64::try_from(envelope.sequence)?,
                        envelope.envelope_hash.clone(),
                    );
                }
                envelope
            }
        };
        if let Some(envelope) = envelope {
            let held: bool = transaction.prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM replica_envelopes
                 WHERE writer=?1 AND sequence=?2 AND envelope_hash=?3)"
            )?.query_row(/* sqlite-large-value: allow-for-now lv-h3f0f377d0d3b owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![envelope.writer, envelope.sequence, envelope.envelope_hash], |row| row.get(0))?;
            if !held {
                return Ok(Some((None, Vec::new())));
            }
            let claims = transaction
                .prepare_cached(
                    "SELECT tombstone.id, tombstone.operation_id FROM checkpoint_claims AS tombstone
                     WHERE tombstone.writer=?1 AND tombstone.sequence=?2
                       AND tombstone.envelope_hash=?3
                       AND EXISTS (SELECT 1 FROM claims WHERE claims.id=tombstone.id)",
                )?
                .query_map(
                    params![envelope.writer, envelope.sequence, envelope.envelope_hash],
                    |row| Ok(deletable_claim(row.get(0)?, row.get(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(Some((Some(envelope), claims)));
        }
        cursor.envelopes_done = true;
    }
    let claim = match &mut cursor.local {
        Some(walk) => next_local_claim(transaction, walk)?,
        None => {
            let claim = transaction
                .prepare_cached(
                    "SELECT tombstone.id, tombstone.operation_id FROM checkpoint_claims AS tombstone
                     WHERE tombstone.id > ?1
                     ORDER BY tombstone.id LIMIT 1",
                )?
                .query_row(/* sqlite-large-value: allow-for-now lv-h0c032884ac26 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [&cursor.claim], |row| {
                    Ok(deletable_claim(row.get(0)?, row.get(1)?))
                })
                .optional()?;
            if let Some(claim) = &claim {
                cursor.claim = claim.id.clone();
            }
            claim
        }
    };
    Ok(match claim {
        Some(claim) => {
            let held: bool = transaction.prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?
                .query_row(/* sqlite-large-value: allow-for-now lv-h12c2aa291add owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ [&claim.id], |row| row.get(0))?;
            Some((None, if held { vec![claim] } else { Vec::new() }))
        }
        None => None,
    })
}

/// A local cut's next envelope tombstone, by rowid. Past the last, the walk stays at the end.
fn next_local_envelope(transaction: &Transaction<'_>, walk: &mut LocalWalk) -> Result<Option<EnvelopeTombstone>> {
    let next = transaction
        .prepare_cached(
            "SELECT rowid, writer, sequence, envelope_hash, accepted_at_unix_ms FROM checkpoint_envelopes
             WHERE rowid > ?1 AND checkpoint=?2 ORDER BY rowid LIMIT 1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hbdfea6a33a89 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![walk.envelope, walk.checkpoint], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                EnvelopeTombstone {
                    writer: row.get(1)?,
                    sequence: row.get(2)?,
                    envelope_hash: row.get(3)?,
                    accepted_at_unix_ms: u128::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                },
            ))
        })
        .optional()?;
    walk.envelope = next.as_ref().map_or(i64::MAX, |(rowid, _)| *rowid);
    Ok(next.map(|(_, envelope)| envelope))
}

/// A local cut's next claim tombstone, by rowid. Past the last, the walk stays at the end.
fn next_local_claim(transaction: &Transaction<'_>, walk: &mut LocalWalk) -> Result<Option<ClaimTombstone>> {
    let next = transaction
        .prepare_cached(
            "SELECT rowid, id, operation_id FROM checkpoint_claims
             WHERE rowid > ?1 AND checkpoint=?2 ORDER BY rowid LIMIT 1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h42e6c69fa763 owner=smallclaims::store::checkpoint_trim TODO=trace-statement-and-value-representation */ params![walk.claim, walk.checkpoint], |row| {
            Ok((row.get::<_, i64>(0)?, deletable_claim(row.get(1)?, row.get(2)?)))
        })
        .optional()?;
    walk.claim = next.as_ref().map_or(i64::MAX, |(rowid, _)| *rowid);
    Ok(next.map(|(_, claim)| claim))
}

/// What `delete_dropped_rows_tx` needs of a claim it deletes.
fn deletable_claim(id: String, operation_id: Option<String>) -> ClaimTombstone {
    ClaimTombstone {
        id,
        operation_id,
        writer: String::new(),
        sequence: 0,
        envelope_hash: String::new(),
        subject: String::new(),
        kind: String::new(),
        actor: None,
        predecessors: Vec::new(),
        request_digest: None,
        accepted_at_unix_ms: 0,
    }
}

#[cfg(test)]
mod capture_lifetime_tests;
#[cfg(test)]
mod phase_span_tests;
#[cfg(test)]
mod pacing_tests;
#[cfg(test)]
mod local_trim_tests;
