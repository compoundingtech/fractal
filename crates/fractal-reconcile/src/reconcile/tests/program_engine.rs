//! The program engine with a real Bun host (docs/st3/program-engine.md): journal and replay
//! across a host crash, eager cancellation from the run's evaluation, and worker liveness.
//! Program steps need the runtime: these tests fail, never skip, when no `bun` is on `PATH`.
use super::*;
use crate::reconcile::program::{JOURNAL_KIND, Liveness, ProgramEngine, RESULT_KEY};
use serde_json::json;
use std::time::Instant;

/// A hand-written bundle against HostContext v1: one journaled step, then either a busy loop or
/// an effect that waits for a gate file while a background child shares its process group.
const BUNDLE: &str = r#"
const step = async (ctx, key, effect) => {
  const replayed = ctx.journal.get(key)
  if (replayed !== undefined) return replayed.value
  ctx.started(key)
  const at = Date.now()
  const value = await effect()
  await ctx.record({ key, status: 'completed', value, ms: Date.now() - at })
  return value
}
export default {
  [Symbol.for('fractal.program.v1')]: async (ctx) => {
    const first = await step(ctx, 'first', async () => ({ attempt: ctx.attempt }))
    if (ctx.payload.mode === 'retry-failure') return { _tag: 'Failure', error: { _tag: 'Broken' } }
    if (ctx.payload.mode === 'idle') await new Promise(() => {})
    if (ctx.payload.mode === 'trace') {
      const worker = process.env.TRACEPARENT
      const child = await ctx.spawn(['sh', '-c', 'printf %s \"$TRACEPARENT\"'], {
        env: { TRACEPARENT: 'must-not-override-attempt-context' },
      })
      return { _tag: 'Success', value: { worker, child: child.stdout } }
    }
    if (ctx.payload.mode === 'credential') {
      await step(ctx, 'who', async () => (await ctx.spawn(['sh', '-c', 'printf %s "$ST_RUN_CREDENTIAL"'])).stdout)
    }
    if (ctx.payload.mode === 'blocked-worker') {
      const fs = await import('node:fs')
      const waited = await step(ctx, 'native-worker', async () => {
        fs.writeFileSync(ctx.payload.pids, fs.readlinkSync('/proc/thread-self'))
        fs.readFileSync(ctx.payload.gate)
        return 'opened'
      })
      return { _tag: 'Success', value: { first, waited } }
    }
    if (ctx.payload.mode === 'busy') for (;;) {}
    const waited = await step(ctx, 'wait', async () => {
      const script = 'sleep 1000 & echo $$ $! > "$PIDS.tmp"; mv "$PIDS.tmp" "$PIDS"; while [ ! -e "$GATE" ]; do sleep 0.05; done; echo opened'
      const out = await ctx.spawn(['sh', '-c', script], { env: { PIDS: ctx.payload.pids, GATE: ctx.payload.gate } })
      return out.stdout.trim()
    })
    return { _tag: 'Success', value: { first, waited } }
  },
}
"#;

#[cfg(target_os = "linux")]
const REPLACEMENT_BUNDLE: &str = r#"
export default {
  [Symbol.for('fractal.program.v1')]: async (ctx) => {
    ctx.started('replacement')
    await ctx.record({ key: 'replacement', status: 'completed', value: true, ms: 0 })
    return { _tag: 'Success', value: true }
  },
}
"#;

struct Fixture {
    store: Arc<Store>,
    reconciler: Reconciler<FakeRuntime>,
    engine: Arc<ProgramEngine>,
    dir: tempfile::TempDir,
}

struct Started {
    run: String,
    step: String,
    pids: std::path::PathBuf,
    gate: std::path::PathBuf,
}

fn fixture(liveness: Liveness) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let bundle = dir.path().join("program.mjs");
    std::fs::write(&bundle, BUNDLE).unwrap();
    let hash = store.put_blob(BUNDLE.as_bytes()).unwrap();
    apply_source(
        &store,
        &format!(
            r#"version 2
mission "program-test" state="ready" {{
  goal "Run a program."
  concurrent-runs max=4
  input "mode" kind="text"
  input "pids" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{}" sha256="{hash}" }}
}}"#,
            bundle.display()
        ),
        "program-test-definition",
    );
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    let incremental = reconciler.incremental.clone();
    let notify = reconciler.notify.clone();
    let event_notify = reconciler.event_notify.clone();
    let engine = ProgramEngine::new(store.clone(), Arc::new(move |subject: &str| {
        incremental.touch(subject);
        signal_changed(&notify, &event_notify);
    }), dir.path().join("programs"), liveness);
    assert!(reconciler.program_engine.set(engine.clone()).is_ok());
    Fixture { store, reconciler, engine, dir }
}

impl Fixture {
    fn start(&self, name: &str, mode: &str) -> Started {
        self.start_mission("program-test", name, mode)
    }

    fn start_mission(&self, mission: &str, name: &str, mode: &str) -> Started {
        let pids = self.dir.path().join(format!("{name}.pids"));
        let gate = self.dir.path().join(format!("{name}.gate"));
        let inputs = BTreeMap::from([
            ("mode".to_owned(), mode.to_owned()),
            ("pids".to_owned(), pids.display().to_string()),
            ("gate".to_owned(), gate.display().to_string()),
        ]);
        let run = self
            .store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: self.dir.path().display().to_string(),
                requester: None,
                mode: None,
                inputs,
                idempotency_key: format!("program-test-{name}"),
            })
            .unwrap();
        let step = run.steps.iter().find(|step| step.step == "program").unwrap().subject.clone();
        Started { run: run.subject, step, pids, gate }
    }

    #[cfg(target_os = "linux")]
    fn publish(&self, mission: &str, bundle: &str, definition: &str) {
        let path = self.dir.path().join(format!("{definition}.mjs"));
        std::fs::write(&path, bundle).unwrap();
        let hash = self.store.put_blob(bundle.as_bytes()).unwrap();
        apply_source(&self.store, &format!(r#"version 2
mission "{mission}" state="ready" {{
  goal "Run the published program."
  concurrent-runs max=4
  input "mode" kind="text"
  input "pids" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{}" sha256="{hash}" }}
}}"#, path.display()), definition);
    }

    #[cfg(target_os = "linux")]
    fn blocked_worker(&self, name: &str) -> (Started, std::fs::File, std::path::PathBuf) {
        use std::os::unix::ffi::OsStrExt as _;
        let gate_path = self.dir.path().join(format!("{name}.gate"));
        let path = std::ffi::CString::new(gate_path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the NUL-terminated path lives through mkfifo and names this test's directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let gate = std::fs::OpenOptions::new().read(true).write(true).open(&gate_path).unwrap();
        let started = self.start(name, "blocked-worker");
        let thread_path = || std::fs::read_to_string(&started.pids).ok()
            .filter(|thread| !thread.trim().is_empty())
            .map(|thread| std::path::PathBuf::from("/proc").join(thread.trim()));
        self.until("the real worker blocks in its native FIFO read", || {
            thread_path().and_then(|path| std::fs::read_to_string(path.join("stat")).ok())
                .is_some_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, fields)| fields.starts_with('S')))
        });
        let task = thread_path().unwrap();
        (started, gate, task)
    }

    /// Run passes until `done` holds; the engine itself works off the reconciler thread.
    fn until(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            self.reconciler.reconcile_once().unwrap();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait without any reconciler pass: what happens now is the engine's and the host's alone.
    fn without_passes(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn status(&self, step: &str) -> String {
        self.store.step_run(step).unwrap().unwrap().status
    }

    /// `(key, status)` of each journal entry, in store order.
    fn journal(&self, step: &str) -> Vec<(String, String)> {
        let mut claims = self.store.claims_for(step, Some(JOURNAL_KIND)).unwrap();
        claims.sort_by_key(|claim| claim.store_index);
        claims
            .iter()
            .map(|claim| {
                let fields = &claim.body["fields"];
                (fields["key"].as_str().unwrap().to_owned(), fields["status"].as_str().unwrap().to_owned())
            })
            .collect()
    }
}

fn pids(path: &std::path::Path) -> Option<Vec<i32>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.split_whitespace().map(|pid| pid.parse().unwrap()).collect())
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn entries(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(key, status)| ((*key).to_owned(), (*status).to_owned())).collect()
}

#[test]
fn trace_context_crosses_host_worker_and_exec_and_is_retained_with_result() {
    let fixture = fixture(Liveness::default());
    let first = fixture.start("trace-first", "trace");
    let second = fixture.start("trace-second", "trace");
    fixture.until("both trace attempts complete", || fixture.status(&first.step) == "completed"
        && fixture.status(&second.step) == "completed");
    let read = |step: &str| {
        let claims = fixture.store.claims_for(step, Some(JOURNAL_KIND)).unwrap();
        let result = claims.iter().find(|claim| claim.body["fields"]["key"] == RESULT_KEY).unwrap();
        let fields = &result.body["fields"];
        assert!(fields.get("trace").is_none(), "journal fields must stay readable by older peers");
        let evidence = result.body["evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 1);
        let traces = fixture.store.claims_for(step, Some("program.attempt.trace")).unwrap();
        let trace = traces.iter().find(|trace| Some(trace.id.as_str()) == evidence[0].as_str()).unwrap();
        let trace: crate::otel::AttemptTraceContext = serde_json::from_value(trace.body["fields"]["trace"].clone()).unwrap();
        assert_eq!(fields["value"]["worker"], trace.traceparent);
        assert_eq!(fields["value"]["child"], trace.traceparent);
        assert_eq!(trace.traceparent.len(), 55);
        trace
    };
    assert_ne!(read(&first.step).trace_id(), read(&second.step).trace_id(),
        "the shared host must not leak context between concurrent attempts");
}

#[test]
fn a_program_resumes_from_its_journal_after_its_host_is_killed() {
    let fixture = fixture(Liveness::default());
    let started = fixture.start("resume", "wait");
    fixture.until("the effect runs", || pids(&started.pids).is_some());
    let before = pids(&started.pids).unwrap();
    assert_eq!(fixture.journal(&started.step), entries(&[("first", "started"), ("first", "completed"), ("wait", "started")]));
    let host = fixture.engine.host_pid().unwrap();
    std::fs::remove_file(&started.pids).unwrap();
    // SAFETY: the host is this test's own child process.
    assert_eq!(unsafe { libc::kill(host as i32, libc::SIGKILL) }, 0);
    // A new host resumes the attempt: `first` replays from the journal and `wait` runs again.
    fixture.without_passes("the effect runs on a new host", || pids(&started.pids).is_some());
    assert_ne!(fixture.engine.host_pid(), Some(host));
    // The killed host's effect group, leader and background child, ended before the new host started.
    assert!(before.iter().all(|pid| !alive(*pid)), "survivors of the killed host: {before:?}");
    std::fs::write(&started.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&started.step) == "completed");
    assert_eq!(
        fixture.journal(&started.step),
        entries(&[
            ("first", "started"),
            ("first", "completed"),
            ("wait", "started"),
            ("wait", "started"),
            ("wait", "completed"),
            (RESULT_KEY, "completed"),
        ])
    );
    let result = fixture.store.claims_for(&started.step, Some(JOURNAL_KIND)).unwrap().into_iter().max_by_key(|claim| claim.store_index).unwrap();
    assert_eq!(result.body["fields"]["value"], json!({"first": {"attempt": 1}, "waited": "opened"}));
}

/// A program run that `missions start` just created journals its first inner step from the
/// change feed alone (R14): the follower evaluates the new run, which readies and dispatches its
/// program step, with no pass, which would first wait out `StartSpacing`.
#[test]
fn a_new_program_run_starts_its_first_step_without_a_pass() {
    let fixture = fixture(Liveness::default());
    // A running daemon's passes, and so its follower, have read the feed up to here.
    fixture.reconciler.reconcile_once().unwrap();
    let started = fixture.start("first", "wait");
    // The follower's chain after the run's creation, as `follow_mission_reads` runs it: an
    // evaluation that wrote is followed at once by another.
    for _ in 0..=MISSION_READS_CONTINUATIONS {
        if !fixture.reconciler.evaluate_changed_mission_runs().unwrap().1 {
            break;
        }
    }
    fixture.without_passes("the first inner step starts", || !fixture.journal(&started.step).is_empty());
    assert_eq!(fixture.journal(&started.step)[0], ("first".to_owned(), "started".to_owned()));
    std::fs::write(&started.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&started.step) == "completed");
}

#[test]
fn a_step_that_leaves_its_attempt_stops_its_program_and_effects_at_once() {
    let fixture = fixture(Liveness::default());
    let cancelled = fixture.start("cancelled", "wait");
    let failed = fixture.start("failed", "wait");
    fixture.until("both effects run", || pids(&cancelled.pids).is_some() && pids(&failed.pids).is_some());
    let group = |started: &Started| pids(&started.pids).unwrap();
    let (cancelled_group, failed_group) = (group(&cancelled), group(&failed));
    fixture.store.request_mission_run_cancellation(&cancelled.run, "not wanted").unwrap();
    fixture.store.set_step_state(&failed.step, "failed", Some("the active execution timeout expired")).unwrap();
    // One evaluation of each run tells the engine; nothing waits for a journal write.
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    fixture.without_passes("both effect groups end", || cancelled_group.iter().chain(&failed_group).all(|pid| !alive(*pid)));
    std::fs::write(&cancelled.gate, "").unwrap();
    std::fs::write(&failed.gate, "").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    for started in [&cancelled, &failed] {
        assert_eq!(fixture.journal(&started.step), entries(&[("first", "started"), ("first", "completed"), ("wait", "started")]));
        assert_ne!(fixture.status(&started.step), "completed");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn supersession_cancellation_acknowledges_the_actual_worker_exit() {
    let fixture = fixture(Liveness::default());
    let (started, gate, task) = fixture.blocked_worker("blocked-worker");
    fixture.store.request_mission_run_cancellation(&started.run, "stop the native worker").unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    // This later program's journal roundtrip proves the shared host processed
    // cancellation without blocking other programs on the terminating worker.
    let probe = fixture.start("worker-exit-probe", "retry-failure");
    fixture.until("the shared host handles the later program", || {
        fixture.journal(&probe.step).iter().any(|(key, status)| key == "first" && status == "completed")
    });
    assert!(
        !fixture.engine.fence_left_effects(&fixture.store.mission_run(&started.run).unwrap().unwrap()).unwrap() || !task.exists(),
        "cancellation acknowledgement must not fence an executing worker thread",
    );
    drop(gate);
    fixture.without_passes("the worker exits and cancellation is acknowledged", || {
        fixture.engine.fence_left_effects(&fixture.store.mission_run(&started.run).unwrap().unwrap()).unwrap() && !task.exists()
    });
}

#[cfg(target_os = "linux")]
#[test]
fn liveness_termination_fences_publication_until_the_native_worker_exits() {
    let fixture = fixture(Liveness { worker_deadline: Duration::from_millis(1_000), ..Liveness::default() });
    let (old, gate, task) = fixture.blocked_worker("liveness-worker");
    // Observe the host fault without evaluating the run: publication must encounter
    // a still-active source whose worker has already entered asynchronous termination.
    fixture.without_passes("the native worker misses its liveness deadline", || {
        fixture.engine.host_fault(&old.step, 1).is_some() && fixture.status(&old.step) == "failed"
    });
    assert!(task.exists(), "liveness reporting must not wait for native I/O to exit");
    assert_eq!(fixture.store.mission_run(&old.run).unwrap().unwrap().phase, "normal");
    let host = fixture.engine.host_pid();
    let journal = fixture.journal(&old.step);
    fixture.publish("program-test", REPLACEMENT_BUNDLE, "liveness-publication");
    let source = fixture.store.mission_run(&old.run).unwrap().unwrap();
    let replacement = fixture.store.mission_run(source.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    fixture.until("the superseded source is terminal", || {
        fixture.store.mission_run(&old.run).unwrap().unwrap().phase == "terminal"
    });
    // With a terminal predecessor the only remaining barrier is actual worker exit.
    fixture.reconciler.evaluate_mission_run_ids(vec![replacement.id.clone()]).unwrap();
    assert_eq!(fixture.store.mission_run(&replacement.id).unwrap().unwrap().phase, "supersession-waiting");
    assert!(fixture.journal(&replacement.steps[0].subject).is_empty());
    assert!(task.exists(), "the native worker is still blocked, not merely awaiting an acknowledgement");
    // A later journal roundtrip demonstrates that termination did not block the host.
    let probe = fixture.start("liveness-probe", "idle");
    fixture.until("the shared host completes a later program", || fixture.status(&probe.step) == "completed");
    assert_eq!(fixture.engine.host_pid(), host);
    assert_eq!(fixture.store.mission_run(&replacement.id).unwrap().unwrap().phase, "supersession-waiting");
    assert!(fixture.journal(&replacement.steps[0].subject).is_empty());
    drop(gate);
    fixture.without_passes("the liveness-terminated native worker exits", || !task.exists());
    fixture.until("the replacement completes after native worker exit", || {
        fixture.status(&replacement.steps[0].subject) == "completed"
    });
    assert!(fixture.journal(&replacement.steps[0].subject).contains(&("replacement".into(), "completed".into())));
    assert_eq!(fixture.journal(&old.step), journal, "the terminated worker cannot publish a late result");
}

#[cfg(target_os = "linux")]
#[test]
fn unrelated_native_worker_cancellation_does_not_fence_another_missions_publication() {
    let fixture = fixture(Liveness::default());
    let (blocked, gate, task) = fixture.blocked_worker("unrelated-native-worker");
    fixture.store.request_mission_run_cancellation(&blocked.run, "stop the unrelated native worker").unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    fixture.publish("unrelated-program", BUNDLE, "unrelated-definition");
    let old = fixture.start_mission("unrelated-program", "quiescent-worker", "idle");
    // This roundtrip is after A's cancel on the shared pipe. B owns no process
    // group and still answers pings; only A has native I/O preventing exit.
    fixture.until("the unrelated program journals its first step", || {
        fixture.journal(&old.step).contains(&("first".into(), "completed".into()))
    });
    assert!(task.exists());
    let host = fixture.engine.host_pid();
    fixture.publish("unrelated-program", REPLACEMENT_BUNDLE, "unrelated-publication");
    let source = fixture.store.mission_run(&old.run).unwrap().unwrap();
    let replacement = fixture.store.mission_run(source.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    fixture.until("the unrelated replacement is admitted and completes", || {
        fixture.status(&replacement.steps[0].subject) == "completed"
    });
    assert!(task.exists(), "mission A must still be awaiting native worker exit while B advances");
    assert_eq!(fixture.engine.host_pid(), host);
    assert!(fixture.journal(&replacement.steps[0].subject).contains(&("replacement".into(), "completed".into())));
    assert!(!fixture.journal(&blocked.step).contains(&("native-worker".into(), "completed".into())));
    drop(gate);
    fixture.without_passes("the unrelated native worker exits", || !task.exists());
}

#[test]
fn an_unresponsive_worker_fails_only_its_own_step() {
    let fixture = fixture(Liveness { worker_deadline: Duration::from_millis(1_000), ..Liveness::default() });
    let busy = fixture.start("busy", "busy");
    let waiting = fixture.start("waiting", "wait");
    fixture.until("the busy program fails", || fixture.status(&busy.step) == "failed");
    let reason = fixture.store.latest_claim(&busy.step, Some("step-run.state")).unwrap().unwrap().body["fields"]["reason"].clone();
    assert_eq!(reason, "the program's worker missed its 1000 ms liveness deadline and was terminated");
    assert_eq!(fixture.engine.host_fault(&busy.step, 1).as_deref(), reason.as_str());
    let host = fixture.engine.host_pid();
    fixture.until("the other program's effect runs", || pids(&waiting.pids).is_some());
    std::fs::write(&waiting.gate, "").unwrap();
    fixture.until("the other program completes", || fixture.status(&waiting.step) == "completed");
    // The host itself stayed up: only the busy worker was terminated.
    assert_eq!(fixture.engine.host_pid(), host);
}

/// An effect carries a credential that binds its run while the attempt runs and no longer once
/// it ends, and the journal is written as the run (docs/st3/program-engine.md §3).
#[test]
fn a_programs_effects_and_journal_act_as_its_run() {
    let fixture = fixture(Liveness::default());
    let started = fixture.start("credential", "credential");
    fixture.until("the effect reports its credential", || pids(&started.pids).is_some());
    let who = fixture.store.claims_for(&started.step, Some(JOURNAL_KIND)).unwrap().into_iter()
        .find(|claim| claim.body["fields"]["key"] == "who" && claim.body["fields"]["status"] == "completed")
        .unwrap();
    let credential = who.body["fields"]["value"].as_str().unwrap().to_owned();
    let binding = fixture.store.run_binding(&credential).unwrap();
    assert_eq!((binding.run.as_str(), binding.step_run.as_str(), binding.attempt), (started.run.as_str(), started.step.as_str(), 1));
    // Passes over a running attempt mint nothing more, and the credential has no expiry of its own.
    for _ in 0..5 {
        fixture.reconciler.reconcile_once().unwrap();
    }
    assert_eq!(fixture.store.run_credentials_issued_for_tests(&started.step), 1);
    let capability = fixture.store.capability(&credential, crate::store::run_credentials::RUN_CREDENTIAL_KIND).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    assert!(capability.expires_at_unix_ms > now + 100 * 365 * 24 * 60 * 60 * 1000);
    std::fs::write(&started.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&started.step) == "completed");
    assert_eq!(fixture.store.run_binding(&credential).unwrap_err().code, "run-credential-ended");
    let claims = fixture.store.claims_for(&started.step, Some(JOURNAL_KIND)).unwrap();
    assert!(!claims.is_empty());
    assert!(claims.iter().all(|claim| claim.actor.as_deref() == Some(started.run.as_str())), "{claims:#?}");
}

#[test]
fn run_retry_program_starts_from_top_with_a_fresh_journal_and_selected_bundle() {
    let fixture = fixture(Liveness::default());
    let source = fixture.start("retry", "retry-failure");
    fixture.until("the source run fails after journaling first", || {
        fixture.store.mission_run(&source.run).unwrap().unwrap().status == "failed"
    });
    let failed = fixture.store.mission_run(&source.run).unwrap().unwrap();
    assert!(fixture.journal(&source.step).contains(&("first".into(), "completed".into())));
    let old_journal = fixture.store.claims_for(&source.step, Some(JOURNAL_KIND)).unwrap();
    let fixed = BUNDLE.replace(
        "if (ctx.payload.mode === 'retry-failure') return { _tag: 'Failure', error: { _tag: 'Broken' } }",
        "",
    );
    let bundle = fixture.dir.path().join("fixed.mjs");
    std::fs::write(&bundle, &fixed).unwrap();
    let hash = fixture.store.put_blob(fixed.as_bytes()).unwrap();
    apply_source(&fixture.store, &format!(r#"version 2
mission "program-test" state="ready" {{
  goal "Run a program."
  concurrent-runs max=4
  input "mode" kind="text"
  input "pids" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{}" sha256="{hash}" }}
}}"#, bundle.display()), "fixed-program");
    std::fs::write(&source.gate, "").unwrap();
    let options = crate::model::MissionRunRetryRequest {
        actor: "person/operator".into(), same_revision: false, reason: None,
        idempotency_key: "retry-latest-program".into(),
    };
    let retried = fixture.store.retry_mission_run(&source.run, &options).unwrap();
    let step = &retried.steps[0].subject;
    assert!(fixture.journal(step).is_empty());
    fixture.until("latest retry completes", || fixture.status(step) == "completed");
    assert!(fixture.journal(step).contains(&("first".into(), "completed".into())));
    assert_ne!(retried.revision, failed.revision);
    assert_eq!(retried.inputs, failed.inputs);
    assert_eq!(fixture.store.claims_for(&source.step, Some(JOURNAL_KIND)).unwrap().len(), old_journal.len());
    let original = fixture.store.retry_mission_run(&source.run, &crate::model::MissionRunRetryRequest {
        same_revision: true, idempotency_key: "retry-original-program".into(), ..options
    }).unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    assert!(fixture.store.mission_run(&original.id).unwrap().unwrap().superseded_by.is_none(),
        "an explicit same-revision retry admitted after publication must not be superseded");
    fixture.until("same revision retries the original failure", || fixture.status(&original.steps[0].subject) == "failed");
    assert_eq!(original.revision, failed.revision);
    assert!(fixture.journal(&original.steps[0].subject).contains(&("first".into(), "completed".into())));
    assert!(fixture.store.mission_run(&original.id).unwrap().unwrap().superseded_by.is_none());
    assert_eq!(fixture.store.mission_run(&source.run).unwrap().unwrap().generation, failed.generation);
}

#[test]
fn publication_restarts_a_mid_journal_program_fresh_and_stops_old_effect_groups() {
    let fixture = fixture(Liveness::default());
    let old = fixture.start("superseded", "wait");
    fixture.until("the old effect starts", || pids(&old.pids).is_some());
    let old_pids = pids(&old.pids).unwrap();
    let journal = fixture.journal(&old.step);
    assert!(journal.contains(&("first".into(), "completed".into())));
    let fixed = BUNDLE.replace("({ attempt: ctx.attempt })", "({ fresh: true, attempt: ctx.attempt })");
    let bundle = fixture.dir.path().join("published.mjs");
    std::fs::write(&bundle, &fixed).unwrap();
    let hash = fixture.store.put_blob(fixed.as_bytes()).unwrap();
    apply_source(&fixture.store, &format!(r#"version 2
mission "program-test" state="ready" {{
  goal "Run the corrected program from its top."
  concurrent-runs max=4
  input "mode" kind="text"
  input "pids" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{}" sha256="{hash}" }}
}}"#, bundle.display()), "superseding-program");
    let source = fixture.store.mission_run(&old.run).unwrap().unwrap();
    let replacement = fixture.store.mission_run(source.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!(replacement.phase, "supersession-waiting");
    let step = replacement.steps[0].subject.clone();
    assert!(fixture.journal(&step).is_empty());
    // Even evaluation in reverse order cannot dispatch the replacement early.
    fixture.reconciler.evaluate_mission_run_ids(vec![replacement.id.clone()]).unwrap();
    assert!(fixture.journal(&step).is_empty());
    fixture.until("the replacement journals first", || fixture.journal(&step).contains(&("first".into(), "completed".into())));
    fixture.without_passes("old effects are reaped", || old_pids.iter().all(|pid| !alive(*pid)));
    assert_eq!(fixture.store.mission_run(&old.run).unwrap().unwrap().phase, "terminal");
    assert_eq!(fixture.journal(&old.step), journal, "the retired program wrote no later journal result");
    let first = fixture.store.claims_for(&step, Some(JOURNAL_KIND)).unwrap().into_iter()
        .find(|claim| claim.body["fields"]["key"] == "first" && claim.body["fields"]["status"] == "completed").unwrap();
    assert_eq!(first.body["fields"]["value"]["fresh"], true, "the old journal was not replayed");
    std::fs::write(&old.gate, "").unwrap();
    fixture.until("the fresh program completes", || fixture.status(&step) == "completed");
    assert_eq!(fixture.store.mission_run(&replacement.id).unwrap().unwrap().inputs, source.inputs);
}
