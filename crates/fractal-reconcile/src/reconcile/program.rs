//! The program engine: runs `program` steps (docs/st3/program-engine.md).
//!
//! The daemon owns everything durable about a program run: which step runs, its journal (one
//! `program.step` claim per inner step result), replay after a restart, and the step's outcome.
//! Program bodies are evaluated by one supervised host process per member
//! (`program_host.mjs`, on Bun), which talks to this module over its stdin and stdout only (host
//! protocol v1, §2). That pipe is the host's only write path, so a host orphaned by a dead daemon
//! can never journal anything.
//!
//! - daemon → host: `start`, `ack`, `nack`, `cancel`, `resolved`, `ping`.
//! - host → daemon: `journal`, `run`, `ask`, `done`, `unresponsive`, `pong`.
//!
//! Every message after `start` names its step and attempt; both sides drop a message for an
//! attempt or host generation they no longer run. Supervision: the engine pings the host and
//! replaces one that does not answer within its deadline; a replaced or crashed host's programs
//! resume from their journals on the next host. Before it starts a host, the engine kills the
//! effect process groups an earlier host recorded and never ended (DQ2).

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::MissionsStore;
use crate::model::{ClaimInput, ClaimRecord, FractalError, MissionRunView, ProgramSpec};
use crate::store::Store;

/// The claim kind of a program journal entry, on the program's `step-run` subject.
pub const JOURNAL_KIND: &str = "program.step";
/// The journal key of a program's own encoded success value, written when it ends.
pub const RESULT_KEY: &str = "$result";
/// The step states in which the engine runs a program step's current attempt. A step waiting on
/// a person keeps its program loaded (§11).
pub const RUNNING_STATES: [&str; 4] = ["ready", "working", "blocked", "waiting-person"];
/// A run credential has no expiry of its own: it binds only while its attempt is its step run's
/// current, live attempt, however long that attempt waits on an ask or a child run.
const RUN_CREDENTIAL_LIFETIME_MS: u64 = u64::MAX;
const HOST_SOURCE: &str = /* sqlite-large-value: allow-for-now lv-h344eb4debbcf owner=fractal_reconcile::reconcile::program TODO=trace-statement-and-value-representation */ include_str!("program_host.mjs");
/// How long a crashed host waits before its replacement starts, so a host that cannot start does
/// not spin.
const HOST_RESTART_DELAY: Duration = Duration::from_millis(500);
const JOURNAL_PAGE: usize = 500;

/// The liveness deadlines of DQ8.
#[derive(Clone, Copy, Debug)]
pub struct Liveness {
    /// A program worker that does not answer the host's ping within this is terminated and its
    /// step fails.
    pub worker_deadline: Duration,
    /// How often the engine pings the host.
    pub host_ping_every: Duration,
    /// A host that does not answer a ping within this is replaced.
    pub host_deadline: Duration,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            worker_deadline: Duration::from_secs(30),
            host_ping_every: Duration::from_secs(5),
            host_deadline: Duration::from_secs(30),
        }
    }
}

/// One program step this daemon runs.
#[derive(Clone, Debug)]
pub struct Dispatch {
    pub step: String,
    pub attempt: u32,
    /// The `mission-run/…` subject the step belongs to.
    pub run: String,
    pub program: ProgramSpec,
    pub payload: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub cwd: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Dispatched {
    /// The host already runs this attempt.
    Running,
    /// The host was asked to start (or resume) it.
    Started,
    /// The step cannot run; the reason fails it.
    Refused(String),
    /// No host could be started to run it; the step keeps its state and the engine reports the
    /// reason through [`ProgramEngine::host_fault`].
    HostDown(String),
}

struct Host {
    child: Child,
    /// Lines for the host's stdin. A writer thread owns the pipe, so neither the reconciler nor the
    /// reader thread ever blocks on a host that stopped reading.
    lines: mpsc::Sender<Vec<u8>>,
    generation: u64,
    /// The unanswered ping and when it was sent.
    ping: Option<(u64, Instant)>,
}

struct Running {
    dispatch: Dispatch,
    /// The host generation running it; a host that exits takes exactly its own programs with it.
    generation: u64,
}

#[derive(Default)]
struct Inner {
    host: Option<Host>,
    generations: u64,
    pings: u64,
    supervised: bool,
    running: HashMap<String, Running>,
    /// By step: the attempt and the last host or liveness failure that stopped it.
    faults: HashMap<String, (u32, String)>,
}

pub struct ProgramEngine {
    store: Arc<Store>,
    wake: Arc<dyn Fn(&str) + Send + Sync>,
    dir: PathBuf,
    liveness: Liveness,
    inner: Mutex<Inner>,
}

impl ProgramEngine {
    /// `wake(step)` tells the reconciler a program step changed outside its evaluation.
    pub fn new(store: Arc<Store>, wake: Arc<dyn Fn(&str) + Send + Sync>, dir: PathBuf, liveness: Liveness) -> Arc<Self> {
        Arc::new(Self { store, wake, dir, liveness, inner: Mutex::default() })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start or resume `dispatch` unless the live host already runs that attempt.
    pub fn dispatch(self: &Arc<Self>, dispatch: Dispatch) -> Result<Dispatched> {
        if self.runs(&dispatch) {
            return Ok(Dispatched::Running);
        }
        let module = match self.pinned_module(&dispatch.program) {
            Ok(module) => module,
            Err(reason) => return Ok(Dispatched::Refused(reason)),
        };
        let journal = self.journal(&dispatch.step, dispatch.attempt)?;
        let mut inner = self.lock();
        if inner.running.get(&dispatch.step).is_some_and(|running| {
            running.dispatch.attempt == dispatch.attempt && inner.host.as_ref().is_some_and(|host| host.generation == running.generation)
        }) {
            return Ok(Dispatched::Running);
        }
        let generation = match self.ensure_host(&mut inner) {
            Ok(generation) => generation,
            Err(error) => {
                let reason = format!("the program host cannot start: {error:#}");
                inner.faults.insert(dispatch.step.clone(), (dispatch.attempt, reason.clone()));
                return Ok(Dispatched::HostDown(reason));
            }
        };
        inner.faults.remove(&dispatch.step);
        // The attempt's effects act as its run (docs/st3/program-engine.md §3). The credential is
        // minted only here, once the host runs and this attempt is about to start, so a refused,
        // already running or host-down dispatch leaves no row; each start or resume mints one, and
        // it never enters the journal or a log.
        let mut env = dispatch.env.clone();
        env.insert(
            crate::store::run_credentials::RUN_CREDENTIAL_ENV.to_owned(),
            self.store.issue_run_credential(&dispatch.step, dispatch.attempt, RUN_CREDENTIAL_LIFETIME_MS)?,
        );
        eprintln!(
            "fractal: program {} attempt {} started with {} journaled steps",
            dispatch.step,
            dispatch.attempt,
            journal.len()
        );
        let message = json!({
            "t": "start",
            "step": dispatch.step,
            "attempt": dispatch.attempt,
            "run": dispatch.run,
            "module": module,
            "sha256": dispatch.program.bundle_hash,
            "payload": dispatch.payload,
            "env": env,
            "cwd": dispatch.cwd,
            "journal": journal,
        });
        inner.running.insert(dispatch.step.clone(), Running { dispatch, generation });
        send(&inner, &message);
        Ok(Dispatched::Started)
    }

    fn runs(&self, dispatch: &Dispatch) -> bool {
        let inner = self.lock();
        let live = inner.host.as_ref().map(|host| host.generation);
        inner
            .running
            .get(&dispatch.step)
            .is_some_and(|running| running.dispatch.attempt == dispatch.attempt && Some(running.generation) == live)
    }

    /// The last host or liveness failure that stopped this attempt, until a dispatch of the step
    /// starts again.
    pub fn host_fault(&self, step: &str, attempt: u32) -> Option<String> {
        self.lock().faults.get(step).filter(|(at, _)| *at == attempt).map(|(_, reason)| reason.clone())
    }

    /// Eager cancellation (§6), called from every evaluation of a run: each program attempt of
    /// `run` this engine runs whose step left it (ended, failed, timed out, retried, cancelled with
    /// its run or replaced by a new generation) is stopped now, with its effects.
    pub fn release_left(&self, run: &MissionRunView) {
        let terminal = matches!(run.status.as_str(), "completed" | "failed" | "cancelled");
        let mut inner = self.lock();
        let left = inner
            .running
            .iter()
            .filter(|(_, running)| running.dispatch.run == run.subject)
            .filter_map(|(step, running)| {
                let attempt = running.dispatch.attempt;
                let reason = match run.steps.iter().find(|view| view.subject == *step) {
                    None => Some("the run moved to another generation".to_owned()),
                    Some(_) if terminal => Some(format!("the run is {}", run.status)),
                    Some(view) if view.attempt != attempt => Some(format!("the step moved to attempt {}", view.attempt)),
                    Some(view) if !RUNNING_STATES.contains(&view.status.as_str()) => Some(format!("the step is {}", view.status)),
                    Some(_) => None,
                };
                reason.map(|reason| (step.clone(), attempt, reason))
            })
            .collect::<Vec<_>>();
        for (step, attempt, reason) in left {
            inner.running.remove(&step);
            eprintln!("fractal: program {step} attempt {attempt} cancelled: {reason}");
            send(&inner, &json!({"t": "cancel", "step": step, "attempt": attempt, "reason": reason}));
        }
    }

    /// The bundle as this revision pinned it, read from the store's blob of its hash (never from
    /// the author's path) into a verified, content-addressed file the host imports.
    fn pinned_module(&self, program: &ProgramSpec) -> std::result::Result<PathBuf, String> {
        let hash = &program.bundle_hash;
        let pinned = self.dir.join("bundles").join(format!("{hash}.mjs"));
        if std::fs::read(&pinned).is_ok_and(|bytes| sha256(&bytes) == *hash) {
            return Ok(pinned);
        }
        let bytes = self
            .store
            .get_blob(hash)
            .map_err(|error| format!("reading program bundle {hash}: {error:#}"))?
            .ok_or_else(|| format!("program bundle `{}` (sha256 {hash}) is not in this member's store", program.bundle))?;
        let actual = sha256(&bytes);
        if actual != *hash {
            return Err(format!("the stored program bundle {hash} has sha256 {actual}"));
        }
        write_atomically(&pinned, &bytes).map_err(|error| format!("pinning program bundle {hash}: {error:#}"))?;
        Ok(pinned)
    }

    /// The entries replay returns: the completed steps and typed failures of this attempt, in
    /// store order. A retried attempt starts from the top, because an earlier attempt's
    /// compensation may have undone what its completed steps did.
    fn journal(&self, step: &str, attempt: u32) -> Result<Vec<Value>> {
        let mut claims = Vec::new();
        let mut before = None;
        loop {
            // Newest first; each page continues below the oldest index of the previous one.
            let page = self.store.claims_for_subject_kind_at(step, JOURNAL_KIND, before, true, JOURNAL_PAGE)?;
            claims.extend(page.claims);
            match page.next_cursor {
                Some(cursor) => before = Some(cursor),
                None => break,
            }
        }
        claims.sort_by_key(|claim| claim.store_index);
        let mut entries = Vec::new();
        for claim in claims {
            let fields = &claim.body["fields"];
            if fields["attempt"].as_u64() != Some(u64::from(attempt)) || fields["key"] == RESULT_KEY {
                continue;
            }
            match fields["status"].as_str() {
                Some("completed") => entries.push(json!({"key": fields["key"], "_tag": "Success", "value": fields["value"]})),
                Some("failed") => entries.push(json!({"key": fields["key"], "_tag": "Failure", "error": fields["error"]})),
                _ => {}
            }
        }
        Ok(entries)
    }

    /// The live host's generation, starting one when there is none or the last one exited.
    fn ensure_host(self: &Arc<Self>, inner: &mut Inner) -> Result<u64> {
        if let Some(host) = inner.host.as_mut() {
            if host.child.try_wait()?.is_none() {
                return Ok(host.generation);
            }
            // Its reader thread resumes the programs it ran (see `host_exited`).
            if let Some(mut host) = inner.host.take() {
                let _ = host.child.wait();
            }
        }
        // No host runs now, so every recorded group belongs to one that is gone (DQ2).
        let ended = self.reap_recorded_groups()?;
        if ended > 0 {
            eprintln!("fractal: ended {ended} effect process groups an earlier program host left");
        }
        let script = self.dir.join(format!("host-{}.mjs", &sha256(HOST_SOURCE.as_bytes())[..12]));
        if !script.is_file() {
            write_atomically(&script, HOST_SOURCE.as_bytes())?;
        }
        let runtime = runtime()?;
        let mut child = Command::new(&runtime)
            .arg(&script)
            .current_dir(&self.dir)
            .env("FRACTAL_PROGRAM_GROUPS", self.groups_dir())
            .env("FRACTAL_PROGRAM_BOOT_ID", boot_id()?)
            .env("FRACTAL_PROGRAM_WORKER_DEADLINE_MS", self.liveness.worker_deadline.as_millis().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting the program host with {}", runtime.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("program host has no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("program host has no stdout"))?;
        inner.generations += 1;
        let generation = inner.generations;
        eprintln!("fractal: program host {generation} started (pid {})", child.id());
        let (lines, queue) = mpsc::channel();
        std::thread::Builder::new()
            .name(format!("program-host-{generation}-in"))
            .spawn(move || write_host(stdin, queue))?;
        let engine = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("program-host-{generation}-out"))
            .spawn(move || engine.read_host(stdout, generation))?;
        inner.host = Some(Host { child, lines, generation, ping: None });
        if !inner.supervised {
            inner.supervised = true;
            let engine = Arc::downgrade(self);
            let every = self.liveness.host_ping_every;
            std::thread::Builder::new()
                .name("program-host-liveness".into())
                .spawn(move || supervise(engine, every))?;
        }
        Ok(generation)
    }

    fn groups_dir(&self) -> PathBuf {
        self.dir.join("groups")
    }

    /// End the effect process groups recorded by hosts that are gone (DQ2). A record from an
    /// earlier boot is dropped without a kill, so a reused pid is never signalled; a group is
    /// killed when its leader still has the recorded start time, or when its leader is gone while
    /// the group lives on. Returns how many groups it killed.
    pub fn reap_recorded_groups(&self) -> Result<usize> {
        let dir = self.groups_dir();
        std::fs::create_dir_all(&dir)?;
        let boot = boot_id()?;
        let mut killed = 0;
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            // A `.tmp` record is one the host was renaming into place when it died: its group was
            // already running, so it is verified like any other record.
            let record: Value = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
            let pid = record["pid"].as_i64().and_then(|pid| i32::try_from(pid).ok()).filter(|pid| *pid > 1);
            if let Some(pid) = pid
                && record["boot_id"].as_str() == Some(boot.as_str())
            {
                let ours = match (leader_gone(pid), start_time(pid), record["start_time"].as_str()) {
                    // The leader is gone: anything left in its group is the effect's.
                    (true, _, _) => true,
                    (false, Some(now), Some(recorded)) => now == recorded,
                    // A live leader whose start time cannot be read is never signalled; the
                    // record stays for the next host start.
                    (false, None, _) => continue,
                    (false, Some(_), None) => false,
                };
                // SAFETY: signalling a process group has no memory effects.
                if ours && unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
                    killed += 1;
                }
            }
            std::fs::remove_file(&path)?;
        }
        Ok(killed)
    }

    /// The host process's pid, while one runs.
    pub fn host_pid(&self) -> Option<u32> {
        self.lock().host.as_ref().map(|host| host.child.id())
    }

    fn read_host(self: Arc<Self>, stdout: ChildStdout, generation: u64) {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let message = match serde_json::from_str::<Value>(&line) {
                Ok(message) => message,
                Err(error) => {
                    eprintln!("fractal: program host sent an unreadable line ({error}): {line}");
                    continue;
                }
            };
            if let Err(error) = self.handle(&message, generation) {
                eprintln!("fractal: program host message {line}: {error:#}");
            }
        }
        self.host_exited(generation);
    }

    fn handle(&self, message: &Value, generation: u64) -> Result<()> {
        if message["t"] == "pong" {
            let mut inner = self.lock();
            if let Some(host) = inner.host.as_mut().filter(|host| host.generation == generation)
                && host.ping.is_some_and(|(seq, _)| message["seq"].as_u64() == Some(seq))
            {
                host.ping = None;
            }
            return Ok(());
        }
        let step = message["step"].as_str().ok_or_else(|| anyhow!("message without a step"))?;
        let attempt = message["attempt"].as_u64().ok_or_else(|| anyhow!("message without an attempt"))?;
        let program = {
            let inner = self.lock();
            match inner.running.get(step) {
                // A message from an attempt or host this engine no longer runs changes nothing.
                Some(running) if u64::from(running.dispatch.attempt) == attempt && running.generation == generation => {
                    running.dispatch.program.bundle_hash.clone()
                }
                _ => return Ok(()),
            }
        };
        let attempt = u32::try_from(attempt)?;
        match message["t"].as_str() {
            Some("journal") => self.journal_entry(step, attempt, generation, &program, message),
            Some("done") => self.done(step, attempt, generation, &program, message),
            Some("unresponsive") => self.unresponsive(step, attempt, generation, message),
            other => Err(anyhow!("unknown message type {other:?}")),
        }
    }

    /// Why the step no longer runs this attempt, if it does not.
    fn left(&self, step: &str, attempt: u32) -> Result<Option<String>> {
        Ok(match self.store.step_run(step)? {
            Some(view) => (view.attempt != attempt || !RUNNING_STATES.contains(&view.status.as_str()))
                .then(|| format!("the step is {} at attempt {}", view.status, view.attempt)),
            None => Some("the step run no longer exists".to_owned()),
        })
    }

    /// The backstop to [`Self::release_left`]: stop the program if its step left this attempt.
    /// True when it was stopped.
    fn stop_if_left(&self, step: &str, attempt: u32, generation: u64) -> Result<bool> {
        let Some(reason) = self.left(step, attempt)? else { return Ok(false) };
        let mut inner = self.lock();
        if forget(&mut inner, step, attempt, generation) {
            send_to(&inner, generation, &json!({"t": "cancel", "step": step, "attempt": attempt, "reason": reason}));
        }
        Ok(true)
    }

    /// Append one journal entry. A completed or failed entry is fenced by its key: one result per
    /// key and attempt, so a second, different result is refused (R03).
    #[allow(clippy::too_many_arguments)]
    fn write_journal(
        &self,
        step: &str,
        attempt: u32,
        program: &str,
        key: &str,
        status: &str,
        result: &Value,
        child: Option<&str>,
    ) -> std::result::Result<ClaimRecord, FractalError> {
        let mut fields = BTreeMap::from([
            ("key".to_owned(), Value::from(key)),
            ("status".to_owned(), Value::from(status)),
            ("attempt".to_owned(), Value::from(attempt)),
            ("program".to_owned(), Value::from(program)),
        ]);
        for name in ["value", "error", "ms"] {
            if let Some(value) = result.get(name).filter(|value| !value.is_null()) {
                fields.insert(name.to_owned(), value.clone());
            }
        }
        if let Some(child) = child {
            fields.insert("child".to_owned(), Value::from(child));
        }
        // The run writes its own journal, as the actor its effects act as.
        let run = self.store.step_run(step).map_err(smallclaims::error::typed)?.map(|view| view.run);
        self.store.append_claim(&ClaimInput {
            subject: step.to_owned(),
            kind: JOURNAL_KIND.to_owned(),
            actor: run,
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: (status != "started").then(|| format!("program:{step}:{attempt}:{key}:result")),
        })
    }

    fn journal_entry(&self, step: &str, attempt: u32, generation: u64, program: &str, message: &Value) -> Result<()> {
        if self.stop_if_left(step, attempt, generation)? {
            return Ok(());
        }
        let key = message["key"].as_str().unwrap_or_default();
        let status = message["status"].as_str().unwrap_or_default();
        let written = if key.is_empty() || !["started", "completed", "failed"].contains(&status) {
            Err(format!("a journal entry needs a key and a status, got key `{key}` status `{status}`"))
        } else if key == RESULT_KEY {
            Err(format!("a program may not journal the reserved key {RESULT_KEY}"))
        } else {
            self.write_journal(step, attempt, program, key, status, message, None).map(|_| ()).map_err(|error| error.to_string())
        };
        let seq = &message["seq"];
        if status == "started" || seq.is_null() {
            return written.map_err(|error| anyhow!(error));
        }
        let inner = self.lock();
        // Only the attempt and host that sent the entry hear about it (R01).
        if !matches!(inner.running.get(step), Some(running) if running.dispatch.attempt == attempt && running.generation == generation) {
            return Ok(());
        }
        match written {
            Ok(()) => send_to(&inner, generation, &json!({"t": "ack", "step": step, "attempt": attempt, "seq": seq})),
            // The program must not continue past a result that is not durable: the step fails.
            Err(error) => send_to(
                &inner,
                generation,
                &json!({"t": "nack", "step": step, "attempt": attempt, "seq": seq, "error": format!("journal write refused: {error}")}),
            ),
        }
        Ok(())
    }

    fn done(&self, step: &str, attempt: u32, generation: u64, program: &str, message: &Value) -> Result<()> {
        if !forget(&mut self.lock(), step, attempt, generation) {
            return Ok(());
        }
        if self.left(step, attempt)?.is_some() {
            return Ok(());
        }
        let exit = &message["exit"];
        let (status, reason) = match exit["_tag"].as_str() {
            Some("Success") => match self.write_journal(step, attempt, program, RESULT_KEY, "completed", &json!({"value": exit["value"]}), None) {
                Ok(_) => ("completed", None),
                Err(error) => ("failed", Some(format!("the program's result could not be journaled: {error}"))),
            },
            _ => ("failed", Some(failure_reason(&exit["error"]))),
        };
        self.store.set_step_state_unless_terminal(step, status, reason.as_deref())?;
        eprintln!("fractal: program {step} {status}");
        (self.wake)(step);
        Ok(())
    }

    /// The host terminated this attempt's worker after it missed its liveness deadline (DQ8): the
    /// step fails once instead of looping across restarts.
    fn unresponsive(&self, step: &str, attempt: u32, generation: u64, message: &Value) -> Result<()> {
        let ms = message["ms"].as_u64().unwrap_or_default();
        let reason = format!("the program's worker missed its {ms} ms liveness deadline and was terminated");
        {
            let mut inner = self.lock();
            if !forget(&mut inner, step, attempt, generation) {
                return Ok(());
            }
            inner.faults.insert(step.to_owned(), (attempt, reason.clone()));
        }
        eprintln!("fractal: program {step} attempt {attempt}: {reason}");
        if self.left(step, attempt)?.is_none() {
            self.store.set_step_state_unless_terminal(step, "failed", Some(&reason))?;
        }
        (self.wake)(step);
        Ok(())
    }

    /// Ping the host; replace one whose last ping went unanswered past its deadline.
    fn ping_host(&self) {
        let mut inner = self.lock();
        inner.pings += 1;
        let seq = inner.pings;
        let deadline = self.liveness.host_deadline;
        let Some(host) = inner.host.as_mut() else { return };
        match host.ping {
            Some((_, sent)) if sent.elapsed() > deadline => {
                eprintln!(
                    "fractal: program host {} did not answer a ping within {} ms; replacing it",
                    host.generation,
                    deadline.as_millis()
                );
                // Its reader thread sees the pipe close and resumes its programs on a new host.
                let _ = host.child.kill();
            }
            Some(_) => {}
            None => {
                host.ping = Some((seq, Instant::now()));
                let _ = host.lines.send(format!("{{\"t\":\"ping\",\"seq\":{seq}}}\n").into_bytes());
            }
        }
    }

    /// A host that exits takes its programs with it; a new host resumes each from its journal.
    fn host_exited(self: Arc<Self>, generation: u64) {
        let resume = {
            let mut inner = self.lock();
            if inner.host.as_ref().is_some_and(|host| host.generation == generation)
                && let Some(mut host) = inner.host.take()
            {
                let _ = host.child.kill();
                let status = host.child.wait();
                eprintln!("fractal: program host {generation} exited ({status:?})");
            }
            let gone = inner
                .running
                .iter()
                .filter(|(_, running)| running.generation == generation)
                .map(|(step, _)| step.clone())
                .collect::<Vec<_>>();
            gone.iter().filter_map(|step| inner.running.remove(step)).map(|running| running.dispatch).collect::<Vec<_>>()
        };
        if resume.is_empty() {
            return;
        }
        std::thread::sleep(HOST_RESTART_DELAY);
        for dispatch in resume {
            let (step, attempt) = (dispatch.step.clone(), dispatch.attempt);
            if !matches!(self.left(&step, attempt), Ok(None)) {
                continue;
            }
            let failure = match self.dispatch(dispatch) {
                Ok(Dispatched::Refused(reason)) => {
                    if let Err(error) = self.store.set_step_state_unless_terminal(&step, "failed", Some(&reason)) {
                        eprintln!("fractal: failing program {step}: {error:#}");
                    }
                    None
                }
                Ok(Dispatched::HostDown(reason)) => Some(reason),
                Ok(Dispatched::Running | Dispatched::Started) => continue,
                Err(error) => Some(format!("resuming the program after its host exited: {error:#}")),
            };
            if let Some(reason) = failure {
                eprintln!("fractal: program {step}: {reason}");
                self.lock().faults.insert(step.clone(), (attempt, reason));
            }
            (self.wake)(&step);
        }
    }
}

fn supervise(engine: Weak<ProgramEngine>, every: Duration) {
    loop {
        std::thread::sleep(every);
        let Some(engine) = engine.upgrade() else { return };
        engine.ping_host();
    }
}

/// Forget `step` only while this engine still runs exactly that attempt on that host generation,
/// so a late message never removes a newer attempt. True when it did.
fn forget(inner: &mut Inner, step: &str, attempt: u32, generation: u64) -> bool {
    let current = inner
        .running
        .get(step)
        .is_some_and(|running| running.dispatch.attempt == attempt && running.generation == generation);
    if current {
        inner.running.remove(step);
    }
    current
}

/// Queue one line for host `generation` only; a replaced host's messages never reach its successor.
fn send_to(inner: &Inner, generation: u64, message: &Value) {
    if inner.host.as_ref().is_some_and(|host| host.generation == generation) {
        send(inner, message);
    }
}

/// True only when no process has this pid (`ESRCH`); any other answer keeps the leader alive.
fn leader_gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let missing = unsafe { libc::kill(pid, 0) } == -1;
    missing && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Queue one line for the live host. A host that is gone drops it; its programs resume on the next.
fn send(inner: &Inner, message: &Value) {
    if let Some(host) = inner.host.as_ref() {
        let mut line = serde_json::to_vec(message).unwrap_or_default();
        line.push(b'\n');
        let _ = host.lines.send(line);
    }
}

fn write_host(mut stdin: ChildStdin, queue: mpsc::Receiver<Vec<u8>>) {
    for line in queue {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
            return;
        }
    }
}

/// A typed failure keeps its tag and fields, so `fractal missions show` says which error ended
/// the program.
fn failure_reason(error: &Value) -> String {
    let text = match error {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.chars().take(900).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// The JavaScript runtime the host runs on: `FRACTAL_PROGRAM_RUNTIME`, else `bun` on the daemon's
/// `PATH`.
fn runtime() -> Result<PathBuf> {
    if let Some(runtime) = std::env::var_os("FRACTAL_PROGRAM_RUNTIME") {
        return Ok(PathBuf::from(runtime));
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("bun"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| anyhow!("no `bun` on the daemon's PATH and no FRACTAL_PROGRAM_RUNTIME to run program steps"))
}

/// This boot's identity, as the host records it with every effect group.
fn boot_id() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim().to_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("/usr/sbin/sysctl").args(["-n", "kern.boottime"]).output()?;
        anyhow::ensure!(output.status.success(), "sysctl kern.boottime failed");
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

/// A live process's start time, read the way the host reads it (`program_host.mjs`).
fn start_time(pid: i32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat.get(stat.rfind(')')? + 2..)?.split(' ').nth(19).map(str::to_owned)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("/bin/ps").args(["-o", "lstart=", "-p", &pid.to_string()]).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(".{}.{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
