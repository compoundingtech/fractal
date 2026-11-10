//! The program engine: runs `program` steps (docs/st3/program-engine.md).
//!
//! The daemon owns everything durable about a program run: which step runs, its journal (one
//! `program.step` claim per inner step), replay after a restart, and the step's outcome. Program
//! bodies are evaluated by one supervised host process per daemon (`program_host.mjs`, on Bun),
//! which talks to this module over its stdin and stdout only. That pipe is the host's only write
//! path, so a host orphaned by a dead daemon can never journal anything.
//!
//! Protocol (one JSON object per line; every message after `start` names its step and attempt, and
//! a message for another attempt than the one running is dropped):
//! - daemon → host: `start` {step, attempt, module, sha256, payload, env, cwd, journal},
//!   `ack` {step, attempt, seq} once a completed or failed entry is durable, `nack` {step, attempt,
//!   seq, error} when it could not be written, `cancel` {step, attempt, reason}.
//! - host → daemon: `journal` {step, attempt, seq, key, status, value?, error?, ms?} and
//!   `done` {step, attempt, exit: {_tag: "Success", value} | {_tag: "Failure", error}}.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::MissionsStore;
use crate::model::{ClaimInput, ProgramSpec};
use crate::store::Store;

/// The claim kind of a program journal entry, on the program's `step-run` subject.
pub const JOURNAL_KIND: &str = "program.step";
const HOST_SOURCE: &str = include_str!("program_host.mjs");
/// How long a crashed host waits before its replacement starts, so a host that cannot start does
/// not spin.
const HOST_RESTART_DELAY: Duration = Duration::from_millis(500);
const JOURNAL_PAGE: usize = 500;
/// The step states in which the engine runs a program step's current attempt.
const RUNNING_STATES: [&str; 3] = ["ready", "working", "blocked"];

/// One program step this daemon runs.
#[derive(Clone, Debug)]
pub struct Dispatch {
    pub step: String,
    pub attempt: u32,
    pub program: ProgramSpec,
    pub payload: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub cwd: String,
}

pub enum Dispatched {
    /// The host already runs this attempt.
    Running,
    /// The host was asked to start (or resume) it.
    Started,
    /// The step cannot run; the reason fails it.
    Refused(String),
}

struct Host {
    child: Child,
    /// Lines for the host's stdin. A writer thread owns the pipe, so neither the reconciler nor the
    /// reader thread ever blocks on a host that stopped reading.
    lines: mpsc::Sender<Vec<u8>>,
    generation: u64,
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
    running: HashMap<String, Running>,
}

pub struct ProgramEngine {
    store: Arc<Store>,
    wake: Arc<dyn Fn() + Send + Sync>,
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl ProgramEngine {
    pub fn new(store: Arc<Store>, wake: Arc<dyn Fn() + Send + Sync>, dir: PathBuf) -> Arc<Self> {
        Arc::new(Self { store, wake, dir, inner: Mutex::default() })
    }

    /// Start or resume `dispatch` unless the live host already runs that attempt.
    pub fn dispatch(self: &Arc<Self>, dispatch: Dispatch) -> Result<Dispatched> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let live = inner.host.as_ref().map(|host| host.generation);
        if inner
            .running
            .get(&dispatch.step)
            .is_some_and(|running| running.dispatch.attempt == dispatch.attempt && Some(running.generation) == live)
        {
            return Ok(Dispatched::Running);
        }
        let module = match self.pinned_module(&dispatch.program) {
            Ok(module) => module,
            Err(reason) => return Ok(Dispatched::Refused(reason)),
        };
        let journal = self.journal(&dispatch.step, dispatch.attempt)?;
        let message = json!({
            "t": "start",
            "step": dispatch.step,
            "attempt": dispatch.attempt,
            "module": module,
            "sha256": dispatch.program.sha256,
            "payload": dispatch.payload,
            "env": dispatch.env,
            "cwd": dispatch.cwd,
            "journal": journal,
        });
        let generation = self.ensure_host(&mut inner)?;
        eprintln!(
            "fractal: program {} attempt {} started with {} journaled steps",
            dispatch.step,
            dispatch.attempt,
            journal.len()
        );
        inner.running.insert(dispatch.step.clone(), Running { dispatch, generation });
        send(&inner, &message);
        Ok(Dispatched::Started)
    }

    /// The bundle as this revision pinned it. The first run of a revision copies the verified
    /// bundle into a content-addressed store, so later runs and resumes read those exact bytes even
    /// after the source path changes; a source that no longer matches is never run.
    fn pinned_module(&self, program: &ProgramSpec) -> std::result::Result<PathBuf, String> {
        let pinned = self.dir.join("bundles").join(format!("{}.mjs", program.sha256));
        if pinned.is_file() {
            return Ok(pinned);
        }
        let bytes = std::fs::read(&program.bundle)
            .map_err(|error| format!("program bundle `{}` is unreadable: {error}", program.bundle))?;
        let actual = hex::encode(sha2::Sha256::digest(&bytes));
        if actual != program.sha256 {
            return Err(format!(
                "program bundle `{}` has sha256 {actual}, but this mission revision pins {}",
                program.bundle, program.sha256
            ));
        }
        write_atomically(&pinned, &bytes).map_err(|error| format!("pinning the program bundle: {error:#}"))?;
        Ok(pinned)
    }

    /// The entries replay returns: the completed steps and typed failures of this attempt. A
    /// retried attempt starts from the top, because an earlier attempt's compensation may have
    /// undone what its completed steps did.
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
            if fields["attempt"].as_u64() != Some(u64::from(attempt)) {
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

    /// The live host's generation, starting a host when there is none or the last one exited.
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
        let script = self.dir.join(format!("host-{}.mjs", &hex::encode(sha2::Sha256::digest(HOST_SOURCE))[..12]));
        if !script.is_file() {
            write_atomically(&script, HOST_SOURCE.as_bytes())?;
        }
        let runtime = bun()?;
        let mut child = Command::new(&runtime)
            .arg(&script)
            .current_dir(&self.dir)
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
        inner.host = Some(Host { child, lines, generation });
        Ok(generation)
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
        let step = message["step"].as_str().ok_or_else(|| anyhow!("message without a step"))?;
        let attempt = message["attempt"].as_u64().ok_or_else(|| anyhow!("message without an attempt"))?;
        let program = {
            let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            match inner.running.get(step) {
                // A message from an attempt or host this engine no longer runs changes nothing.
                Some(running) if u64::from(running.dispatch.attempt) == attempt && running.generation == generation => {
                    running.dispatch.program.sha256.clone()
                }
                _ => return Ok(()),
            }
        };
        let attempt = u32::try_from(attempt)?;
        match message["t"].as_str() {
            Some("journal") => self.journal_entry(step, attempt, &program, message),
            Some("done") => self.done(step, attempt, message),
            other => Err(anyhow!("unknown message type {other:?}")),
        }
    }

    /// Stop the program if its step no longer runs this attempt (ended, cancelled, timed out or
    /// retried). True when it was stopped.
    fn stop_if_left(&self, step: &str, attempt: u32) -> Result<bool> {
        let left = match self.store.step_run(step)? {
            Some(view) => {
                (view.attempt != attempt || !RUNNING_STATES.contains(&view.status.as_str())).then(|| {
                    format!("the step is {} at attempt {}", view.status, view.attempt)
                })
            }
            None => Some("the step run no longer exists".to_owned()),
        };
        let Some(reason) = left else { return Ok(false) };
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.running.remove(step);
        send(&inner, &json!({"t": "cancel", "step": step, "attempt": attempt, "reason": reason}));
        Ok(true)
    }

    fn journal_entry(&self, step: &str, attempt: u32, program: &str, message: &Value) -> Result<()> {
        if self.stop_if_left(step, attempt)? {
            return Ok(());
        }
        let key = message["key"].as_str().ok_or_else(|| anyhow!("journal entry without a key"))?;
        let status = message["status"].as_str().ok_or_else(|| anyhow!("journal entry without a status"))?;
        let mut fields = BTreeMap::from([
            ("key".to_owned(), Value::from(key)),
            ("status".to_owned(), Value::from(status)),
            ("attempt".to_owned(), Value::from(attempt)),
            ("program".to_owned(), Value::from(program)),
        ]);
        for name in ["value", "error", "ms"] {
            if let Some(value) = message.get(name).filter(|value| !value.is_null()) {
                fields.insert(name.to_owned(), value.clone());
            }
        }
        // One result per key and attempt. The key fences a double execution: a second, different
        // result for one key is refused.
        let idempotency_key = (status != "started").then(|| format!("program:{step}:{attempt}:{key}:{status}"));
        let written = self.store.append_claim(&ClaimInput {
            subject: step.to_owned(),
            kind: JOURNAL_KIND.to_owned(),
            actor: None,
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key,
        });
        if status == "started" {
            return written.map(|_| ()).map_err(Into::into);
        }
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = &message["seq"];
        match written {
            Ok(_) => send(&inner, &json!({"t": "ack", "step": step, "attempt": attempt, "seq": seq})),
            // The program must not continue past a result that is not durable: the step fails.
            Err(error) => send(
                &inner,
                &json!({"t": "nack", "step": step, "attempt": attempt, "seq": seq, "error": format!("journal write refused: {error}")}),
            ),
        }
        Ok(())
    }

    fn done(&self, step: &str, attempt: u32, message: &Value) -> Result<()> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).running.remove(step);
        if self.stop_if_left(step, attempt)? {
            return Ok(());
        }
        let exit = &message["exit"];
        let (status, reason) = match exit["_tag"].as_str() {
            Some("Success") => ("completed", None),
            _ => ("failed", Some(failure_reason(&exit["error"]))),
        };
        self.store.set_step_state_unless_terminal(step, status, reason.as_deref())?;
        eprintln!("fractal: program {step} {status}");
        (self.wake)();
        Ok(())
    }

    /// A host that exits takes its programs with it; a new host resumes each from its journal.
    fn host_exited(self: Arc<Self>, generation: u64) {
        let resume = {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            if inner.host.as_ref().is_some_and(|host| host.generation == generation)
                && let Some(mut host) = inner.host.take()
            {
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
            let step = dispatch.step.clone();
            match self.dispatch(dispatch) {
                Ok(Dispatched::Refused(reason)) => {
                    if let Err(error) = self.store.set_step_state_unless_terminal(&step, "failed", Some(&reason)) {
                        eprintln!("fractal: failing program {step}: {error:#}");
                    }
                    (self.wake)();
                }
                Ok(_) => {}
                Err(error) => eprintln!("fractal: resuming program {step}: {error:#}"),
            }
        }
    }
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

/// The JavaScript runtime the host runs on: `FRACTAL_PROGRAM_RUNTIME`, else `bun` on the daemon's
/// `PATH`.
fn bun() -> Result<PathBuf> {
    if let Some(runtime) = std::env::var_os("FRACTAL_PROGRAM_RUNTIME") {
        return Ok(PathBuf::from(runtime));
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("bun"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| anyhow!("no `bun` on the daemon's PATH to run program steps"))
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
mod tests {
    use super::*;

    fn engine(dir: &Path) -> Arc<ProgramEngine> {
        ProgramEngine::new(Arc::new(Store::open_memory("node").unwrap()), Arc::new(|| {}), dir.to_owned())
    }

    fn entry(engine: &ProgramEngine, step: &str, key: &str, status: &str, attempt: u32, value: Value) {
        let mut fields = BTreeMap::from([
            ("key".to_owned(), Value::from(key)),
            ("status".to_owned(), Value::from(status)),
            ("attempt".to_owned(), Value::from(attempt)),
        ]);
        fields.insert(if status == "failed" { "error" } else { "value" }.to_owned(), value);
        engine
            .store
            .append_claim(&ClaimInput {
                subject: step.to_owned(),
                kind: JOURNAL_KIND.to_owned(),
                actor: None,
                fields,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    #[test]
    fn replay_returns_this_attempts_results_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let step = "step-run/generation/program";
        entry(&engine, step, "merge", "completed", 1, json!({"commit": "c1"}));
        entry(&engine, step, "publish-local", "started", 1, Value::Null);
        entry(&engine, step, "publish-local", "failed", 1, json!({"_tag": "StepFailed", "step": "publish"}));
        entry(&engine, step, "revert", "completed", 1, json!({"reverted": ["c1"]}));
        entry(&engine, step, "merge", "completed", 2, json!({"commit": "c2"}));
        assert_eq!(
            engine.journal(step, 1).unwrap(),
            vec![
                json!({"key": "merge", "_tag": "Success", "value": {"commit": "c1"}}),
                json!({"key": "publish-local", "_tag": "Failure", "error": {"_tag": "StepFailed", "step": "publish"}}),
                json!({"key": "revert", "_tag": "Success", "value": {"reverted": ["c1"]}}),
            ]
        );
        // A retried attempt never replays the first attempt, whose revert undid its merge.
        assert_eq!(engine.journal(step, 2).unwrap(), vec![json!({"key": "merge", "_tag": "Success", "value": {"commit": "c2"}})]);
    }

    #[test]
    fn replay_pages_through_long_journals_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let step = "step-run/generation/program";
        for n in 1..=(JOURNAL_PAGE + 3) {
            entry(&engine, step, &format!("poll#{n}"), "completed", 1, Value::from(n));
        }
        let journal = engine.journal(step, 1).unwrap();
        assert_eq!(journal.len(), JOURNAL_PAGE + 3);
        assert!(journal.iter().enumerate().all(|(index, entry)| entry["value"] == Value::from(index + 1)));
    }

    #[test]
    fn a_revision_runs_the_bytes_it_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let bundle = dir.path().join("land.mjs");
        std::fs::write(&bundle, b"export default 1").unwrap();
        let sha256 = hex::encode(sha2::Sha256::digest(b"export default 1"));
        let program = ProgramSpec { bundle: bundle.to_string_lossy().into_owned(), sha256 };
        let pinned = engine.pinned_module(&program).unwrap();
        // The source changes after the first run pinned it: this revision keeps its own bytes.
        std::fs::write(&bundle, b"export default 2").unwrap();
        assert_eq!(engine.pinned_module(&program).unwrap(), pinned);
        assert_eq!(std::fs::read(&pinned).unwrap(), b"export default 1");
        // A revision whose bytes were never pinned and no longer match is refused.
        let other = ProgramSpec { sha256: "0".repeat(64), ..program };
        assert!(engine.pinned_module(&other).unwrap_err().contains("this mission revision pins"));
    }
}
