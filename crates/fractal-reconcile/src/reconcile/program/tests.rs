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
    engine.lock().running.insert(step.into(), Running { dispatch, generation: 1, requests: BTreeMap::new() });
    let message = |attempt: u32, t: &str| {
        json!({"t": t, "step": step, "attempt": attempt, "seq": 1, "key": "merge", "status": "completed", "value": 1,
            "exit": {"_tag": "Success", "value": 1}, "ms": 5, "mission": "publish", "input": {}, "for": "person/owner", "title": "Go?"})
    };
    for t in ["journal", "run", "ask", "done", "unresponsive"] {
        // An earlier attempt of the step, then the right attempt from a replaced host.
        engine.handle(&message(1, t), 1).unwrap();
        engine.handle(&message(2, t), 2).unwrap();
    }
    assert_eq!(journal_claims(&engine, step), 0);
    assert!(engine.lock().running.get(step).is_some_and(|running| running.requests.is_empty()));
    assert_eq!(engine.host_fault(step, 2), None);
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
    engine.lock().running.insert(step.subject.clone(), Running { dispatch: dispatch(1), generation: 1, requests: BTreeMap::new() });
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
    let groups = engine.groups_dir();
    std::fs::create_dir_all(&groups).unwrap();
    let mut effect = Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let mut reused = Command::new("sleep").arg("1000").process_group(0).spawn().unwrap();
    let record = |child: &Child, start: Option<String>| {
        std::fs::write(groups.join(format!("{}.json", child.id())), json!({
            "pid": child.id(), "boot_id": boot_id().unwrap(), "start_time": start,
            "step": "step-run/retired-generation/effect", "attempt": 1,
        }).to_string()).unwrap();
    };
    // An unconfirmed identity is a visible error, never permission to run the
    // replacement or to signal a process whose identity we cannot prove.
    record(&effect, None);
    assert!(engine.fence_left_effects().is_err());
    assert!(effect.try_wait().unwrap().is_none());
    record(&effect, start_time(effect.id() as i32));
    record(&reused, Some("not-this-process".into()));
    assert!(engine.fence_left_effects().unwrap());
    let status = effect.wait().unwrap();
    assert_eq!(std::os::unix::process::ExitStatusExt::signal(&status), Some(libc::SIGKILL));
    assert!(reused.try_wait().unwrap().is_none());
    reused.kill().unwrap();
    reused.wait().unwrap();
}
