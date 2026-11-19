use super::*;

fn engine(dir: &Path) -> Arc<ProgramEngine> {
    ProgramEngine::new(Arc::new(Store::open_memory("node").unwrap()), Arc::new(|_: &str| {}), dir.to_owned(), Liveness::default())
}

fn entry(engine: &ProgramEngine, step: &str, key: &str, status: &str, attempt: u32, value: Value) {
    let name = if status == "failed" { "error" } else { "value" };
    let result = Value::Object(serde_json::Map::from_iter([(name.to_owned(), value)]));
    engine.write_journal(step, attempt, &"a".repeat(64), key, status, &result, None).unwrap();
}

fn journal_claims(engine: &ProgramEngine, step: &str) -> usize {
    engine.store.claims_for(step, Some(JOURNAL_KIND)).unwrap().len()
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
    entry(&engine, step, RESULT_KEY, "completed", 1, json!({"done": true}));
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
    assert!(journal.iter().enumerate().all(|(index, entry)| entry["value"] == index + 1));
}

#[test]
fn a_revision_runs_the_stored_blob_it_pins() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let hash = engine.store.put_blob(b"export default 1").unwrap();
    // The author's path is provenance only: the member runs the store's bytes.
    let program = ProgramSpec { bundle: "/nowhere/land.mjs".into(), bundle_hash: hash.clone(), stalled_after_ms: None };
    let pinned = engine.pinned_module(&program).unwrap();
    assert_eq!(std::fs::read(&pinned).unwrap(), b"export default 1");
    // A damaged pinned copy is replaced from the blob, never run.
    std::fs::write(&pinned, b"export default 2").unwrap();
    assert_eq!(engine.pinned_module(&program).unwrap(), pinned);
    assert_eq!(std::fs::read(&pinned).unwrap(), b"export default 1");
    // A revision whose blob this member does not hold fails its step.
    let missing = ProgramSpec { bundle_hash: "0".repeat(64), ..program };
    assert!(engine.pinned_module(&missing).unwrap_err().contains("is not in this member's store"));
}

#[test]
fn a_key_has_one_result_per_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let step = "step-run/generation/program";
    let program = "a".repeat(64);
    let write = |attempt, status, value: Value| {
        engine.write_journal(step, attempt, &program, "merge", status, &json!({"value": value, "ms": 3}), None)
    };
    write(1, "started", Value::Null).unwrap();
    write(1, "started", Value::Null).unwrap();
    write(1, "completed", json!({"commit": "c1"})).unwrap();
    // The same result again (a resent entry) is the same durable result.
    write(1, "completed", json!({"commit": "c1"})).unwrap();
    // A double execution with a different result, or a failure for a completed key, is refused.
    assert!(write(1, "completed", json!({"commit": "c2"})).is_err());
    assert!(engine.write_journal(step, 1, &program, "merge", "failed", &json!({"error": {"_tag": "X"}}), None).is_err());
    // A retried attempt has its own result.
    write(2, "completed", json!({"commit": "c2"})).unwrap();
    assert_eq!(journal_claims(&engine, step), 4);
    assert_eq!(engine.journal(step, 1).unwrap(), vec![json!({"key": "merge", "_tag": "Success", "value": {"commit": "c1"}})]);
}

#[test]
fn messages_for_another_attempt_or_host_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let step = "step-run/generation/program";
    let dispatch = Dispatch {
        step: step.into(),
        attempt: 2,
        run: "mission-run/run".into(),
        program: ProgramSpec { bundle: "/b.mjs".into(), bundle_hash: "a".repeat(64), stalled_after_ms: None },
        payload: BTreeMap::new(),
        env: BTreeMap::new(),
        cwd: "/".into(),
    };
    engine.lock().running.insert(step.into(), Running { dispatch, generation: 1, requests: BTreeMap::new(),
        _trace: crate::otel::AttemptTrace::start("st.program.attempt", Vec::new(), &[]) });
    let message = |attempt: u32, t: &str| {
        json!({"t": t, "step": step, "attempt": attempt, "seq": 1, "key": "merge", "status": "completed", "value": 1,
            "exit": {"_tag": "Success", "value": 1}, "ms": 5, "mission": "publish", "input": {}, "for": "person/owner", "title": "Go?"})
    };
    for t in ["journal", "run", "ask", "ending", "done", "unresponsive"] {
        // An earlier attempt of the step, then the right attempt from a replaced host.
        engine.handle(&message(1, t), 1).unwrap();
        engine.handle(&message(2, t), 2).unwrap();
    }
    assert_eq!(journal_claims(&engine, step), 0);
    assert!(engine.lock().running.get(step).is_some_and(|running| running.requests.is_empty()));
    assert_eq!(engine.host_fault(step, 2), None);
    assert!(engine.lock().terminating.is_empty());
    // A late message for the earlier attempt never removes the attempt that replaced it, and a
    // refused entry is answered rather than left waiting (no host here, so nothing is sent).
    engine.done(step, 1, 1, &"a".repeat(64), &message(1, "done")).unwrap();
    engine.unresponsive(step, 1, 1, &message(1, "unresponsive")).unwrap();
    assert!(engine.lock().running.contains_key(step));
    assert_eq!(engine.host_fault(step, 1), None);
}

/// A pass may hand the engine a view read before an attempt ended. An attempt this engine is
/// ending never starts again, even while the store still says it runs, and neither does one its
/// step left; the fault that stopped it stays. The engine holds an attempt as ended only until
/// the step's state is written or the write fails, and the next attempt still starts.
#[test]
fn a_dispatch_for_an_attempt_that_ended_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let step = engine.store.start_program_step_for_tests();
    let hash = engine.store.put_blob(b"export default 1").unwrap();
    let dispatch = |attempt: u32| Dispatch {
        step: step.subject.clone(),
        attempt,
        run: step.run.clone(),
        program: ProgramSpec { bundle: "/b.mjs".into(), bundle_hash: hash.clone(), stalled_after_ms: None },
        payload: BTreeMap::new(),
        env: BTreeMap::new(),
        cwd: "/".into(),
    };
    engine.lock().running.insert(step.subject.clone(), Running { dispatch: dispatch(1), generation: 1, requests: BTreeMap::new(),
        _trace: crate::otel::AttemptTrace::start("st.program.attempt", Vec::new(), &[]) });
    engine.unresponsive(&step.subject, 1, 1, &json!({"ms": 1000})).unwrap();
    let reason = "the program's worker missed its 1000 ms liveness deadline and was terminated";
    assert_eq!(engine.store.step_run(&step.subject).unwrap().unwrap().status, "failed");
    assert_eq!(engine.host_fault(&step.subject, 1).as_deref(), Some(reason));
    assert!(engine.lock().ended.is_empty(), "the written state, not the engine, now says it ended");
    // While the engine is ending attempt 1, a pass's view read before the failure is written.
    engine.store.set_step_run_state_for_tests(&step.subject, "working", 1);
    engine.lock().ended.insert(step.subject.clone(), 1);
    assert_eq!(engine.dispatch(dispatch(1)).unwrap(), Dispatched::Left("the program of attempt 1 already ended".into()));
    // A write that fails leaves the attempt to the next pass, as before.
    let failed = engine.settle(&step.subject, 1, || -> Result<()> { Err(anyhow!("database is locked")) });
    assert!(failed.is_err());
    assert!(engine.lock().ended.is_empty());
    assert_eq!(engine.ended_or_left(&engine.lock(), &step.subject, 1).unwrap(), None);
    // An attempt its step left, here failed by the reconciler, does not start either.
    engine.store.set_step_run_state_for_tests(&step.subject, "failed", 2);
    assert_eq!(engine.dispatch(dispatch(2)).unwrap(), Dispatched::Left("the step is failed at attempt 2".into()));
    // The retry's attempt starts.
    engine.store.set_step_run_state_for_tests(&step.subject, "working", 3);
    assert_eq!(engine.ended_or_left(&engine.lock(), &step.subject, 3).unwrap(), None);
    let inner = engine.lock();
    assert!(inner.host.is_none() && inner.running.is_empty());
}

#[test]
fn recorded_groups_are_killed_only_when_their_leader_is_verified() {
    use std::os::unix::process::CommandExt as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let groups = engine.groups_dir();
    std::fs::create_dir_all(&groups).unwrap();
    let boot = boot_id().unwrap();
    let leader = || Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let alive = |child: &mut Child| child.try_wait().unwrap().is_none();
    let record = |child: &Child, boot: &str, start: Option<String>| {
        let pid = child.id();
        std::fs::write(groups.join(format!("{pid}.json")), json!({"pid": pid, "boot_id": boot, "start_time": start}).to_string()).unwrap();
    };
    // Recorded by this boot with the leader's start time: an effect a dead host left behind.
    let mut ours = leader();
    record(&ours, &boot, start_time(ours.id() as i32));
    // The host died between writing this record and renaming it into place: the group runs.
    std::fs::rename(groups.join(format!("{}.json", ours.id())), groups.join(format!("{}.json.tmp", ours.id()))).unwrap();
    // The same pid with another start time is a process that reused it.
    let mut reused = leader();
    record(&reused, &boot, Some("0".into()));
    // A record from an earlier boot is dropped without a kill: its pid may be anyone's now.
    let mut earlier = leader();
    record(&earlier, "an-earlier-boot", start_time(earlier.id() as i32));
    std::fs::write(groups.join("half-written.json.tmp"), "{").unwrap();
    assert_eq!(engine.reap_recorded_groups().unwrap(), 1);
    let status = ours.wait().unwrap();
    assert_eq!(std::os::unix::process::ExitStatusExt::signal(&status), Some(libc::SIGKILL));
    assert!(alive(&mut reused) && alive(&mut earlier));
    assert_eq!(std::fs::read_dir(&groups).unwrap().count(), 0);
    for mut child in [reused, earlier] {
        child.kill().unwrap();
        child.wait().unwrap();
    }
}

#[test]
fn supersession_fences_recorded_effect_groups_without_signalling_reused_identities() {
    use std::os::unix::process::CommandExt as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let step = engine.store.start_program_step_for_tests();
    engine.store.set_step_run_state_for_tests(&step.subject, "failed", 1);
    let run = engine.store.mission_run(&step.run).unwrap().unwrap();
    let groups = engine.groups_dir();
    std::fs::create_dir_all(&groups).unwrap();
    let mut effect = Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let mut reused = Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let record = |child: &Child, start: Option<String>| {
        std::fs::write(groups.join(format!("{}.json", child.id())), json!({
            "pid": child.id(), "boot_id": boot_id().unwrap(), "start_time": start,
            "step": step.subject, "attempt": 1,
        }).to_string()).unwrap();
    };
    // An unconfirmed identity is a visible error, never permission to run the
    // replacement or to signal a process whose identity we cannot prove.
    record(&effect, None);
    assert!(engine.fence_left_effects(&run).is_err());
    assert!(effect.try_wait().unwrap().is_none());
    record(&effect, start_time(effect.id() as i32));
    record(&reused, Some("not-this-process".into()));
    assert!(engine.fence_left_effects(&run).unwrap());
    let status = effect.wait().unwrap();
    assert_eq!(std::os::unix::process::ExitStatusExt::signal(&status), Some(libc::SIGKILL));
    assert!(reused.try_wait().unwrap().is_none());
    reused.kill().unwrap();
    reused.wait().unwrap();
}

fn scoped_run(engine: &ProgramEngine, key: &str) -> MissionRunView {
    if engine.store.mission_spec("worker-scope", None).unwrap().is_none() {
        crate::reconcile::tests::apply_source(&engine.store, r#"version 2
mission "worker-scope" state="ready" {
  goal "Keep execution ownership exact."
  concurrent-runs max=16
  step "work" { agentless }
}"#, "worker-scope-declaration");
    }
    engine.store.create_mission_run(&crate::model::MissionRunRequest {
        mission: "worker-scope".into(), revision: None,
        workspace: "/tmp".into(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: key.into(),
    }).unwrap()
}

fn running_attempt(engine: &ProgramEngine, run: &MissionRunView, attempt: u32, generation: u64) {
    let step = &run.steps[0].subject;
    engine.store.set_step_run_state_for_tests(step, "working", attempt);
    engine.lock().running.insert(step.clone(), Running {
        dispatch: Dispatch {
            step: step.clone(), attempt, run: run.subject.clone(),
            program: ProgramSpec { bundle: "/bundle.mjs".into(), bundle_hash: "a".repeat(64), stalled_after_ms: None },
            payload: BTreeMap::new(), env: BTreeMap::new(), cwd: "/".into(),
        },
        generation, requests: BTreeMap::new(),
        _trace: crate::otel::AttemptTrace::start("st.program.attempt", Vec::new(), &[]),
    });
}

#[test]
fn host_ending_survives_outcome_and_requires_each_exact_termination_acknowledgement() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    let wakes = Arc::new(Mutex::new(Vec::new()));
    let recorded = wakes.clone();
    let engine = ProgramEngine::new(store, Arc::new(move |key| recorded.lock().unwrap().push(key.to_owned())),
        dir.path().to_owned(), Liveness::default());
    let run = scoped_run(&engine, "ending-scope");
    let step = &run.steps[0].subject;
    let message = |t: &str, attempt| json!({"t": t, "step": step, "attempt": attempt, "ms": 1});
    running_attempt(&engine, &run, 1, 1);
    engine.handle(&message("ending", 1), 1).unwrap();
    engine.handle(&message("unresponsive", 1), 1).unwrap();
    assert_eq!(engine.store.step_run(step).unwrap().unwrap().status, "failed");
    assert!(!engine.lock().running.contains_key(step));
    engine.release_left(&engine.store.mission_run(&run.id).unwrap().unwrap()).unwrap();
    assert!(!engine.fence_left_effects(&run).unwrap());
    // A later attempt, and the same attempt resumed on another host, cannot overwrite
    // the native-I/O worker that is still terminating on its original host.
    for (attempt, generation) in [(2, 1), (2, 2)] {
        running_attempt(&engine, &run, attempt, generation);
        engine.handle(&message("ending", attempt), generation).unwrap();
        assert!(engine.end(step, attempt, generation, None, false).is_some());
    }
    engine.handle(&message("cancelled", 1), 2).unwrap();
    assert_eq!(engine.lock().terminating.len(), 3);
    for (attempt, generation, remaining) in [(1, 1, 2), (2, 1, 1), (2, 2, 0)] {
        engine.handle(&message("cancelled", attempt), generation).unwrap();
        assert_eq!(engine.lock().terminating.len(), remaining);
        assert_eq!(engine.fence_left_effects(&run).unwrap(), remaining == 0);
    }
    assert_eq!(wakes.lock().unwrap().iter().filter(|key| key.as_str() == "program-effects").count(), 3);
}

#[test]
fn daemon_failure_retains_its_worker_fence_until_exact_exit_acknowledgement() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "daemon-failure");
    let step = &run.steps[0].subject;
    running_attempt(&engine, &run, 1, 1);
    let child = Command::new("sleep").arg("1000").spawn().unwrap();
    let (lines, receiver) = mpsc::channel();
    engine.lock().host = Some(Host { child, lines, generation: 1, ping: None });
    engine.fail(step, 1, "the daemon refused a program request").unwrap();
    assert_eq!(engine.store.step_run(step).unwrap().unwrap().status, "failed");
    assert!(!engine.lock().running.contains_key(step));
    let cancel = receiver.try_iter().map(|line| serde_json::from_slice::<Value>(&line).unwrap())
        .find(|message| message["t"] == "cancel")
        .expect("daemon failure sends cancellation even when host pings interleave");
    assert_eq!(cancel["t"], "cancel");
    assert_eq!(cancel["step"], step.as_str());
    assert_eq!(cancel["attempt"], 1);
    assert!(!engine.fence_left_effects(&run).unwrap());
    // A late host lifecycle notification cannot lose the daemon's pending proof.
    engine.handle(&json!({"t": "ending", "step": step, "attempt": 1}), 1).unwrap();
    engine.handle(&json!({"t": "cancelled", "step": step, "attempt": 1}), 2).unwrap();
    assert!(!engine.fence_left_effects(&run).unwrap());
    engine.handle(&json!({"t": "cancelled", "step": step, "attempt": 1}), 1).unwrap();
    assert!(engine.fence_left_effects(&run).unwrap());
    engine.clone().host_exited(1);
}

#[test]
fn daemon_refusals_export_failed_attempt_spans() {
    const CHILD: &str = "FRACTAL_PROGRAM_REFUSAL_TRACE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "reconcile::program::tests::daemon_refusals_export_failed_attempt_spans", "--nocapture"])
            .env(CHILD, "1")
            .status().unwrap();
        assert!(status.success(), "isolated refusal trace test failed");
        return;
    }
    let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone()).build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        for kind in ["run", "ask"] {
            let run = scoped_run(&engine, kind);
            let step = &run.steps[0].subject;
            running_attempt(&engine, &run, 1, 1);
            // Both requests have a valid key but lack their required payload.
            engine.handle(&json!({"t": kind, "step": step, "attempt": 1, "key": "invalid", "seq": 1}), 1).unwrap();
            assert_eq!(engine.store.step_run(step).unwrap().unwrap().status, "failed");
            assert!(!engine.lock().running.contains_key(step));
        }
        let run = scoped_run(&engine, "result-refusal");
        let step = &run.steps[0].subject;
        running_attempt(&engine, &run, 1, 1);
        // A conflicting durable result makes the daemon refuse the success journal.
        engine.write_journal(step, 1, &"a".repeat(64), RESULT_KEY, "completed", &json!({"value": 1}), None).unwrap();
        engine.handle(&json!({"t": "done", "step": step, "attempt": 1,
            "exit": {"_tag": "Success", "value": 2}}), 1).unwrap();
        assert_eq!(engine.store.step_run(step).unwrap().unwrap().status, "failed");
        assert!(!engine.lock().running.contains_key(step));
    }
    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    let attempts = spans.iter().filter(|span| span.name == "st.program.attempt").collect::<Vec<_>>();
    assert_eq!(attempts.len(), 3);
    for span in attempts {
        assert!(matches!(span.status, opentelemetry::trace::Status::Error { .. }), "{span:?}");
    }
}

#[test]
fn unrelated_workers_and_effect_groups_do_not_hold_or_receive_a_scoped_fence() {
    use std::os::unix::process::CommandExt as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let unrelated = scoped_run(&engine, "unrelated-worker");
    let replacement = scoped_run(&engine, "independent-worker");
    let step = &unrelated.steps[0].subject;
    running_attempt(&engine, &unrelated, 1, 1);
    engine.handle(&json!({"t": "ending", "step": step, "attempt": 1}), 1).unwrap();
    assert!(engine.end(step, 1, 1, None, false).is_some());
    assert!(engine.fence_left_effects(&replacement).unwrap());
    assert!(!engine.fence_left_effects(&unrelated).unwrap());
    engine.handle(&json!({"t": "cancelled", "step": step, "attempt": 1}), 1).unwrap();
    engine.store.set_step_run_state_for_tests(step, "failed", 1);
    let mut effect = Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let groups = engine.groups_dir();
    std::fs::create_dir_all(&groups).unwrap();
    let path = groups.join(format!("{}.json", effect.id()));
    let record = |start: Option<String>| {
        std::fs::write(&path, json!({"pid": effect.id(), "boot_id": boot_id().unwrap(),
            "start_time": start, "step": step, "attempt": 1}).to_string()).unwrap();
    };
    // Even malformed identity evidence is irrelevant when its owner is unrelated.
    record(None);
    assert!(engine.fence_left_effects(&replacement).unwrap());
    assert!(engine.fence_left_effects(&unrelated).is_err());
    record(start_time(effect.id() as i32));
    assert!(engine.fence_left_effects(&replacement).unwrap());
    assert!(effect.try_wait().unwrap().is_none());
    assert!(engine.fence_left_effects(&unrelated).unwrap());
    let status = effect.wait().unwrap();
    assert_eq!(std::os::unix::process::ExitStatusExt::signal(&status), Some(libc::SIGKILL));
}

#[test]
fn scope_includes_superseded_ancestors_and_descendants_of_historical_generations() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    // Publish both candidates before admission, then adopt the older one explicitly.
    // The root follows head so later publications can exercise supersession ancestry.
    crate::reconcile::tests::apply_source(&engine.store, r#"version 2
mission "worker-scope" state="ready" {
  goal "Keep the original generation."
  concurrent-runs max=16
  step "work" { agentless }
}"#, "historical-first");
    let first = engine.store.mission_spec("worker-scope", None).unwrap().unwrap();
    crate::reconcile::tests::apply_source(&engine.store, r#"version 2
mission "worker-scope" state="ready" {
  goal "Create a distinct adopted generation."
  concurrent-runs max=16
  step "adopted" { agentless }
}"#, "historical-adoption-candidate");
    let child_request = |key: &str| crate::model::MissionRunRequest {
        mission: "worker-scope".into(), revision: None,
        workspace: "/tmp".into(), requester: Some("person/operator".into()),
        mode: None, inputs: BTreeMap::new(), idempotency_key: key.into(),
    };
    let original = engine.store.create_mission_run(&child_request("historical-root")).unwrap();
    let child = engine.store.create_child_mission_run(
        &child_request("historical-child"), &original, &original.steps[0].subject, None,
    ).unwrap();
    let grandchild = engine.store.create_child_mission_run(
        &child_request("historical-grandchild"), &child, &child.steps[0].subject, None,
    ).unwrap();
    let next = first;
    let adopted = engine.store.adopt_mission_revision(&original.id, &next,
        "person/operator", "replace the generation", "historical-adoption").unwrap();
    assert_ne!(original.generation, adopted.generation);
    let scope = engine.store.mission_run_supersession_step_scope(&adopted).unwrap();
    for run in [&original, &adopted, &child, &grandchild] {
        assert!(scope.contains(&run.steps[0].subject), "missing historical owner {}", run.subject);
    }
    crate::reconcile::tests::apply_source(&engine.store, r#"version 2
mission "worker-scope" state="ready" {
  goal "Replace the full ancestry."
  concurrent-runs max=16
  step "newest" { agentless }
}"#, "historical-supersession");
    let source = engine.store.mission_run(&original.id).unwrap().unwrap();
    let replacement = engine.store.mission_run(source.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    crate::reconcile::tests::apply_source(&engine.store, r#"version 2
mission "worker-scope" state="ready" {
  goal "Replace the full ancestry again."
  concurrent-runs max=16
  step "last" { agentless }
}"#, "historical-second-supersession");
    let intermediate = engine.store.mission_run(&replacement.id).unwrap().unwrap();
    let newest = engine.store.mission_run(intermediate.superseded_by.as_deref().unwrap()).unwrap().unwrap();
    let scope = engine.store.mission_run_supersession_step_scope(&newest).unwrap();
    for run in [&original, &adopted, &child, &grandchild, &replacement, &newest] {
        let step = &run.steps[0].subject;
        assert!(scope.contains(step), "missing scoped owner {}", run.subject);
        engine.lock().terminating.insert((1, step.clone(), 1));
        assert!(!engine.fence_left_effects(&newest).unwrap());
        engine.handle(&json!({"t": "cancelled", "step": step, "attempt": 1}), 1).unwrap();
    }
    assert!(engine.fence_left_effects(&newest).unwrap());
}

#[test]
fn host_exit_reaps_before_releasing_only_its_termination_proofs() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "reaped-host");
    let step = &run.steps[0].subject;
    let child = Command::new("sleep").arg("1000").spawn().unwrap();
    let pid = child.id();
    let (lines, _receiver) = mpsc::channel();
    {
        let mut inner = engine.lock();
        inner.host = Some(Host { child, lines, generation: 1, ping: None });
        send_cancel(&mut inner, 1, step, 1, "the run ended");
        send_cancel(&mut inner, 1, step, 2, "the next attempt ended");
        inner.terminating.insert((2, step.clone(), 1));
    }
    engine.clone().host_exited(1);
    assert!(leader_gone(pid as i32));
    assert!(engine.lock().host.is_none());
    assert_eq!(engine.lock().terminating.len(), 1);
    assert!(!engine.fence_left_effects(&run).unwrap());
    engine.handle(&json!({"t": "cancelled", "step": step, "attempt": 1}), 2).unwrap();
    assert!(engine.fence_left_effects(&run).unwrap());
}

#[test]
fn resource_protocol_preserves_native_identity_and_exact_cancellation() {
    let payload = json!({
        "name": "checks", "host": "executor", "coverage": "enforced",
        "capacity": {"cores": 4, "memory": 8589934592_u64},
        "admissionTimeout": "1800000ms"
    });
    for cancelled in [false, true] {
        let message = json!({
            "t": if cancelled { "resource-cancel" } else { "resource" },
            "operation": "resource.lane.acquire", "payload": payload,
        });
        match wait("step-run/generation/check", 2, "resource.lane/checks", &message).unwrap() {
            Wait::Resource { operation, payload: actual, cancelled: actual_cancelled } => {
                assert_eq!(operation, "resource.lane.acquire");
                assert_eq!(actual, payload);
                assert_eq!(actual_cancelled, cancelled);
            }
            other => panic!("resource request changed kind: {other:?}"),
        }
    }
    assert!(wait("step-run/generation/check", 2, "checks", &json!({
        "t": "resource", "operation": "resource.lane.unknown", "payload": {}
    })).is_err());
    assert!(wait("step-run/generation/check", 2, "checks", &json!({
        "t": "resource", "operation": "resource.lane.acquire", "payload": null
    })).is_err());
}

#[test]
fn a_replayed_grant_still_visits_native_authority_and_cancellation_wins_the_reply_race() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "resource-replay");
    running_attempt(&engine, &run, 1, 7);
    let step = &run.steps[0].subject;
    let key = "resource.lane/checks";
    entry(&engine, step, key, "completed", 1, json!({"membershipId":"stale"}));
    let before = journal_claims(&engine, step);
    let message = json!({
        "t":"resource", "step":step, "attempt":1, "seq":10, "key":key,
        "operation":"resource.lane.acquire",
        "payload":{"name":"checks","host":"executor","coverage":"logical"}
    });
    engine.handle(&message, 7).unwrap();
    let requests = engine.requests(&run.subject);
    assert_eq!(requests.len(), 1, "a journaled grant must not satisfy native acquisition");
    assert_eq!(journal_claims(&engine, step), before, "native requests do not journal capabilities");
    let original = requests[0].clone();
    let mut cancellation = message;
    cancellation["t"] = Value::from("resource-cancel");
    engine.handle(&cancellation, 7).unwrap();
    engine.resolve_resource(&original, json!({"_tag":"Success","value":{"membershipId":"racing"}})).unwrap();
    let requests = engine.requests(&run.subject);
    assert_eq!(requests.len(), 1, "racing admission cannot discard the cancellation");
    assert!(matches!(&requests[0].wait, Wait::Resource { cancelled: true, .. }));
    engine.resolve_resource(&requests[0], json!({
        "_tag":"Failure","error":{"_tag":"ResourceError","code":"cancelled","message":"withdrawn"}
    })).unwrap();
    assert!(engine.requests(&run.subject).is_empty());
    assert_eq!(journal_claims(&engine, step), before, "neither acquisition nor withdrawal overwrites the journal");
}

#[test]
fn launch_freeze_defers_resource_exec_and_clear_invalidates_the_pending_effect() {
    frozen_resource_exec(false);
}

#[test]
fn launch_freeze_drain_timeout_wakes_resource_exec_without_a_timer() {
    frozen_resource_exec(true);
}

fn frozen_resource_exec(timed_out_drain: bool) {
    use crate::reconcile::{NativeRuntime, Reconciler};
    use crate::store::resource_membership::{Admission, Coverage, Fit, LaneRequest, Owner};
    use fractal_runtime::Capacity;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "frozen-resource-exec");
    running_attempt(&engine, &run, 1, 1);
    let child = Command::new("sleep").arg("1000").spawn().unwrap();
    let (lines, _receiver) = mpsc::channel();
    engine.lock().host = Some(Host { child, lines, generation: 1, ping: None });
    let runtime = Arc::new(NativeRuntime::new(dir.path(), None, Path::new("unused-pty")));
    let reconciler = Reconciler::new(engine.store.clone(), runtime, "node".into(), Arc::new(tokio::sync::Notify::new()));
    reconciler.ensure_resource_authority().unwrap();
    let lane = LaneRequest {
        id: "freeze-resource-lane".into(), name: "checks".into(), host: "node".into(),
        max_members: 1, capacity: Some(Capacity { cores: 1, memory: 256 << 20 }),
        coverage: Coverage::Logical, admission_timeout_ms: None,
        class: Default::default(), disk: None, io: None,
        owner: Owner { run: run.subject.clone(), step: run.steps[0].subject.clone(),
            attempt: 1, invocation: Some("program:checks".into()) },
    };
    let fit = Fit { room: Some(Capacity { cores: 32, memory: 128 << 30 }), ..Fit::default() };
    let Admission::Admitted(grant) = engine.store.request_resource_membership(&lane, fit, 1).unwrap() else {
        panic!("logical membership must be admitted");
    };
    let marker = dir.path().join("executions");
    let payload = json!({"membership":grant, "argv":["/bin/sh","-c","printf x >> \"$1\"","sh",marker],
        "options":{"cwd":dir.path()}});
    engine.handle(&json!({"t":"resource","step":run.steps[0].subject,"attempt":1,"seq":1,
        "key":"resource.exec/check","operation":"resource.lane.exec","payload":payload}), 1).unwrap();
    let request = engine.requests(&run.subject)[0].clone();
    let prior_permit = timed_out_drain.then(|| engine.store.admit_launch().unwrap().unwrap());
    let setter = if timed_out_drain {
        let store = engine.store.clone();
        let setter = std::thread::spawn(move ||
            store.set_launch_freeze("person/operator", "activation", "replace daemon", 60_000, 2_000));
        let deadline = Instant::now() + Duration::from_secs(1);
        while engine.store.admit_launch().unwrap().is_some() {
            assert!(Instant::now() < deadline, "set must enter its drain");
            std::thread::yield_now();
        }
        Some(setter)
    } else {
        engine.store.set_launch_freeze("person/operator", "activation", "replace daemon", 60_000, 5_000).unwrap();
        None
    };
    let expiry = engine.store.launch_freeze().unwrap().map(|freeze| u128::from(freeze.expires_at_unix_ms));
    reconciler.incremental.observe(&engine.store).unwrap();
    let ((changed, due), reads) = smallclaims::touched::record(|| smallclaims::touched::record_due(||
        reconciler.settle_program_resource(&engine, &run, &request, "resource.lane.exec", &payload, false).unwrap()));
    assert!(!changed);
    assert_eq!(due, expiry);
    assert!(!marker.exists());
    assert!(engine.store.resource_membership_record(&lane.id, &lane.owner).unwrap().roots.is_empty());
    assert_eq!(engine.requests(&run.subject).len(), 1, "the effect must remain pending");
    reconciler.incremental.evaluated("pending-resource-effect", reads, due);
    assert!(!reconciler.incremental.needs("pending-resource-effect", 0));
    if let Some(setter) = setter {
        assert_eq!(setter.join().unwrap().unwrap_err().code, "launch-freeze-timeout");
        assert!(engine.store.launch_freeze().unwrap().is_none());
        drop(prior_permit);
    } else {
        engine.store.clear_launch_freeze("person/operator", "activation").unwrap();
    }
    reconciler.incremental.observe(&engine.store).unwrap();
    assert!(reconciler.incremental.needs("pending-resource-effect", 0), "clear must wake the cached run, not just the daemon");
    assert!(reconciler.settle_program_resource(&engine, &run, &request, "resource.lane.exec", &payload, false).unwrap());
    let roots = engine.store.resource_membership_record(&lane.id, &lane.owner).unwrap().roots;
    assert_eq!(roots.len(), 1, "the deferred effect must allocate exactly one root");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(10)); }
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    reconciler.settle_program_resource(&engine, &run, &request, "resource.lane.exec", &payload, false).unwrap();
    assert_eq!(engine.store.resource_membership_record(&lane.id, &lane.owner).unwrap().roots.len(), 1);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x", "replay must not spawn a duplicate");
    let mut host = engine.lock().host.take().unwrap();
    let _ = host.child.kill();
    host.child.wait().unwrap();
}

#[test]
fn await_pr_protocol_validates_repo_number_and_head() {
    let valid = json!({"t":"await-pr","repo":"Acme/Garden","pr":12,"head":"pinned"});
    assert!(matches!(wait("step-run/generation/work", 1, "await-outcome", &valid).unwrap(),
        Wait::AwaitPr { repo, pr: 12, head } if repo == "acme/garden" && head == "pinned"));
    for (name, invalid) in [
        ("repo", json!("acme/garden/extra")), ("repo", json!("acme/garden#12")),
        ("pr", json!(0)), ("pr", json!(1.5)), ("pr", json!(9_007_199_254_740_992_u64)),
        ("head", json!(" ")),
    ] {
        let mut message = valid.clone();
        message[name] = invalid;
        assert!(wait("step-run/generation/work", 1, "await-outcome", &message).is_err(), "{message}");
    }
}

#[test]
fn completed_await_pr_replay_never_registers_a_watch() {
    use crate::watches::ProgramPrWatchesStore as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "await-pr-replay");
    running_attempt(&engine, &run, 1, 7);
    let step = &run.steps[0].subject;
    entry(&engine, step, "await-outcome", "completed", 1,
        json!({"outcome":"merged","mergeCommit":"merge"}));
    let before = journal_claims(&engine, step);
    engine.handle(&json!({
        "t":"await-pr","step":step,"attempt":1,"seq":10,"key":"await-outcome",
        "repo":"acme/garden","pr":12,"head":"head",
    }), 7).unwrap();
    assert!(engine.requests(&run.subject).is_empty());
    assert!(engine.store.program_pr_watches().unwrap().is_empty());
    assert_eq!(journal_claims(&engine, step), before);
}

#[test]
fn resumed_await_pr_adopts_registration_and_releases_it_on_completion_or_attempt_departure() {
    use crate::watches::ProgramPrWatchesStore as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "await-pr-adoption");
    running_attempt(&engine, &run, 1, 7);
    let step = &run.steps[0].subject;
    let message = json!({
        "t":"await-pr","step":step,"attempt":1,"seq":10,"key":"await-outcome",
        "repo":"acme/garden","pr":12,"head":"head",
    });
    engine.handle(&message, 7).unwrap();
    let request = engine.requests(&run.subject).pop().unwrap();
    let subject = await_pr::subject(&request);
    let original = engine.register_pr(&request, &subject, json!({"repo":"acme/garden","pr":12,"head":"head"})).unwrap();
    assert_eq!(engine.store.program_pr_watches().unwrap(), vec![original.clone()]);
    assert!(engine.journal(step, 1).unwrap().is_empty(), "registration is not an outcome");
    running_attempt(&engine, &run, 1, 8);
    engine.handle(&message, 8).unwrap();
    let resumed = engine.requests(&run.subject).pop().unwrap();
    assert_eq!(await_pr::subject(&resumed), subject);
    let before = journal_claims(&engine, step);
    assert_eq!(engine.register_pr(&resumed, &subject, json!({"repo":"acme/garden","pr":12,"head":"head"})).unwrap(), original);
    assert_eq!(journal_claims(&engine, step), before, "adoption does not reset the time baseline");
    engine.resolve(&resumed, &subject, json!({"_tag":"Success","value":{"outcome":"merged","mergeCommit":"merge"}})).unwrap();
    assert!(engine.store.program_pr_watches().unwrap().is_empty());

    let mut second = message;
    second["key"] = json!("await-outcome#2");
    engine.handle(&second, 8).unwrap();
    let request = engine.requests(&run.subject).pop().unwrap();
    let subject = await_pr::subject(&request);
    engine.register_pr(&request, &subject, json!({"repo":"acme/garden","pr":12,"head":"head"})).unwrap();
    assert_eq!(engine.store.program_pr_watches().unwrap().len(), 1);
    engine.store.set_step_run_state_for_tests(step, "working", 2);
    assert!(engine.store.program_pr_watches().unwrap().is_empty(), "departed attempts retain no observer");
    engine.cancel_waits(step, 1, "attempt superseded").unwrap();
}

#[test]
fn native_pr_registration_retains_the_standing_observer_without_a_seat_subscription() {
    use crate::reconcile::{NativeRuntime, Reconciler};
    use crate::watches::WatchesStore as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "await-pr-observer");
    running_attempt(&engine, &run, 1, 7);
    engine.handle(&json!({
        "t":"await-pr","step":run.steps[0].subject,"attempt":1,"seq":1,"key":"await-outcome",
        "repo":"acme/garden","pr":12,"head":"head",
    }), 7).unwrap();
    let request = engine.requests(&run.subject).pop().unwrap();
    engine.register_pr(&request, &await_pr::subject(&request), json!({"repo":"acme/garden","pr":12,"head":"head"})).unwrap();
    let runtime = Arc::new(NativeRuntime::new(dir.path(), None, Path::new("unused-pty")));
    let reconciler = Reconciler::new(engine.store.clone(), runtime, "node".into(), Arc::new(tokio::sync::Notify::new()));
    reconciler.reconcile_github_watches(&engine.store.github_watch_declarations().unwrap()).unwrap();
    let declarations = engine.store.github_watch_declarations().unwrap();
    assert!(declarations.iter().all(|desired| desired.kind != "subscription"), "a native wait is not a seat watch");
    let observer = declarations.iter().find(|desired| desired.subject == "observer/github/acme/garden").unwrap();
    assert!(!crate::graph::observer_spec(&observer.desired).unwrap().stopped);
    engine.store.set_step_run_state_for_tests(&request.step, "cancelled", 1);
    reconciler.reconcile_github_watches(&declarations).unwrap();
    let declarations = engine.store.github_watch_declarations().unwrap();
    let observer = declarations.iter().find(|desired| desired.subject == "observer/github/acme/garden").unwrap();
    assert!(crate::graph::observer_spec(&observer.desired).unwrap().stopped);
}

fn observe_program_pr(engine: &ProgramEngine, mut pulls: Value) {
    use crate::watches::ProgramPrWatchesStore as _;
    let targets = await_pr::targets(engine.store.program_pr_watches().unwrap(), "acme/garden").unwrap();
    for pull in pulls.as_array_mut().unwrap() {
        if let Some(target) = targets.iter().find(|target| pull["number"].as_u64() == Some(target.pr)) {
            pull["await_pr_observation"] = json!({
                "registrations":target.registrations,"head":pull["head_sha"],
                "merged":pull["merged"],"merge_commit_sha":pull["merge_commit_sha"],
            });
        }
    }
    observe_program_pr_facts(engine, &json!({"repository_id":7,"pull_requests":pulls,"await_prs_not_found":[]}));
}

fn observe_program_pr_facts(engine: &ProgramEngine, facts: &Value) {
    use crate::watches::WatchesStore as _;
    let thread = crate::github_watch::ThreadRef::parse("acme/garden#12").unwrap();
    engine.store.ensure_watch_observer(&thread).unwrap();
    let revision = engine.store.selected_desired_revision(&thread.observer()).unwrap().unwrap();
    engine.store.record_resource_observation(
        &thread.observer(), &revision, None, &thread.resource(), None,
        facts,
        smallclaims::store::now_ms() + 60_000, &[], None,
    ).unwrap();
}

fn record_program_repo_missing(engine: &ProgramEngine, evidence: Vec<String>) {
    use crate::watches::WatchesStore as _;
    let thread = crate::github_watch::ThreadRef::parse("acme/garden#12").unwrap();
    engine.store.ensure_watch_observer(&thread).unwrap();
    engine.store.append_claim(&ClaimInput {
        subject: thread.observer(), kind: "observer.state".into(), actor: None,
        fields: BTreeMap::from([
            ("state".into(), json!("unreachable")),
            ("error_code".into(), json!("github-repository-not-found")),
            ("reason".into(), json!("GitHub repository returned HTTP 404")),
        ]),
        evidence, expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[test]
fn native_await_pr_resolves_observed_merge_and_journals_the_caller_key() {
    use crate::watches::ProgramPrWatchesStore as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "await-pr-merge");
    running_attempt(&engine, &run, 1, 7);
    observe_program_pr(&engine, json!([{
        "number":12,"state":"open","head_sha":"head","head":"head","merged":false,
    }]));
    engine.handle(&json!({
        "t":"await-pr","step":run.steps[0].subject,"attempt":1,"seq":1,"key":"await-outcome",
        "repo":"acme/garden","pr":12,"head":"head",
    }), 7).unwrap();
    let request = engine.requests(&run.subject).pop().unwrap();
    let (pending, reads) = smallclaims::touched::record(|| await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head"));
    assert!(!pending.unwrap());
    assert!(reads.contains("resource/github/acme/garden/pull-request/12"));
    assert_eq!(engine.store.program_pr_watches().unwrap().len(), 1);
    observe_program_pr(&engine, json!([{
        "number":12,"state":"closed","head_sha":"head","head":"head","merged":true,"merge_commit_sha":"merge",
    }]));
    assert!(await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head").unwrap());
    assert_eq!(engine.journal(&request.step, 1).unwrap(), vec![json!({
        "key":"await-outcome","_tag":"Success","value":{"outcome":"merged","mergeCommit":"merge"},
    })]);
    assert!(engine.store.program_pr_watches().unwrap().is_empty());
}

#[test]
fn native_await_pr_returns_typed_unknown_repository_and_pull_request_errors() {
    use crate::watches::ProgramPrWatchesStore as _;
    for known_repo in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let run = scoped_run(&engine, if known_repo { "unknown-pr" } else { "unknown-repo" });
        running_attempt(&engine, &run, 1, 7);
        if known_repo {
            observe_program_pr(&engine, json!([]));
        }
        if !known_repo {
            record_program_repo_missing(&engine, Vec::new());
        }
        engine.handle(&json!({
            "t":"await-pr","step":run.steps[0].subject,"attempt":1,"seq":1,"key":"await-outcome",
            "repo":"acme/garden","pr":12,"head":"head",
        }), 7).unwrap();
        let request = engine.requests(&run.subject).pop().unwrap();
        let (pending, reads) = smallclaims::touched::record(|| await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head"));
        assert!(!pending.unwrap(), "pre-registration failures and incremental absence cannot settle a new wait");
        assert!(reads.contains("resource/github/acme/garden"));
        assert!(reads.contains("observer/github/acme/garden"));
        let registrations = engine.store.program_pr_watches().unwrap();
        let token = registrations[0]["registration"].as_str().unwrap();
        if known_repo {
            observe_program_pr_facts(&engine, &json!({
                "repository_id":7,"pull_requests":[],
                "await_prs_not_found":[{"pr":12,"registrations":[token]}],
            }));
        } else {
            record_program_repo_missing(&engine, vec![token.to_owned()]);
        }
        assert!(await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head").unwrap());
        let journal = engine.journal(&request.step, 1).unwrap();
        assert_eq!(journal[0]["_tag"], "Failure");
        assert_eq!(journal[0]["error"]["_tag"], "AwaitPrFailed");
        assert_eq!(journal[0]["error"]["code"], if known_repo { "unknown-pr" } else { "unknown-repo" });
        assert_eq!(journal[0]["error"]["repo"], "acme/garden");
        assert_eq!(journal[0]["error"]["pr"], 12);
    }
}

#[test]
fn native_await_pr_resolves_only_observed_queue_exit_or_changed_head() {
    struct ResetClock;
    impl Drop for ResetClock {
        fn drop(&mut self) { smallclaims::store::set_thread_clock(None); }
    }
    for moved in [false, true] {
        smallclaims::store::set_thread_clock(Some(1_900_000_000_000));
        let _reset = ResetClock;
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let run = scoped_run(&engine, if moved { "head-moved" } else { "queue-exit" });
        running_attempt(&engine, &run, 1, 7);
        observe_program_pr(&engine, json!([{
            "number":12,"state":"open","head_sha":"head","head":"head","merged":false,"merge_queue":null,
        }]));
        engine.handle(&json!({
            "t":"await-pr","step":run.steps[0].subject,"attempt":1,"seq":1,"key":"await-outcome",
            "repo":"acme/garden","pr":12,"head":"head",
        }), 7).unwrap();
        let request = engine.requests(&run.subject).pop().unwrap();
        assert!(!await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head").unwrap(),
            "not yet queued is not a dequeue");
        smallclaims::store::set_thread_clock(Some(1_900_000_001_000));
        observe_program_pr(&engine, json!([{
            "number":12,"state":"open","head_sha":"head","head":"head","merged":false,
            "merge_queue":{"state":"queued","position":1},
        }]));
        assert!(!await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head").unwrap());
        smallclaims::store::set_thread_clock(Some(1_900_000_002_000));
        observe_program_pr(&engine, json!([{
            "number":12,"state":"open","head_sha":if moved { "new" } else { "head" },
            "head":if moved { "new" } else { "head" },"merged":false,"merge_queue":null,
        }]));
        assert!(await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "head").unwrap());
        let journal = engine.journal(&request.step, 1).unwrap();
        assert_eq!(journal[0]["value"], json!({
            "outcome":"dequeued","reason":if moved { "head-changed" } else { "merge-queue-exit" },
        }));
    }
}

#[test]
fn a_new_wait_does_not_treat_an_older_observed_head_as_head_movement() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let run = scoped_run(&engine, "await-pr-stale-head");
    running_attempt(&engine, &run, 1, 7);
    observe_program_pr_facts(&engine, &json!({
        "repository_id":7,"await_prs_not_found":[],"pull_requests":[{
            "number":12,"state":"open","head_sha":"old","head":"old","merged":false,
            "await_pr_observation":{
                "registrations":["another-registration"],"head":"old","merged":false,"merge_commit_sha":null,
            },
        }],
    }));
    engine.handle(&json!({
        "t":"await-pr","step":run.steps[0].subject,"attempt":1,"seq":1,"key":"await-outcome",
        "repo":"acme/garden","pr":12,"head":"requested",
    }), 7).unwrap();
    let request = engine.requests(&run.subject).pop().unwrap();
    assert!(!await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "requested").unwrap(),
        "the standing observer's pre-registration head is not movement");
    observe_program_pr(&engine, json!([{
        "number":12,"state":"open","head_sha":"requested","head":"requested","merged":false,
    }]));
    assert!(!await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "requested").unwrap(),
        "a fresh acquisition of the requested head keeps waiting");
    assert!(engine.journal(&request.step, 1).unwrap().is_empty());
    observe_program_pr(&engine, json!([{
        "number":12,"state":"open","head_sha":"later","head":"later","merged":false,
    }]));
    assert!(await_pr::settle(&engine.store, &engine, &request, "acme/garden", 12, "requested").unwrap());
    assert_eq!(engine.journal(&request.step, 1).unwrap()[0]["value"],
        json!({"outcome":"dequeued","reason":"head-changed"}));
}
