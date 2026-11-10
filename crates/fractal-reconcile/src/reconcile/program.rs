//! The program engine: runs `program` steps (docs/st3/program-engine.md).
//!
//! The daemon owns everything durable about a program run: which step runs, its journal (one
//! `program.step` claim per inner step), replay after a restart, and the step's outcome. Program
//! bodies are evaluated by one supervised host process per daemon (`program_host.mjs`, on Bun),
//! which talks to this module over its stdin and stdout only. That pipe is the host's only write
//! path, so a host orphaned by a dead daemon can never journal anything.
//!
//! Protocol (one JSON object per line):
//! - daemon → host: `start` {step, attempt, module, sha256, payload, env, cwd, journal},
//!   `ack` {step, seq} once a completed or failed entry is durable, `cancel` {step, reason}.
//! - host → daemon: `journal` {step, seq, key, status, value?, error?, ms?} and
//!   `done` {step, exit: {_tag: "Success", value} | {_tag: "Failure", error}}.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
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
    stdin: ChildStdin,
    generation: u64,
}

#[derive(Default)]
struct Inner {
    host: Option<Host>,
    generations: u64,
    running: HashMap<String, Dispatch>,
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

    /// Start or resume `dispatch` unless the host already runs that attempt.
    pub fn dispatch(self: &Arc<Self>, dispatch: Dispatch) -> Result<Dispatched> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.running.get(&dispatch.step).is_some_and(|running| running.attempt == dispatch.attempt) {
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
        self.ensure_host(&mut inner)?;
        eprintln!(
            "fractal: program {} attempt {} started with {} journaled steps",
            dispatch.step,
            dispatch.attempt,
            journal.len()
        );
        inner.running.insert(dispatch.step.clone(), dispatch);
        send(&mut inner, &message)?;
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

    /// The entries replay returns: every completed step, and the failures of this attempt (a retried
    /// attempt resumes after its completed steps and runs failed ones again).
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
            let entry_attempt = fields["attempt"].as_u64().unwrap_or_default();
            match fields["status"].as_str() {
                Some("completed") => entries.push(json!({"key": fields["key"], "_tag": "Success", "value": fields["value"]})),
                Some("failed") if entry_attempt == u64::from(attempt) => {
                    entries.push(json!({"key": fields["key"], "_tag": "Failure", "error": fields["error"]}))
                }
                _ => {}
            }
        }
        Ok(entries)
    }

    fn ensure_host(self: &Arc<Self>, inner: &mut Inner) -> Result<()> {
        if let Some(host) = inner.host.as_mut()
            && host.child.try_wait()?.is_none()
        {
            return Ok(());
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
        inner.host = Some(Host { child, stdin, generation });
        let engine = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("program-host-{generation}"))
            .spawn(move || engine.read_host(stdout, generation))?;
        Ok(())
    }

    fn read_host(self: Arc<Self>, stdout: std::process::ChildStdout, generation: u64) {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let message = match serde_json::from_str::<Value>(&line) {
                Ok(message) => message,
                Err(error) => {
                    eprintln!("fractal: program host sent an unreadable line ({error}): {line}");
                    continue;
                }
            };
            if let Err(error) = self.handle(&message) {
                eprintln!("fractal: program host message {line}: {error:#}");
            }
        }
        self.host_exited(generation);
    }

    fn handle(&self, message: &Value) -> Result<()> {
        let step = message["step"].as_str().ok_or_else(|| anyhow!("message without a step"))?;
        let attempt = {
            let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(dispatch) = inner.running.get(step) else {
                return Ok(()); // A cancelled or finished program; its late messages change nothing.
            };
            dispatch.attempt
        };
        match message["t"].as_str() {
            Some("journal") => self.journal_entry(step, attempt, message),
            Some("done") => self.done(step, message),
            other => Err(anyhow!("unknown message type {other:?}")),
        }
    }

    fn journal_entry(&self, step: &str, attempt: u32, message: &Value) -> Result<()> {
        // A step that ended by other means (cancelled, timed out) stops its program at the next
        // journal write instead of running more effects.
        if let Some(view) = self.store.step_run(step)?
            && matches!(view.status.as_str(), "completed" | "failed" | "cancelled")
        {
            let reason = format!("the step is {}", view.status);
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            inner.running.remove(step);
            return send(&mut inner, &json!({"t": "cancel", "step": step, "reason": reason}));
        }
        let key = message["key"].as_str().ok_or_else(|| anyhow!("journal entry without a key"))?;
        let status = message["status"].as_str().ok_or_else(|| anyhow!("journal entry without a status"))?;
        let program = {
            let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            inner.running.get(step).map(|dispatch| dispatch.program.sha256.clone()).unwrap_or_default()
        };
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
        // A completed key is final for the step run; a failure is final for its attempt. The key
        // fences a double execution: a second, different result for one key is refused.
        let idempotency_key = match status {
            "completed" => Some(format!("program:{step}:{key}:completed")),
            "failed" => Some(format!("program:{step}:{attempt}:{key}:failed")),
            _ => None,
        };
        self.store.append_claim(&ClaimInput {
            subject: step.to_owned(),
            kind: JOURNAL_KIND.to_owned(),
            actor: None,
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key,
        })?;
        if status != "started" {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            send(&mut inner, &json!({"t": "ack", "step": step, "seq": message["seq"]}))?;
        }
        Ok(())
    }

    fn done(&self, step: &str, message: &Value) -> Result<()> {
        let exit = &message["exit"];
        let (status, reason) = match exit["_tag"].as_str() {
            Some("Success") => ("completed", None),
            _ => ("failed", Some(failure_reason(&exit["error"]))),
        };
        self.store.set_step_state_unless_terminal(step, status, reason.as_deref())?;
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).running.remove(step);
        eprintln!("fractal: program {step} {status}");
        (self.wake)();
        Ok(())
    }

    /// A host that exits takes its programs with it; a new host resumes each from its journal.
    fn host_exited(self: Arc<Self>, generation: u64) {
        let resume = {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            if inner.host.as_ref().is_none_or(|host| host.generation != generation) {
                return;
            }
            if let Some(mut host) = inner.host.take() {
                let status = host.child.wait();
                eprintln!("fractal: program host {generation} exited ({status:?})");
            }
            inner.running.drain().map(|(_, dispatch)| dispatch).collect::<Vec<_>>()
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

fn send(inner: &mut Inner, message: &Value) -> Result<()> {
    let host = inner.host.as_mut().ok_or_else(|| anyhow!("no program host"))?;
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    host.stdin.write_all(&line)?;
    host.stdin.flush()?;
    Ok(())
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
    fn replay_returns_completed_steps_and_only_this_attempts_failures() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let step = "step-run/generation/program";
        entry(&engine, step, "classify", "completed", 1, json!({"classes": ["routine"]}));
        entry(&engine, step, "check", "started", 1, Value::Null);
        entry(&engine, step, "check", "failed", 1, json!({"_tag": "StepFailed", "step": "check"}));
        entry(&engine, step, "verify-local#2", "completed", 2, json!({"stale": []}));
        let first = engine.journal(step, 1).unwrap();
        assert_eq!(
            first,
            vec![
                json!({"key": "classify", "_tag": "Success", "value": {"classes": ["routine"]}}),
                json!({"key": "check", "_tag": "Failure", "error": {"_tag": "StepFailed", "step": "check"}}),
                json!({"key": "verify-local#2", "_tag": "Success", "value": {"stale": []}}),
            ]
        );
        // A retried attempt keeps what completed and runs the failed step again.
        let second = engine.journal(step, 2).unwrap();
        assert_eq!(second.iter().map(|entry| entry["key"].as_str().unwrap()).collect::<Vec<_>>(), ["classify", "verify-local#2"]);
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
