//! What the retention passes have done and why they stop (`docs/st3/retention.md`, "Retention
//! passes"). The view reads only what the passes already keep: a few `meta` keys, the newest row
//! of the `checkpoints` ledger (one row per daily cut), and the first or last rowid of a table.
//! Nothing here counts the rows of a large table. Counts since start are this process's own,
//! added in memory after each pass's page commits.
use super::*;
use super::idempotency::Cleanup;
use super::principals::JudgingPrune;
use super::record_bytes::{RecordBytesRelease, VALID_RECORD_DELETION};
use super::envelope_release::ENVELOPE_RELEASE;

/// Why the last checkpoint pass trimmed no further.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimStop {
    /// Replication is catching up with a peer, so what lies before the cut still changes.
    CatchingUp,
    /// `[trim] local` is off.
    Disabled,
    /// A trim stopped because the graph would change and waits for a person.
    HaltedCut,
    /// An earlier cut is still `trimming`; the next cut begins once it finishes.
    UnfinishedCut,
    /// The day's `[trim] daily_claims` allowance is spent, or has not accrued yet.
    DailyCap,
    /// The trim stopped to restart the WAL; the next pass goes on from there.
    WalRestart,
    /// This pass found that deleting would change the graph and halted the cut.
    GraphChanged,
    /// Every due cut is trimmed.
    NothingDue,
}

/// How a checkpoint pass ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimOutcome {
    /// Every cut the pass began, it finished.
    Complete,
    /// The pass left a cut it had begun: deferred to a later pass, halted because the graph would
    /// change, or set aside.
    Partial,
    /// The pass returned an error.
    Failed,
}

/// One checkpoint pass: what it trimmed and why it stopped. The deleted counts are what its
/// committed chunks deleted, however the pass ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrimPass {
    pub at_unix_ms: u128,
    pub stop: TrimStop,
    pub outcome: TrimOutcome,
    /// Cuts this pass finished.
    pub trimmed_cuts: u64,
    /// Cuts this pass left `trimming`, to go on in a later pass.
    pub deferred_cuts: u64,
    /// Cuts this pass set aside: resuming them would delete a signed claim still inside the
    /// freshness window's forget bound, so the next day's cut goes on instead.
    pub set_aside_cuts: u64,
    pub deleted_envelopes: u64,
    pub deleted_claims: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimSinceStart {
    pub passes: u64,
    pub trimmed_cuts: u64,
    pub deferred_cuts: u64,
    pub set_aside_cuts: u64,
    pub deleted_envelopes: u64,
    pub deleted_claims: u64,
}

/// The newest cut in the `checkpoints` ledger. Its eligible counts are the tombstones its plan
/// recorded; a cut whose plan was refused before recording has none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimCut {
    pub id: String,
    pub state: String,
    pub eligible_envelopes: Option<u64>,
    pub eligible_claims: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimRetention {
    /// `[trim] local`.
    pub local: bool,
    /// `[trim] daily_claims`.
    pub daily_claims: u64,
    /// Claims deleted on the current UTC day (`local_trim_day`).
    pub deleted_today: u64,
    pub newest_cut: Option<TrimCut>,
    /// The oldest cut still `trimming`, which the next cut waits for.
    pub unfinished_cut: Option<String>,
    pub last_pass: Option<TrimPass>,
    pub since_start: TrimSinceStart,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordBytesSinceStart {
    pub pages: u64,
    pub emptied_records: u64,
    pub emptied_bytes: u64,
}

/// The pass that empties valid records' `raw` (`record_bytes`): its cursor against the newest
/// record. It is done when the cursor reached the newest record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordBytesRetention {
    pub cursor_rowid: i64,
    pub newest_rowid: i64,
    pub done: bool,
    pub since_start: RecordBytesSinceStart,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgingSinceStart {
    pub pages: u64,
    pub examined: u64,
    pub deleted: u64,
}

/// The prune of settled judging rows (`principals::judging`). `cursor` is absent before its first
/// page, `done` once it walked every row, and otherwise the last claim it passed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JudgingRetention {
    pub cursor: Option<String>,
    pub finished: bool,
    pub since_start: JudgingSinceStart,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReceiptCleanupSinceStart {
    pub chunks: u64,
    pub examined: u64,
    pub deleted: u64,
    pub extended: u64,
}

/// Receipts stored before expiry was recorded: rows at or below `boundary_rowid`
/// (`idempotency_legacy_rowid`). The cleanup walks in rowid order and moves a receipt it keeps past
/// the boundary, so some remain while the oldest row is at or below it. The counts since start are
/// the whole cleanup's, legacy and expired receipts alike.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyReceiptRetention {
    pub boundary_rowid: Option<i64>,
    pub oldest_rowid: Option<i64>,
    pub pending: bool,
    pub since_start: ReceiptCleanupSinceStart,
}

/// A pass the daemon does not run in this build, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PassSwitch {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off_because: Option<String>,
}

impl PassSwitch {
    fn new(enabled: bool, off_because: &str) -> Self {
        Self { enabled, off_because: (!enabled).then(|| off_because.to_owned()) }
    }
}

/// What every retention pass of this member has done, for `fractal replication status`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionStatus {
    pub trim: TrimRetention,
    pub valid_record_bytes: RecordBytesRetention,
    pub settled_judging: JudgingRetention,
    pub legacy_receipts: LegacyReceiptRetention,
    pub envelope_release: PassSwitch,
    pub valid_record_deletion: PassSwitch,
}

/// The process's counts of the passes. Every update is an in-memory addition after a commit.
#[derive(Default)]
pub(crate) struct Tally {
    counts: Mutex<Counts>,
    /// Rows the trim's committed chunks deleted, in the running pass and since start, and cuts
    /// the running pass set aside. They are added while the writer is still on loan, so they
    /// take no lock.
    pass_envelopes: AtomicU64,
    pass_claims: AtomicU64,
    pass_set_aside: AtomicU64,
    deleted_envelopes: AtomicU64,
    deleted_claims: AtomicU64,
}

#[derive(Default)]
struct Counts {
    /// Why the running checkpoint pass stopped early, as the step noted it.
    noted: Option<TrimStop>,
    last_pass: Option<TrimPass>,
    /// Passes and cuts; the deleted counts since start are the atomics.
    trim: TrimSinceStart,
    valid_record_bytes: RecordBytesSinceStart,
    settled_judging: JudgingSinceStart,
    receipts: ReceiptCleanupSinceStart,
}

impl Tally {
    fn counts(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn begin_trim_pass(&self) {
        self.counts().noted = None;
        self.pass_envelopes.store(0, Ordering::Relaxed);
        self.pass_claims.store(0, Ordering::Relaxed);
        self.pass_set_aside.store(0, Ordering::Relaxed);
    }

    pub(crate) fn note_trim_stop(&self, stop: TrimStop) {
        self.counts().noted = Some(stop);
    }

    /// A trim chunk committed these deletions.
    pub(crate) fn trim_chunk(&self, envelopes: usize, claims: usize) {
        for (pass, total, rows) in [
            (&self.pass_envelopes, &self.deleted_envelopes, envelopes as u64),
            (&self.pass_claims, &self.deleted_claims, claims as u64),
        ] {
            pass.fetch_add(rows, Ordering::Relaxed);
            total.fetch_add(rows, Ordering::Relaxed);
        }
    }

    /// A cut was set aside.
    pub(crate) fn set_aside_cut(&self) {
        self.pass_set_aside.fetch_add(1, Ordering::Relaxed);
    }

    /// A checkpoint pass ended with the `actions` it recorded, which stop at the failure if
    /// it `failed`. A cut that would change the graph is the stop; otherwise the reason the
    /// step noted, or nothing left due. The deleted counts are the committed chunks', so a
    /// pass that halts or fails partway still reports what it deleted. A pass that halted on
    /// the graph, deferred or set aside a cut is partial.
    pub(crate) fn finish_trim_pass(&self, actions: &[CheckpointAction], failed: bool, at_unix_ms: u128) {
        let mut pass = TrimPass {
            at_unix_ms,
            stop: TrimStop::NothingDue,
            outcome: TrimOutcome::Complete,
            trimmed_cuts: 0,
            deferred_cuts: 0,
            set_aside_cuts: self.pass_set_aside.swap(0, Ordering::Relaxed),
            deleted_envelopes: self.pass_envelopes.swap(0, Ordering::Relaxed),
            deleted_claims: self.pass_claims.swap(0, Ordering::Relaxed),
        };
        let mut graph_changed = false;
        for action in actions {
            match action {
                CheckpointAction::Trimmed { .. } => pass.trimmed_cuts += 1,
                CheckpointAction::TrimDeferred { .. } => pass.deferred_cuts += 1,
                CheckpointAction::TrimGraphChanged { .. } => graph_changed = true,
                CheckpointAction::SweptTrimmedClaimRows { .. } => {}
            }
        }
        pass.outcome = if failed {
            TrimOutcome::Failed
        } else if graph_changed || pass.deferred_cuts > 0 || pass.set_aside_cuts > 0 {
            TrimOutcome::Partial
        } else {
            TrimOutcome::Complete
        };
        let mut counts = self.counts();
        let noted = counts.noted.take();
        pass.stop = if graph_changed { TrimStop::GraphChanged } else { noted.unwrap_or(TrimStop::NothingDue) };
        counts.trim.passes += 1;
        counts.trim.trimmed_cuts += pass.trimmed_cuts;
        counts.trim.deferred_cuts += pass.deferred_cuts;
        counts.trim.set_aside_cuts += pass.set_aside_cuts;
        counts.last_pass = Some(pass);
    }

    pub(crate) fn valid_record_bytes(&self, page: &RecordBytesRelease) {
        if page.scanned == 0 {
            return;
        }
        let mut counts = self.counts();
        counts.valid_record_bytes.pages += 1;
        counts.valid_record_bytes.emptied_records += page.emptied as u64;
        counts.valid_record_bytes.emptied_bytes += page.bytes;
    }

    pub(crate) fn settled_judging(&self, page: &JudgingPrune) {
        if page.examined == 0 {
            return;
        }
        let mut counts = self.counts();
        counts.settled_judging.pages += 1;
        counts.settled_judging.examined += page.examined as u64;
        counts.settled_judging.deleted += page.deleted as u64;
    }

    pub(crate) fn receipt_cleanup(&self, chunk: &Cleanup) {
        if chunk.examined == 0 {
            return;
        }
        let mut counts = self.counts();
        counts.receipts.chunks += 1;
        counts.receipts.examined += chunk.examined as u64;
        counts.receipts.deleted += chunk.deleted as u64;
        counts.receipts.extended += chunk.extended as u64;
    }
}

impl Store {
    /// Why a trim this pass left `trimming` stopped: the day's paced allowance, recomputed from
    /// the same day, clock and deletion counter the trim used, or else the WAL bound.
    pub(crate) fn trim_deferral_stop(&self) -> Result<TrimStop> {
        let deleted = super::checkpoint_trim::deleted_on(&self.readers.get(), self.trim_day.load(Ordering::Relaxed))?;
        let left = super::checkpoint_trim::trim_allowance(
            self.local_trim_daily_claims(),
            self.trim_now_ms.load(Ordering::Relaxed),
            deleted,
        );
        Ok(if left == 0 { TrimStop::DailyCap } else { TrimStop::WalRestart })
    }

    /// What each retention pass has done. Point reads only: `meta` keys, the newest and the
    /// oldest unfinished row of `checkpoints` (one row per daily cut), and rowid bounds.
    pub fn retention_status(&self) -> Result<RetentionStatus> {
        let connection = self.readers.get();
        let newest_cut = connection
            .query_row(
                "SELECT id, state, detail FROM checkpoints ORDER BY cut_unix_ms DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
            )
            .optional()?
            .map(|(id, state, detail)| {
                let detail = detail.and_then(|detail| serde_json::from_str::<Value>(&detail).ok());
                let eligible = |key: &str| detail.as_ref().and_then(|detail| detail[key].as_u64());
                TrimCut { id, state, eligible_envelopes: eligible("envelopes"), eligible_claims: eligible("claims") }
            });
        let unfinished_cut = connection
            .query_row(
                "SELECT id FROM checkpoints WHERE state='trimming' ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let today = u64::try_from(now_ms() / super::checkpoint_agreement::DAY_MS).unwrap_or(u64::MAX);
        let deleted_today = super::checkpoint_trim::deleted_on(&connection, today)?;
        let (cursor_rowid, newest_rowid) = super::record_bytes::progress(&connection)?;
        let judging_cursor = super::principals::settled_judging_cursor(&connection)?;
        let boundary_rowid = fleet_meta(&connection, "idempotency_legacy_rowid")?
            .and_then(|boundary| boundary.parse::<i64>().ok());
        let oldest_rowid: Option<i64> =
            connection.query_row("SELECT MIN(rowid) FROM idempotency", [], |row| row.get(0))?;
        drop(connection);
        let counts = self.retention.counts();
        Ok(RetentionStatus {
            trim: TrimRetention {
                local: self.trims_locally(),
                daily_claims: self.local_trim_daily_claims(),
                deleted_today,
                newest_cut,
                unfinished_cut,
                last_pass: counts.last_pass.clone(),
                since_start: TrimSinceStart {
                    deleted_envelopes: self.retention.deleted_envelopes.load(Ordering::Relaxed),
                    deleted_claims: self.retention.deleted_claims.load(Ordering::Relaxed),
                    ..counts.trim.clone()
                },
            },
            valid_record_bytes: RecordBytesRetention {
                cursor_rowid,
                newest_rowid,
                done: cursor_rowid >= newest_rowid,
                since_start: counts.valid_record_bytes.clone(),
            },
            settled_judging: JudgingRetention {
                finished: judging_cursor.as_deref() == Some("done"),
                cursor: judging_cursor,
                since_start: counts.settled_judging.clone(),
            },
            legacy_receipts: LegacyReceiptRetention {
                pending: oldest_rowid.zip(boundary_rowid).is_some_and(|(oldest, boundary)| oldest <= boundary),
                boundary_rowid,
                oldest_rowid,
                since_start: counts.receipts.clone(),
            },
            envelope_release: PassSwitch::new(
                ENVELOPE_RELEASE,
                "off until the claim-set transport can serve a released envelope's claims to a peer that lacks them",
            ),
            valid_record_deletion: PassSwitch::new(
                VALID_RECORD_DELETION,
                "off until the signature-reference writer turns on by its own flag (#6765)",
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{checkpoint_agreement::CheckpointContext, runtime::Plain};

    fn context(scratch: &Path, now_unix_ms: u128) -> CheckpointContext {
        CheckpointContext { now_unix_ms, scratch: scratch.to_path_buf() }
    }

    /// A plain store with the projection tables a local cut's capture reads.
    fn local_store() -> Store {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        store
            .connection
            .write()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS desired(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS mission_definitions(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS mission_revisions(claim_id TEXT);
                 CREATE TABLE IF NOT EXISTS documents(binding_claim_id TEXT);",
            )
            .unwrap();
        store
    }

    fn last_stop(store: &Store) -> TrimStop {
        store.retention_status().unwrap().trim.last_pass.unwrap().stop
    }

    #[test]
    fn a_fresh_store_reports_every_pass_without_a_pass_run() {
        let store = local_store();
        let status = store.retention_status().unwrap();
        assert_eq!(status.trim.last_pass, None);
        assert_eq!(status.trim.newest_cut, None);
        assert_eq!(status.trim.deleted_today, 0);
        assert_eq!(status.trim.daily_claims, store.local_trim_daily_claims());
        assert!(status.valid_record_bytes.done);
        assert_eq!(status.settled_judging.cursor, None);
        assert!(!status.settled_judging.finished);
        assert!(!status.legacy_receipts.pending);
        // Both passes are off in this build, and say why.
        assert!(!status.envelope_release.enabled);
        assert!(status.envelope_release.off_because.as_deref().unwrap().contains("claim-set transport"));
        assert!(!status.valid_record_deletion.enabled);
        assert!(status.valid_record_deletion.off_because.is_some());
    }

    #[test]
    fn a_checkpoint_pass_names_its_stop_and_counts_what_it_trimmed() {
        let scratch = tempfile::tempdir().unwrap();
        let now = 30 * super::super::checkpoint_agreement::DAY_MS;
        let off = local_store();
        off.checkpoint_step(&context(scratch.path(), now)).unwrap();
        assert_eq!(last_stop(&off), TrimStop::Disabled);

        let capped = local_store();
        capped.trim_locally(true).unwrap();
        capped.trim_locally_at_most(0);
        capped.checkpoint_step(&context(scratch.path(), now)).unwrap();
        assert_eq!(last_stop(&capped), TrimStop::DailyCap);

        let local = local_store();
        local.trim_locally(true).unwrap();
        local.checkpoint_step(&context(scratch.path(), now)).unwrap();
        let status = local.retention_status().unwrap().trim;
        let pass = status.last_pass.unwrap();
        assert_eq!((pass.stop, pass.trimmed_cuts, pass.deferred_cuts), (TrimStop::NothingDue, 1, 0));
        assert_eq!(pass.outcome, TrimOutcome::Complete);
        let cut = status.newest_cut.unwrap();
        assert_eq!(cut.id, checkpoint_name(newest_due_cut(now)));
        assert_eq!(cut.state, "trimmed");
        assert_eq!((cut.eligible_envelopes, cut.eligible_claims), (Some(0), Some(0)));
        assert_eq!(status.unfinished_cut, None);

        // A stopped trim halts the next pass until a person resumes it.
        local.set_checkpoint_state("stopped-for-test", 0, "graph-changed").unwrap();
        local.checkpoint_step(&context(scratch.path(), now + super::super::checkpoint_agreement::DAY_MS)).unwrap();
        let status = local.retention_status().unwrap().trim;
        assert_eq!(status.last_pass.unwrap().stop, TrimStop::HaltedCut);
        assert_eq!((status.since_start.passes, status.since_start.trimmed_cuts), (2, 1));
    }

    fn notes(store: &Store) -> i64 {
        store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claims WHERE kind='example.note'", [], |row| row.get(0))
            .unwrap()
    }

    /// A cut whose tombstones committed, as a crash leaves it, holding `count` notes: the next
    /// pass finishes it.
    fn interrupted_cut(store: &Store, checkpoint: &str, cut_unix_ms: u128, count: u64) {
        store.set_checkpoint_state(checkpoint, cut_unix_ms, "trimming").unwrap();
        let mut tombstones = Vec::new();
        for n in 0..count {
            let claim = store
                .append_claim(&ClaimInput {
                    subject: format!("note/{checkpoint}/{n}"),
                    kind: "example.note".into(),
                    actor: None,
                    fields: BTreeMap::from([("text".into(), json!(n))]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
            tombstones.push(ClaimTombstone {
                id: claim.id,
                operation_id: claim.operation_id,
                writer: "writer".into(),
                sequence: n,
                envelope_hash: format!("envelope-{checkpoint}-{n}"),
                subject: claim.subject,
                kind: claim.kind,
                actor: claim.actor,
                predecessors: claim.predecessors,
                request_digest: claim.request_digest,
                accepted_at_unix_ms: claim.accepted_at_unix_ms,
            });
        }
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        super::super::checkpoint::record_checkpoint_tombstones_tx(&transaction, checkpoint, &[], &tombstones).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn a_failed_pass_keeps_the_cuts_it_finished_and_what_its_chunks_deleted() {
        let scratch = tempfile::tempdir().unwrap();
        let store = local_store();
        store.trim_locally(true).unwrap();
        // Interrupted cuts finish in id order: the first in one chunk, then the second fails
        // after its second chunk committed.
        interrupted_cut(&store, "checkpoint/1970-01-02", 1, 1);
        interrupted_cut(&store, "checkpoint/1970-01-03", 2, 3);
        store.set_trim_chunk_envelopes(1);
        store.set_trim_fault(Some(TrimFault::AfterChunk(2)));
        let now = 30 * super::super::checkpoint_agreement::DAY_MS;
        assert!(store.checkpoint_step(&context(scratch.path(), now)).is_err());
        let trim = store.retention_status().unwrap().trim;
        let pass = trim.last_pass.unwrap();
        assert_eq!(pass.outcome, TrimOutcome::Failed);
        assert_eq!((pass.trimmed_cuts, pass.deferred_cuts), (1, 0), "the cut finished before the failure counts");
        assert_eq!((pass.deleted_envelopes, pass.deleted_claims), (0, 3));
        assert_eq!(notes(&store), 1, "the pass reports what it deleted");
        assert_eq!(
            (trim.since_start.passes, trim.since_start.trimmed_cuts, trim.since_start.deleted_claims),
            (1, 1, 3)
        );

        // The next pass finishes the second cut and counts only its own chunks.
        store.checkpoint_step(&context(scratch.path(), now)).unwrap();
        let trim = store.retention_status().unwrap().trim;
        let pass = trim.last_pass.unwrap();
        assert_eq!((pass.outcome, pass.deleted_claims), (TrimOutcome::Complete, 1));
        assert!(pass.trimmed_cuts >= 1, "{pass:?}");
        assert_eq!(notes(&store), 0);
        assert_eq!((trim.since_start.passes, trim.since_start.deleted_claims), (2, 4));
    }

    #[test]
    fn a_pass_that_halts_or_sets_a_cut_aside_is_partial_and_keeps_what_it_deleted() {
        let store = local_store();
        let chunk = |phase: &str, envelopes: usize, claims: usize| {
            let zero = std::time::Duration::ZERO;
            store.log_if_slow_trim_chunk(phase, zero, zero, envelopes, claims, 0, None)
        };
        // Two chunks commit, then the next would change the graph: the cut halts, and its action
        // carries no counts. Recording tombstones deletes nothing.
        store.retention.begin_trim_pass();
        chunk("record", 4, 4);
        chunk("envelopes", 1, 3);
        chunk("claims", 0, 2);
        let halted = [CheckpointAction::TrimGraphChanged { checkpoint: "checkpoint/cut".into() }];
        store.retention.finish_trim_pass(&halted, false, 7);
        let trim = store.retention_status().unwrap().trim;
        assert_eq!(
            trim.last_pass,
            Some(TrimPass {
                at_unix_ms: 7,
                stop: TrimStop::GraphChanged,
                outcome: TrimOutcome::Partial,
                trimmed_cuts: 0,
                deferred_cuts: 0,
                set_aside_cuts: 0,
                deleted_envelopes: 1,
                deleted_claims: 5,
            })
        );
        assert_eq!((trim.since_start.deleted_envelopes, trim.since_start.deleted_claims), (1, 5));

        // A cut set aside before any chunk committed pushes no action and deletes nothing; the
        // pass is partial all the same.
        store.retention.begin_trim_pass();
        store.set_checkpoint_state("checkpoint/young", 0, "set-aside").unwrap();
        store.retention.finish_trim_pass(&[], false, 8);
        let pass = store.retention_status().unwrap().trim.last_pass.unwrap();
        assert_eq!(
            (pass.outcome, pass.set_aside_cuts, pass.deleted_envelopes, pass.deleted_claims),
            (TrimOutcome::Partial, 1, 0, 0)
        );

        // A cut finished in the pass reports what its chunks deleted: complete.
        store.retention.begin_trim_pass();
        chunk("claims", 0, 2);
        let finished = [CheckpointAction::Trimmed { checkpoint: "checkpoint/cut".into(), envelopes: 0, claims: 2 }];
        store.retention.finish_trim_pass(&finished, false, 9);
        let trim = store.retention_status().unwrap().trim;
        let pass = trim.last_pass.unwrap();
        assert_eq!((pass.outcome, pass.stop, pass.set_aside_cuts), (TrimOutcome::Complete, TrimStop::NothingDue, 0));
        assert_eq!(pass.deleted_claims, 2);
        assert_eq!(
            (trim.since_start.passes, trim.since_start.set_aside_cuts, trim.since_start.deleted_claims),
            (3, 1, 7)
        );
    }

    #[test]
    fn legacy_receipts_show_pending_until_the_cleanup_passes_them() {
        let store = local_store();
        store
            .append_claim(&ClaimInput {
                subject: "daemon/writer".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([("code".into(), json!("retention-test"))]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        {
            let connection = store.connection.write();
            connection
                .execute("INSERT INTO idempotency(operation_id, response) VALUES ('legacy', '{}')", [])
                .unwrap();
            connection
                .execute(
                    "UPDATE meta SET value=(SELECT CAST(MAX(rowid) AS TEXT) FROM idempotency)
                     WHERE key='idempotency_legacy_rowid'",
                    [],
                )
                .unwrap();
        }
        let before = store.retention_status().unwrap().legacy_receipts;
        assert!(before.pending, "{before:?}");
        assert_eq!(before.oldest_rowid, before.boundary_rowid);

        let chunk = store.cleanup_idempotency(i64::MAX).unwrap();
        assert_eq!(chunk.examined, 1);
        let after = store.retention_status().unwrap().legacy_receipts;
        assert!(!after.pending, "{after:?}");
        assert_eq!(after.since_start.chunks, 1);
        assert_eq!(after.since_start.examined, 1);
        assert_eq!(after.since_start.deleted + after.since_start.extended, 1);
    }

    #[test]
    fn page_counts_add_up_since_start() {
        let store = local_store();
        let tally = &store.retention;
        tally.valid_record_bytes(&RecordBytesRelease { scanned: 0, ..Default::default() });
        tally.valid_record_bytes(&RecordBytesRelease { scanned: 3, emptied: 2, bytes: 70, ..Default::default() });
        tally.settled_judging(&JudgingPrune { examined: 5, deleted: 4, finished: true });
        let status = store.retention_status().unwrap();
        assert_eq!(
            status.valid_record_bytes.since_start,
            RecordBytesSinceStart { pages: 1, emptied_records: 2, emptied_bytes: 70 }
        );
        assert_eq!(status.settled_judging.since_start, JudgingSinceStart { pages: 1, examined: 5, deleted: 4 });
    }

    #[test]
    fn the_status_round_trips_and_an_older_daemon_sends_none() {
        let store = local_store();
        let status = store.retention_status().unwrap();
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(serde_json::from_value::<RetentionStatus>(value).unwrap(), status);
        // A daemon from before this view sends no field, which reads as unknown.
        let earlier = serde_json::to_value(crate::replication::ReplicationStatus::default()).unwrap();
        assert!(earlier.get("retention").is_none());
        let read: crate::replication::ReplicationStatus = serde_json::from_value(earlier).unwrap();
        assert!(read.retention.is_none());
        assert_eq!(serde_json::from_str::<RetentionStatus>("{}").unwrap(), RetentionStatus::default());
    }
}
