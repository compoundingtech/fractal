//! The checkpoint worker: one pass of a member's own trim, its phase, and the person's word that
//! lets a stopped trim go on. Members agree on no checkpoint with one another; each plans and
//! trims its own daily cut (`checkpoint_trim.rs`). The `checkpoint.*` claim kinds older builds
//! wrote are still admitted, and nothing here writes them.

use super::*;

pub const DAY_MS: u128 = 86_400_000;

#[cfg(feature = "test-support")]
thread_local! {
    static TEST_CHECKPOINT_IDENTITY: std::cell::Cell<Option<(&'static str, &'static str)>> = const { std::cell::Cell::new(None) };
}

/// Pin build identity for a simulation, so what names the build (inventories, staged sets,
/// answers checks) does not change when the test executable is rebuilt. Production keeps its
/// real identity.
/// The guard restores the enclosing thread's identity, including during unwinding.
#[cfg(feature = "test-support")]
#[must_use = "keep the guard alive throughout the simulation"]
pub fn checkpoint_test_identity(build: &'static str, executable: &'static str) -> impl Drop {
    struct Identity {
        previous: Option<(&'static str, &'static str)>,
        _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Identity {
        fn drop(&mut self) {
            TEST_CHECKPOINT_IDENTITY.with(|identity| identity.set(self.previous));
        }
    }
    Identity {
        previous: TEST_CHECKPOINT_IDENTITY.with(|identity| identity.replace(Some((build, executable)))),
        _thread: std::marker::PhantomData,
    }
}

#[cfg(all(test, feature = "test-support"))]
#[test]
fn simulation_identity_is_scoped_to_its_thread_and_restores_on_unwind() {
    let real_build = checkpoint_build();
    {
        let _outer = checkpoint_test_identity("fixture-build", "fixture-executable");
        assert_eq!(checkpoint_build(), "fixture-build");
        assert_eq!(checkpoint_executable_digest().unwrap(), "fixture-executable");
        std::thread::scope(|scope| {
            assert_eq!(scope.spawn(checkpoint_build).join().unwrap(), real_build);
        });
        let unwind = std::panic::catch_unwind(|| {
            let _inner = checkpoint_test_identity("nested-build", "nested-executable");
            assert_eq!(checkpoint_build(), "nested-build");
            panic!("fixture unwind");
        });
        assert!(unwind.is_err());
        assert_eq!(checkpoint_build(), "fixture-build");
        assert_eq!(checkpoint_executable_digest().unwrap(), "fixture-executable");
    }
    assert_eq!(checkpoint_build(), real_build);
}

/// The build this member names in its inventory and answers checks. `CARGO_PKG_VERSION` alone
/// is the same on every node today.
pub fn checkpoint_build() -> String {
    #[cfg(feature = "test-support")]
    if let Some((build, _)) = TEST_CHECKPOINT_IDENTITY.with(std::cell::Cell::get) {
        return build.to_owned();
    }
    option_env!("ST3_BUILD_REVISION")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_owned()
}

/// One admitted `checkpoint.*` claim. Lists of them are in canonical order.

/// What one pass of checkpoint work needs from outside the store.
#[derive(Clone, Debug)]
pub struct CheckpointContext {
    pub now_unix_ms: u128,
    /// A directory for the trim's staged set.
    pub scratch: PathBuf,
}

/// What one pass of checkpoint work did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum CheckpointAction {
    Trimmed {
        checkpoint: String,
        envelopes: usize,
        claims: usize,
    },
    /// A trim stopped because deleting would change the graph, or its plan would drop the
    /// newest claim of a subject and kind. No trim runs until a person looks.
    TrimGraphChanged {
        checkpoint: String,
    },
    /// A trim stopped before its end: it deleted the day's claims (`[trim] daily_claims`) and
    /// goes on the next day, or it could not restart the WAL (`TrimWal`) and goes on at the next
    /// step. The cut stays `trimming`. The counts are this step's.
    TrimDeferred {
        checkpoint: String,
        envelopes: usize,
        claims: usize,
    },
    /// Signature and verdict rows of claims an earlier build's trim dropped were deleted.
    SweptTrimmedClaimRows {
        rows: usize,
    },
}

/// What a member's own trim is doing: the newest cut it trimmed, whether a trim stopped and
/// waits for a person, and the worker's phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrimStatusView {
    pub node: String,
    /// `[trim] local`: whether this member trims at all.
    pub trims: bool,
    /// The newest cut this member trimmed.
    pub trimmed: Option<String>,
    /// A trim stopped because the graph would change, and waits for a person.
    pub halted: bool,
    /// Process-local worker evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<CheckpointWorker>,
}

impl TrimStatusView {
    /// Why trimming here waits for a person or failed, if it does.
    pub fn blocking_summary(&self) -> Option<String> {
        let failed = self.worker.as_ref().and_then(|worker| worker.error.as_ref());
        if !self.halted && failed.is_none() {
            return None;
        }
        let mut details = Vec::new();
        if self.halted {
            details.push("a trim stopped because the graph would change; \
                `fractal replication checkpoint resume` goes on once a person looked".to_owned());
        }
        if let (Some(worker), Some(error)) = (&self.worker, failed) {
            details.push(format!("phase={:?} failed: {error}", worker.phase));
        }
        Some(format!("{}: {}", self.node, details.join("; ")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPhase {
    Disabled,
    Activation,
    ScratchCleanup,
    Sweep,
    Rebuild,
    CatchingUp,
    LocalTrim,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointWorker {
    pub phase: CheckpointPhase,
    pub updated_at_unix_ms: u128,
    /// Failure stays attached to the phase that failed until the next attempt starts.
    pub error: Option<String>,
}

/// A person's word that a trim which stopped because the graph would change may be left as it
/// is, so trimming goes on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointResumeRequest {
    pub reason: String,
    pub actor: String,
}

/// Release labels can be just the package version. Hash the actual running code once, rather
/// than making a reader/projection fix depend on somebody remembering to bump a rules version.
/// On Linux the proc link still names this process's binary after an atomic deployment replaces
/// its path. Never silently reuse a failed result if executable identity cannot be established.
pub(super) fn checkpoint_executable_digest() -> Result<&'static str> {
    #[cfg(feature = "test-support")]
    if let Some((_, executable)) = TEST_CHECKPOINT_IDENTITY.with(std::cell::Cell::get) {
        return Ok(executable);
    }
    use std::io::Read as _;
    static DIGEST: std::sync::LazyLock<std::result::Result<String, String>> = std::sync::LazyLock::new(|| {
        let hash = || -> Result<String> {
            #[cfg(target_os = "linux")]
            let path = PathBuf::from("/proc/self/exe");
            #[cfg(not(target_os = "linux"))]
            let path = std::env::current_exe()?;
            let mut executable = fs::File::open(path)?;
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 64 * 1_024];
            loop {
                let read = executable.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            Ok(hex::encode(digest.finalize()))
        };
        hash().map_err(|error| format!("{error:#}"))
    });
    DIGEST.as_ref().map(String::as_str)
        .map_err(|error| anyhow::anyhow!("checkpoint executable identity unavailable: {error}"))
}

/// Envelope identities cover immutable claim content, but not admission state or the
/// materialized tombstones left by earlier trims. Include that closure and canonical claim
/// order as well as the plan. A build change may fix reader/projection semantics without

impl Store {
    /// One pass of checkpoint work: finish a trim a crash or the daily cap left, then trim this
    /// member's own due cut. A node that is catching up with a peer does nothing, since what it
    /// holds before the cut is still changing. Call it after exchanges and every few minutes,
    /// off the request path.
    pub fn checkpoint_step(&self, context: &CheckpointContext) -> Result<Vec<CheckpointAction>> {
        self.checkpoint_worker_phase(CheckpointPhase::Sweep);
        let result = self.checkpoint_step_inner(context);
        if let Err(error) = &result {
            self.checkpoint_worker_error(format!("{error:#}"));
        }
        result
    }

    pub fn checkpoint_worker_phase(&self, phase: CheckpointPhase) {
        *self.checkpoint_worker.lock().unwrap_or_else(PoisonError::into_inner) = Some(CheckpointWorker {
            phase, updated_at_unix_ms: now_ms(), error: None,
        });
    }

    pub fn checkpoint_worker_error(&self, error: String) {
        if let Some(worker) = self.checkpoint_worker.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            worker.error = Some(error);
            worker.updated_at_unix_ms = now_ms();
        }
    }

    fn checkpoint_step_inner(&self, context: &CheckpointContext) -> Result<Vec<CheckpointAction>> {
        self.runtime.checkpoint_preflight()?;
        let mut actions = Vec::new();
        // Local rows outside every digest, so this does not wait for a peer.
        let rows = self.sweep_trimmed_claim_rows()?;
        if rows > 0 {
            actions.push(CheckpointAction::SweptTrimmedClaimRows { rows });
        }
        // What a trim deferred and a crash left before its rebuild.
        self.checkpoint_worker_phase(CheckpointPhase::Rebuild);
        self.rebuild_deferred_paced()?;
        if self.replication_catching_up() {
            self.checkpoint_worker_phase(CheckpointPhase::CatchingUp);
            return Ok(actions);
        }
        // Every member trims on its own: it finishes a trim it had already committed, then trims
        // its own due cut. No member agrees on a checkpoint with another or waits for one.
        if !self.trims_locally() {
            self.checkpoint_worker_phase(CheckpointPhase::Disabled);
            return Ok(actions);
        }
        self.checkpoint_worker_phase(CheckpointPhase::LocalTrim);
        self.trim_day.store(
            u64::try_from(context.now_unix_ms / super::checkpoint::DAY_MS)?,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.finish_interrupted_trims(&mut actions)?;
        self.trim_local_cut(context, &mut actions)?;
        Ok(actions)
    }

    /// Whether a trim stopped because the graph would change. Trimming then waits for a person;
    /// see `resume_checkpoints`.
    pub fn checkpoint_halted(&self) -> Result<bool> {
        Ok(self
            .readers
            .get()
            .query_row(
                "SELECT 1 FROM checkpoints WHERE state='graph-changed' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// A person looked at a trim that stopped because the graph would change. The rows stay
    /// as they are, and trimming goes on from the next cut.
    pub fn resume_checkpoints(&self, actor: &str, reason: &str) -> Result<(), FractalError> {
        self.resume_checkpoints_allowing(actor, reason, None)
    }

    /// As [`Self::resume_checkpoints`], and on a member that trims `agent` may resume too: the agent its stop alerts (`[trim] notify`), which owns the fault. Resuming past a
    /// stop is the safety decision, so no other agent may.
    pub fn resume_checkpoints_allowing(
        &self,
        actor: &str,
        reason: &str,
        agent: Option<&str>,
    ) -> Result<(), FractalError> {
        self.runtime.checkpoint_preflight().map_err(internal)?;
        let actor = normalize_actor(actor, "person");
        let allowed = actor.starts_with("person/") || (self.trims_locally() && agent == Some(actor.as_str()));
        if !allowed || reason.trim().is_empty() {
            return Err(FractalError::new(
                "invalid-checkpoint-resume",
                "a person resumes checkpoints, with a reason (or, where trim is local, the [trim] notify agent)",
            ));
        }
        let mut connection = self.connection.write();
        let transaction = connection.transaction().map_err(internal)?;
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h7e11b486a2fc owner=smallclaims::store::checkpoint_agreement TODO=stage-generation-and-keyed-reclamation */ "UPDATE checkpoints SET state='set-aside', detail=?1, updated_at_unix_ms=?2
                 WHERE state='graph-changed'",
                params![
                    json!({"resumed_by": actor, "reason": reason}).to_string(),
                    i64::try_from(now_ms()).unwrap_or(i64::MAX)
                ],
            )
            .map_err(internal)?;
        transaction.commit().map_err(internal)?;
        Ok(())
    }


    /// What `fractal doctor` reports of this member's trim.
    pub fn trim_status(&self) -> Result<TrimStatusView> {
        let trimmed = self
            .readers
            .get()
            .query_row(
                "SELECT id FROM checkpoints WHERE state='trimmed' ORDER BY cut_unix_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(TrimStatusView {
            node: self.origin.clone(),
            trims: self.trims_locally(),
            trimmed,
            halted: self.checkpoint_halted()?,
            worker: self.checkpoint_worker.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        })
    }

    /// Shift the clock this store dates its writes by. A simulation of machines with skewed
    /// clocks or days passing sets it; nothing else does.
    pub fn set_write_clock_offset(&self, offset_ms: i64) -> Result<()> {
        let connection = self.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-hf7027d3cb134 owner=smallclaims::store::checkpoint_agreement TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM temp.write_clock", [])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h4da262ac8192 owner=smallclaims::store::checkpoint_agreement TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO temp.write_clock(offset_ms) VALUES (?1)",
            [offset_ms],
        )?;
        Ok(())
    }

    /// Date this store's writes at `at_unix_ms` until its clock is set again. Claim IDs hash
    /// the time they were written, so a simulation that sets this before each step writes the
    /// same claims however fast it runs. Nothing else sets it.
    pub fn set_write_clock_at(&self, at_unix_ms: u128) -> Result<()> {
        let connection = self.connection.write();
        connection.execute(/* sqlite-large-value: allow-for-now lv-h956c01951452 owner=smallclaims::store::checkpoint_agreement TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM temp.write_clock", [])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hf41fabef76b6 owner=smallclaims::store::checkpoint_agreement TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO temp.write_clock(offset_ms, at_ms) VALUES (0, ?1)",
            [i64::try_from(at_unix_ms)?],
        )?;
        Ok(())
    }
}

/// The accepted time of a new batch or claim by `origin`: the clock, but never before the cut
/// of a checkpoint this node has recorded, and never before `origin`'s newest batch, so nothing
/// this node writes afterwards is dated before a cut it trimmed. Folds read claims in canonical order, which starts with the accepted time, so a writer
/// whose clock stepped back would otherwise date its new claims before its older ones.
/// `temp.write_clock` shifts or fixes the clock for a simulation; see
/// `Store::set_write_clock_offset` and `Store::set_write_clock_at`.
pub fn write_time(connection: &Connection, origin: &str) -> Result<u128> {
    let floor: Option<i64> = connection
        .prepare_cached("SELECT MAX(cut_unix_ms) FROM checkpoints")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h493b95db430a owner=smallclaims::store::checkpoint_agreement TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
    let newest_own: Option<String> = connection
        .prepare_cached(
            "SELECT accepted_at_unix_ms FROM batches WHERE origin=?1
             ORDER BY replica_sequence DESC LIMIT 1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hedbb3482a27f owner=smallclaims::store::checkpoint_agreement TODO=trace-statement-and-value-representation */ [origin], |row| row.get(0))
        .optional()?;
    let (offset, at): (i64, Option<i64>) = connection
        .prepare_cached("SELECT offset_ms, at_ms FROM temp.write_clock")
        .and_then(|mut statement| statement.query_row(/* sqlite-large-value: allow-for-now lv-h34c3494cd92e owner=smallclaims::store::checkpoint_agreement TODO=trace-statement-and-value-representation */ [], |row| Ok((row.get(0)?, row.get(1)?))))
        .unwrap_or((0, None));
    let now = match at {
        Some(at) => i128::from(at),
        None => i128::try_from(now_ms()).unwrap_or(i128::MAX) + i128::from(offset),
    };
    let now = u128::try_from(now.max(0)).unwrap_or(0);
    let floor = floor
        .and_then(|floor| u128::try_from(floor).ok())
        .unwrap_or(0);
    let newest_own = newest_own
        .and_then(|accepted| accepted.parse::<u128>().ok())
        .unwrap_or(0);
    Ok(now.max(floor).max(newest_own))
}
