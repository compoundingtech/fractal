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
//! - host → daemon: `journal`, `run`, `ask`, `ending`, `done`, `unresponsive`, `cancelled`, `pong`.
//!
//! Every message after `start` names its step and attempt; both sides drop a message for an
//! attempt or host generation they no longer run. Supervision: the engine pings the host and
//! replaces one that does not answer within its deadline; a replaced or crashed host's programs
//! resume from their journals on the next host. Before it starts a host, the engine kills the
//! effect process groups an earlier host recorded and never ended (DQ2).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::model::{
    ClaimInput, ClaimRecord, FractalError, MissionRunView, PROGRAM_ASKER, PersonAskRequest, PersonStepResponse, ProgramSpec,
};
use crate::store::Store;
use crate::{MissionsStore, PersonWorkStore};

pub use crate::program_journal::{JOURNAL_KIND, RESULT_KEY};
/// The step states in which the engine runs a program step's current attempt. A step waiting on
/// a person keeps its program loaded (§11).
pub const RUNNING_STATES: [&str; 4] = ["ready", "working", "blocked", "waiting-person"];
/// A run credential has no expiry of its own: it binds only while its attempt is its step run's
/// current, live attempt, however long that attempt waits on an ask or a child run.
pub(super) const RUN_CREDENTIAL_LIFETIME_MS: u64 = u64::MAX;
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
    /// The step no longer runs this attempt (the reason says why); nothing was started.
    Left(String),
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
    /// By journal key: the child runs and asks the program waits on.
    requests: BTreeMap<String, Request>,
    /// Closed whenever this exact host attempt is forgotten, including cancellation.
    _trace: crate::otel::AttemptTrace,
}

/// A child run or person ask a program waits on (§6). The run's evaluation starts it and, once it
/// ends, journals its result under `key` and resolves it ([`ProgramEngine::resolve`]).
#[derive(Clone, Debug)]
pub struct Request {
    pub step: String,
    pub attempt: u32,
    pub key: String,
    pub wait: Wait,
    /// The child run or ask subject once it exists.
    pub child: Option<String>,
    /// The bundle hash of the program that asked, which journals the key's entries.
    program: String,
    seq: Value,
    generation: u64,
    watched_generation: Option<String>,
    since: Instant,
}

#[derive(Clone, Debug)]
pub enum Wait {
    /// A linked child run of `mission` with these inputs.
    Run { mission: String, input: BTreeMap<String, String> },
    /// A native person ask on the program step, asked by [`PROGRAM_ASKER`].
    Ask(Box<PersonAskRequest>),
    /// Resource operations always visit the host authority, including journal replay.
    Resource { operation: String, payload: Value, cancelled: bool },
}

impl Request {
    /// The idempotency key of the child run or ask: one per key and attempt, so a resumed attempt
    /// adopts it and a retried attempt starts a new one.
    pub fn idempotency_key(&self) -> String {
        idempotency_key(&self.step, self.attempt, &self.key)
    }
}

fn idempotency_key(step: &str, attempt: u32, key: &str) -> String {
    format!("program:{step}:{attempt}:{key}")
}

#[derive(Default)]
struct Inner {
    host: Option<Host>,
    generations: u64,
    pings: u64,
    supervised: bool,
    running: HashMap<String, Running>,
    /// Exact worker identities awaiting termination, independent of their recorded outcomes.
    terminating: HashSet<(u64, String, u32)>,
    /// By step: the attempt and the last host or liveness failure that stopped it.
    faults: HashMap<String, (u32, String)>,
    /// By step: the attempt this engine ended itself (its program finished or its worker was
    /// terminated) while it writes the step's new state ([`ProgramEngine::settle`]). No dispatch
    /// restarts that attempt in between.
    ended: HashMap<String, u32>,
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

    /// Start or resume `dispatch` unless the live host already runs that attempt or its step no
    /// longer runs it.
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
        if let Some(reason) = self.ended_or_left(&inner, &dispatch.step, dispatch.attempt)? {
            return Ok(Dispatched::Left(reason));
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
        let trace = self.start_trace(&dispatch)?;
        trace.correlation.environment(&mut env);
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
        let resource_context = self.store.resource_context_for_run(&dispatch.run)?;
        let resource_membership = resource_context.as_ref().and_then(|context| context.grant.as_ref());
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
            "resourceMembership": resource_membership,
        });
        inner.running.insert(dispatch.step.clone(), Running { dispatch, generation, requests: BTreeMap::new(), _trace: trace });
        send(&inner, &message);
        Ok(Dispatched::Started)
    }

    fn start_trace(&self, dispatch: &Dispatch) -> Result<crate::otel::AttemptTrace> {
        use opentelemetry::KeyValue;
        let existing = self.store.claims_for_subject_kind_at(&dispatch.run, "mission.run.trace", None, true, 1)?;
        let run_claim = if let Some(claim) = existing.claims.into_iter().next() {
            claim
        } else {
            let run_root = crate::otel::AttemptTrace::start("st.mission.run", vec![
                KeyValue::new("span.label", "mission run"),
                KeyValue::new("st.run", dispatch.run.clone()),
            ], &[]);
            self.store.append_claim(&ClaimInput {
                subject: dispatch.run.clone(), kind: "mission.run.trace".to_owned(),
                actor: None,
                fields: BTreeMap::from([("trace".to_owned(), serde_json::to_value(&run_root.correlation)?)]),
                evidence: Vec::new(), expected_subject: None,
                idempotency_key: Some(format!("mission-run-trace:{}", dispatch.run)),
            })?
        };
        let mut links = vec![serde_json::from_value(run_claim.body["fields"]["trace"].clone())?];
        let previous = self.store.claims_for_subject_kind_at(&dispatch.step, "program.attempt.trace", None, true, 1)?;
        if let Some(previous) = previous.claims.first() {
            links.push(serde_json::from_value(previous.body["fields"]["trace"].clone())?);
        } else if let Some(run) = self.store.mission_run(&dispatch.run)?
            && let Some(source) = run.retry_of.as_deref().or(run.supersedes.as_deref())
            && let Some(source) = self.store.mission_run(source)?
            && let Some(current) = self.store.step_run(&dispatch.step)?
            && let Some(prior) = source.steps.iter().find(|prior| prior.step == current.step)
            && let Some(previous) = self.store.claims_for_subject_kind_at(&prior.subject, "program.attempt.trace", None, true, 1)?.claims.first()
        {
            links.push(serde_json::from_value(previous.body["fields"]["trace"].clone())?);
        }
        let trace = crate::otel::AttemptTrace::start("st.program.attempt", vec![
            KeyValue::new("span.label", format!("attempt {}", dispatch.attempt)),
            KeyValue::new("st.run", dispatch.run.clone()),
            KeyValue::new("st.step", dispatch.step.clone()),
            KeyValue::new("st.attempt", i64::from(dispatch.attempt)),
        ], &links);
        self.store.append_claim(&ClaimInput {
            subject: dispatch.step.clone(), kind: "program.attempt.trace".to_owned(),
            actor: None,
            fields: BTreeMap::from([
                ("attempt".to_owned(), Value::from(dispatch.attempt)),
                ("trace".to_owned(), serde_json::to_value(&trace.correlation)?),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        })?;
        Ok(trace)
    }

    /// Why `attempt` must not start, read under the engine lock. A pass may hand over a view read
    /// before the attempt ended: one this engine is ending, whose step's new state is not written
    /// yet, or one its step left.
    fn ended_or_left(&self, inner: &Inner, step: &str, attempt: u32) -> Result<Option<String>> {
        if inner.ended.get(step) == Some(&attempt) {
            return Ok(Some(format!("the program of attempt {attempt} already ended")));
        }
        self.left(step, attempt)
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
    /// its run or replaced by a new generation) is stopped now, with its effects, and the child
    /// runs and asks it still waits on are cancelled.
    pub fn release_left(&self, run: &MissionRunView) -> Result<()> {
        let terminal = matches!(run.status.as_str(), "completed" | "failed" | "cancelled");
        let left = {
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
                    reason.map(|reason| (step.clone(), attempt, running.generation, reason))
                })
                .collect::<Vec<_>>();
            for (step, attempt, generation, reason) in &left {
                inner.running.remove(step);
                eprintln!("fractal: program {step} attempt {attempt} cancelled: {reason}");
                send_cancel(&mut inner, *generation, step, *attempt, reason);
            }
            left
        };
        for (step, attempt, _, reason) in left {
            self.cancel_waits(&step, attempt, &reason)?;
        }
        Ok(())
    }

    /// What the programs of `run` wait on, for its evaluation to start and resolve.
    pub fn requests(&self, run: &str) -> Vec<Request> {
        let inner = self.lock();
        inner
            .running
            .values()
            .filter(|running| running.dispatch.run == run)
            .flat_map(|running| running.requests.values().cloned())
            .collect()
    }

    /// The child run or ask `request` waits on now exists: journal it as the key's `child`, even
    /// when the program stopped waiting meanwhile, so nothing it started goes unrecorded. When its
    /// step has left the attempt by then, the step's cancellation (which may have read the journal
    /// before this entry) did not see it: cancel it here.
    pub fn waiting(&self, request: &Request, child: &str) -> Result<()> {
        self.write_journal(&request.step, request.attempt, &request.program, &request.key, "started", &Value::Null, Some(child))?;
        let running = self
            .program_of(request, |running| {
                if let Some(pending) = running.requests.get_mut(&request.key) {
                    pending.child = Some(child.to_owned());
                }
            })
            .is_some();
        // A program removed by its host's exit is resumed and adopts the child; one that ended or
        // left its attempt (each sets the step's state before it cancels) does not.
        if !running && let Some(reason) = self.left(&request.step, request.attempt)? {
            self.cancel_waits(&request.step, request.attempt, &reason)?;
        }
        Ok(())
    }

    /// The child run or ask `child` that `request` waited on ended with `exit` (`{_tag, value}` or
    /// `{_tag, error}`): journal it under the key, then hand it to the program (`resolved`). A key
    /// that already holds a result keeps it, and the program gets that one.
    pub fn resolve(&self, request: &Request, child: &str, exit: Value) -> Result<()> {
        let Some(program) = self.program_of(request, |_| {}) else { return Ok(()) };
        let (status, mut result) = match exit["_tag"].as_str() {
            Some("Success") => ("completed", json!({"value": exit["value"]})),
            _ => ("failed", json!({"error": exit["error"]})),
        };
        result["ms"] = Value::from(u64::try_from(request.since.elapsed().as_millis()).unwrap_or(u64::MAX));
        let exit = match self.write_journal(&request.step, request.attempt, &program, &request.key, status, &result, Some(child)) {
            Ok(_) => exit,
            Err(error) => {
                let mut journaled = self
                    .journal(&request.step, request.attempt)?
                    .into_iter()
                    .find(|entry| entry["key"] == request.key.as_str())
                    .ok_or_else(|| anyhow!("journaling `{}` of {}: {error}", request.key, request.step))?;
                if let Some(entry) = journaled.as_object_mut() {
                    entry.remove("key");
                }
                journaled
            }
        };
        let mut inner = self.lock();
        let Some((generation, pending)) = inner
            .running
            .get_mut(&request.step)
            .filter(|running| running.dispatch.attempt == request.attempt)
            .and_then(|running| Some((running.generation, running.requests.remove(&request.key)?)))
        else {
            return Ok(());
        };
        send_to(
            &inner,
            generation,
            &json!({"t": "resolved", "step": request.step, "attempt": request.attempt, "seq": pending.seq, "key": request.key, "exit": exit}),
        );
        Ok(())
    }

    /// Resolve a live host capability operation without replaying a journaled grant. The
    /// resource module owns durable request identity and generation reattachment.
    pub fn resolve_resource(&self, request: &Request, exit: Value) -> Result<()> {
        let mut inner = self.lock();
        let Some(running) = inner.running.get_mut(&request.step)
            .filter(|running| running.dispatch.attempt == request.attempt && running.generation == request.generation)
        else { return Ok(()) };
        let Some(pending) = running.requests.get(&request.key) else { return Ok(()) };
        if pending.seq != request.seq { return Ok(()) }
        // Cancellation can replace the queued operation while its admission is in flight.
        if matches!((&request.wait, &pending.wait),
            (Wait::Resource { cancelled: false, .. }, Wait::Resource { cancelled: true, .. }))
        {
            return Ok(());
        }
        let generation = running.generation;
        let pending = running.requests.remove(&request.key).expect("request checked above");
        send_to(&inner, generation, &json!({
            "t": "resolved", "step": request.step, "attempt": request.attempt,
            "seq": pending.seq, "key": request.key, "exit": exit,
        }));
        Ok(())
    }

    /// Register an OS completion waiter once per live request/generation, before inspecting its
    /// completion again. A resumed worker gets a fresh subscription to the same generation.
    pub fn watch_resource_once(&self, request: &Request, generation: &str, start: impl FnOnce() -> Result<()>) -> Result<()> {
        let mut inner = self.lock();
        let Some(pending) = inner.running.get_mut(&request.step)
            .filter(|running| running.dispatch.attempt == request.attempt && running.generation == request.generation)
            .and_then(|running| running.requests.get_mut(&request.key))
        else { return Ok(()) };
        if pending.watched_generation.as_deref() == Some(generation) { return Ok(()) }
        start()?;
        pending.watched_generation = Some(generation.to_owned());
        Ok(())
    }

    pub fn resource_owner_live(&self, step: &str, attempt: u32) -> bool {
        let inner = self.lock();
        inner.running.get(step).is_some_and(|running| running.dispatch.attempt == attempt
            && inner.host.as_ref().is_some_and(|host| host.generation == running.generation))
    }

    /// The program asked for something the daemon refuses (a defect of the bundle): its step fails
    /// and the program stops, with what it already waits on.
    pub fn fail(&self, step: &str, attempt: u32, reason: &str) -> Result<()> {
        // Held as ended until its failure is written: a pass with a view read before then must
        // not start the attempt again. Register its termination under the same mutex so a
        // concurrent replacement cannot observe a forgotten attempt without its fence.
        {
            let mut inner = self.lock();
            let generation = inner.running.get(step).filter(|running| running.dispatch.attempt == attempt)
                .map(|running| {
                    running._trace.failed();
                    running.generation
                });
            if let Some(generation) = generation
                && forget(&mut inner, step, attempt, generation)
            {
                inner.ended.insert(step.to_owned(), attempt);
                send_cancel(&mut inner, generation, step, attempt, reason);
            }
        }
        eprintln!("fractal: program {step} attempt {attempt} failed: {reason}");
        // The step's state first: a child started concurrently then sees it left (`waiting`).
        self.settle(step, attempt, || self.store.set_step_state_unless_terminal(step, "failed", Some(reason)))?;
        self.cancel_waits(step, attempt, reason)?;
        (self.wake)(step);
        Ok(())
    }

    /// The bundle hash of the program that made `request`, while this engine still runs that
    /// attempt, after `update` saw its entry.
    fn program_of(&self, request: &Request, update: impl FnOnce(&mut Running)) -> Option<String> {
        let mut inner = self.lock();
        let running = inner.running.get_mut(&request.step).filter(|running| running.dispatch.attempt == request.attempt)?;
        update(running);
        Some(running.dispatch.program.bundle_hash.clone())
    }

    /// Cancel the child runs and asks this attempt journaled as waits and has no result for, once
    /// nothing waits on them: the program ended or its step left the attempt. A run that ends
    /// cancels its children itself, and its asks end with it.
    fn cancel_waits(&self, step: &str, attempt: u32, reason: &str) -> Result<()> {
        let reason = format!("its program step {step} (attempt {attempt}) no longer waits on it: {reason}");
        for child in self.open_waits(step, attempt)? {
            if child.starts_with("mission-run/") {
                let child = self.store.mission_run_supersession_successor(&child)?;
                if !matches!(self.store.mission_run_status(&child)?.as_deref(), None | Some("completed" | "failed" | "cancelled")) {
                    self.store.request_mission_run_cancellation(&child, &reason)?;
                }
                continue;
            }
            if !self.store.step_run(&child)?.is_some_and(|ask| matches!(ask.status.as_str(), "ready" | "pending")) {
                continue;
            }
            let cancelled = self.store.finish_person_step(
                &PersonStepResponse {
                    delegation: None,
                    subject: child.clone(),
                    actor: PROGRAM_ASKER.to_owned(),
                    summary: reason.clone(),
                    evidence: Vec::new(),
                    episode: None,
                    answer: None,
                    idempotency_key: format!("program-cancel:{child}"),
                },
                true,
            );
            match cancelled {
                // `stale-fence`: the ask no longer waits (answered, or its run or generation ended).
                Err(error) if error.code != "stale-fence" => return Err(error.into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// The child runs and asks this attempt journaled under a key that has no result yet.
    fn open_waits(&self, step: &str, attempt: u32) -> Result<Vec<String>> {
        let mut waits = BTreeMap::new();
        for claim in self.journal_claims(step)? {
            let fields = &claim.body["fields"];
            let Some(key) = fields["key"].as_str().filter(|_| fields["attempt"].as_u64() == Some(u64::from(attempt))) else {
                continue;
            };
            match (fields["status"].as_str(), fields["child"].as_str()) {
                (Some("started"), Some(child)) => {
                    waits.insert(key.to_owned(), child.to_owned());
                }
                (Some("completed" | "failed"), _) => {
                    waits.remove(key);
                }
                _ => {}
            }
        }
        Ok(waits.into_values().collect())
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
        let mut entries = Vec::new();
        for claim in self.journal_claims(step)? {
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

    /// Every journal entry of `step`, in store order.
    fn journal_claims(&self, step: &str) -> Result<Vec<ClaimRecord>> {
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
        Ok(claims)
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

    /// Fence only this run's execution scope before admitting its replacement.
    /// An outcome is not proof of worker termination: wait for the host's exact acknowledgement.
    pub fn fence_left_effects(&self, run: &MissionRunView) -> Result<bool> {
        smallclaims::touched::note_read(|| "program-effects".to_owned());
        // Derive canonical Store ownership before acquiring the engine mutex.
        let scope = self.store.mission_run_supersession_step_scope(run)?;
        if self.lock().terminating.iter().any(|(_, step, _)| scope.contains(step)) { return Ok(false); }
        let dir = self.groups_dir();
        std::fs::create_dir_all(&dir)?;
        let boot = boot_id()?;
        for entry in std::fs::read_dir(dir)? {
            let record: Value = serde_json::from_slice(&std::fs::read(entry?.path())?)?;
            let step = record["step"].as_str().ok_or_else(|| anyhow!("effect group has no step identity"))?;
            if !scope.contains(step) { continue; }
            let attempt = record["attempt"].as_u64().and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| anyhow!("effect group has no attempt identity"))?;
            let recorded_boot = record["boot_id"].as_str().ok_or_else(|| anyhow!("effect group has no boot identity"))?;
            if self.left(step, attempt)?.is_none() || recorded_boot != boot {
                continue;
            }
            let pid = record["pid"].as_i64().and_then(|value| i32::try_from(value).ok())
                .filter(|pid| *pid > 1).ok_or_else(|| anyhow!("effect group has no process identity"))?;
            if !leader_gone(pid) {
                let observed = start_time(pid).ok_or_else(|| anyhow!("cannot confirm effect group {pid} identity"))?;
                let recorded = record["start_time"].as_str().ok_or_else(|| anyhow!("effect group {pid} has no start identity"))?;
                if observed != recorded { continue; }
            }
            // SIGKILL is an execution fence, not merely desired cancellation.
            // ESRCH confirms that this exact group has already ended.
            // SAFETY: signalling a verified process group has no memory effects.
            if unsafe { libc::kill(-pid, libc::SIGKILL) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error.into());
                }
            }
        }
        Ok(true)
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
        let attempt = message["attempt"].as_u64().and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| anyhow!("message without a valid attempt"))?;
        if message["t"] == "ending" {
            let mut inner = self.lock();
            if inner.running.get(step).is_some_and(|running|
                running.dispatch.attempt == attempt && running.generation == generation)
            {
                inner.terminating.insert((generation, step.to_owned(), attempt));
            }
            return Ok(());
        }
        if message["t"] == "cancelled" {
            let removed = self.lock().terminating.remove(&(generation, step.to_owned(), attempt));
            if removed { (self.wake)("program-effects"); }
            return Ok(());
        }
        let program = {
            let inner = self.lock();
            match inner.running.get(step) {
                // A message from an attempt or host this engine no longer runs changes nothing.
                Some(running) if running.dispatch.attempt == attempt && running.generation == generation => {
                    running.dispatch.program.bundle_hash.clone()
                }
                _ => return Ok(()),
            }
        };
        match message["t"].as_str() {
            Some("journal") => self.journal_entry(step, attempt, generation, &program, message),
            Some("run" | "ask" | "resource" | "resource-cancel") => self.request(step, attempt, generation, &program, message),
            Some("done") => self.done(step, attempt, generation, &program, message),
            Some("unresponsive") => self.unresponsive(step, attempt, generation, message),
            other => Err(anyhow!("unknown message type {other:?}")),
        }
    }

    /// A program waits on a child run or a person ask (§6). Registered here, it is started and
    /// resolved by its run's evaluation. The `started` entry, written before the request is
    /// registered, is the change that evaluates the run; a request the daemon cannot accept fails
    /// the step.
    fn request(&self, step: &str, attempt: u32, generation: u64, program: &str, message: &Value) -> Result<()> {
        if self.stop_if_left(step, attempt, generation)? {
            return Ok(());
        }
        let Some(key) = message["key"].as_str().filter(|key| !key.is_empty() && *key != RESULT_KEY) else {
            return self.fail(step, attempt, &format!("the program made a request without a key or with the reserved key {RESULT_KEY}"));
        };
        let wait = match wait(step, attempt, key, message) {
            Ok(wait) => wait,
            Err(reason) => return self.fail(step, attempt, &format!("the program made an invalid request for `{key}`: {reason}")),
        };
        if !matches!(wait, Wait::Resource { .. }) {
            self.write_journal(step, attempt, program, key, "started", &Value::Null, None)?;
        }
        {
            let mut inner = self.lock();
            let Some(running) = inner
                .running
                .get_mut(step)
                .filter(|running| running.dispatch.attempt == attempt && running.generation == generation)
            else {
                return Ok(());
            };
            let request = Request {
                step: step.to_owned(),
                attempt,
                key: key.to_owned(),
                wait,
                child: None,
                program: program.to_owned(),
                seq: message["seq"].clone(),
                generation,
                watched_generation: None,
                since: Instant::now(),
            };
            running.requests.insert(key.to_owned(), request);
        }
        (self.wake)(step);
        Ok(())
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
        {
            let mut inner = self.lock();
            if forget(&mut inner, step, attempt, generation) {
                send_cancel(&mut inner, generation, step, attempt, &reason);
            }
        }
        self.cancel_waits(step, attempt, &reason)?;
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
        let traces = self.store.claims_for_subject_kind_at(step, "program.attempt.trace", None, true, 1)
            .map_err(smallclaims::error::typed)?;
        let evidence = traces.claims.first()
            .filter(|claim| claim.body["fields"]["attempt"].as_u64() == Some(u64::from(attempt)))
            .map(|trace| vec![trace.id.clone()])
            .unwrap_or_default();
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
            evidence,
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
        let Some(trace) = self.end(step, attempt, generation, None, message["exit"]["_tag"] != "Success") else {
            return Ok(());
        };
        let exit = &message["exit"];
        let written = self.settle(step, attempt, || {
            let (status, reason) = match exit["_tag"].as_str() {
                Some("Success") => match self.write_journal(step, attempt, program, RESULT_KEY, "completed", &json!({"value": exit["value"]}), None) {
                    Ok(_) => ("completed", None),
                    Err(error) => {
                        trace.failed();
                        ("failed", Some(format!("the program's result could not be journaled: {error}")))
                    }
                },
                _ => ("failed", Some(failure_reason(&exit["error"]))),
            };
            self.store.set_step_state_unless_terminal(step, status, reason.as_deref())?;
            Ok(status)
        })?;
        let Some(status) = written else {
            if let Some(reason) = self.left(step, attempt)? {
                self.cancel_waits(step, attempt, &reason)?;
            }
            return Ok(());
        };
        // A program can end while a child run or ask it started is still open (a lost race):
        // nothing waits on it any more. The step's state comes first, so a child started
        // concurrently sees it left (`waiting`).
        self.cancel_waits(step, attempt, "the program ended")?;
        eprintln!("fractal: program {step} {status}");
        (self.wake)(step);
        Ok(())
    }

    /// The host terminated this attempt's worker after it missed its liveness deadline (DQ8): the
    /// step fails once instead of looping across restarts.
    fn unresponsive(&self, step: &str, attempt: u32, generation: u64, message: &Value) -> Result<()> {
        let ms = message["ms"].as_u64().unwrap_or_default();
        let reason = format!("the program's worker missed its {ms} ms liveness deadline and was terminated");
        let Some(_trace) = self.end(step, attempt, generation, Some(&reason), true) else {
            return Ok(());
        };
        eprintln!("fractal: program {step} attempt {attempt}: {reason}");
        self.settle(step, attempt, || self.store.set_step_state_unless_terminal(step, "failed", Some(&reason)))?;
        self.cancel_waits(step, attempt, &reason)?;
        (self.wake)(step);
        Ok(())
    }

    /// Forget exactly this host attempt, returning its span owner so terminal journal
    /// writes can still mark it failed before it closes.
    fn end(&self, step: &str, attempt: u32, generation: u64, fault: Option<&str>, failed: bool) -> Option<crate::otel::AttemptTrace> {
        let mut inner = self.lock();
        inner.running.get(step)
            .filter(|running| running.dispatch.attempt == attempt && running.generation == generation)?;
        let running = inner.running.remove(step)?;
        if failed {
            running._trace.failed();
        }
        if let Some(fault) = fault {
            inner.faults.insert(step.to_owned(), (attempt, fault.to_owned()));
        }
        inner.ended.insert(step.to_owned(), attempt);
        Some(running._trace)
    }

    /// Write the new state of a step whose `attempt` this engine just ended, unless the step
    /// already left it; what `write` returned, or `None` when the step had left. Then the attempt
    /// is no longer held as ended: the store says it ended, or, when the read or write failed, the
    /// next pass resumes it as before.
    fn settle<T>(&self, step: &str, attempt: u32, write: impl FnOnce() -> Result<T>) -> Result<Option<T>> {
        let settled = match self.left(step, attempt) {
            Ok(Some(_)) => Ok(None),
            Ok(None) => write().map(Some),
            Err(error) => Err(error),
        };
        let mut inner = self.lock();
        if inner.ended.get(step) == Some(&attempt) {
            inner.ended.remove(step);
        }
        settled
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
        let (resume, released) = {
            let mut inner = self.lock();
            if inner.host.as_ref().is_some_and(|host| host.generation == generation)
                && let Some(mut host) = inner.host.take()
            {
                let _ = host.child.kill();
                match host.child.wait() {
                    Ok(status) => eprintln!("fractal: program host {generation} exited ({status:?})"),
                    Err(error) => {
                        eprintln!("fractal: cannot reap program host {generation}: {error}");
                        inner.host = Some(host);
                        return;
                    }
                }
            }
            let terminating = inner.terminating.len();
            inner.terminating.retain(|(host, _, _)| *host != generation);
            let released = inner.terminating.len() != terminating;
            let gone = inner
                .running
                .iter()
                .filter(|(_, running)| running.generation == generation)
                .map(|(step, _)| step.clone())
                .collect::<Vec<_>>();
            let resume = gone.iter().filter_map(|step| inner.running.remove(step)).map(|running| running.dispatch).collect::<Vec<_>>();
            (resume, released)
        };
        if released { (self.wake)("program-effects"); }
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
                Ok(Dispatched::Running | Dispatched::Started | Dispatched::Left(_)) => continue,
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

fn send_cancel(inner: &mut Inner, generation: u64, step: &str, attempt: u32, reason: &str) {
    if inner.host.as_ref().is_some_and(|host| host.generation == generation) {
        inner.terminating.insert((generation, step.to_owned(), attempt));
        send_to(inner, generation, &json!({"t": "cancel", "step": step, "attempt": attempt, "reason": reason}));
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

/// What a `run` or `ask` message asks the daemon to wait on, or why it cannot.
fn wait(step: &str, attempt: u32, key: &str, message: &Value) -> std::result::Result<Wait, String> {
    let text = |name: &str| {
        message[name]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("`{name}` must be a non-empty string"))
    };
    if matches!(message["t"].as_str(), Some("resource" | "resource-cancel")) {
        let operation = text("operation")?;
        if !matches!(operation.as_str(),
            "resource.lane.acquire" | "resource.lane.release" | "resource.lane.exec" | "resource.lane.run")
        {
            return Err(format!("unknown resource operation `{operation}`"));
        }
        if !message["payload"].is_object() {
            return Err("resource payload must be an object".to_owned());
        }
        return Ok(Wait::Resource {
            operation,
            payload: message["payload"].clone(),
            cancelled: message["t"] == "resource-cancel",
        });
    }
    if message["t"] == "run" {
        let mission = text("mission")?;
        let input = serde_json::from_value::<BTreeMap<String, String>>(message["input"].clone())
            .map_err(|_| "`input` must map each mission input to a string".to_owned())?;
        let mission = mission.strip_prefix("mission/").unwrap_or(&mission).to_owned();
        return Ok(Wait::Run { mission, input });
    }
    let deadline_ms = match &message["deadline"] {
        Value::Null => None,
        Value::String(deadline) => Some(crate::declaration_syntax::parse_duration(deadline, true).map_err(|error| error.message)?),
        _ => return Err("`deadline` must be a duration such as `10m`".into()),
    };
    let default_answer = match &message["default"] {
        Value::Null => None,
        Value::String(answer) => Some(answer.clone()),
        _ => return Err("`default` must name an answer of the request".into()),
    };
    let title = text("title")?;
    Ok(Wait::Ask(Box::new(PersonAskRequest {
        priority: None,
        legacy_request: None,
        person: text("for")?,
        reason: title.clone(),
        title,
        actor: PROGRAM_ASKER.to_owned(),
        step: Some(step.to_owned()),
        new_run: None,
        // A program step has no claim: its attempt fences the ask.
        incarnation: Some(attempt.to_string()),
        request: Some(message["request"].clone()).filter(|request| !request.is_null()),
        deadline_ms,
        default_answer,
        idempotency_key: idempotency_key(step, attempt, key),
    })))
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
