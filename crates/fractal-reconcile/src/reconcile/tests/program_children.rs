//! Linked child runs and person asks of program steps, with a real Bun host
//! (docs/st3/program-engine.md §6, §7): adoption on resume, the capacity wait and its reactive
//! wake, cancellation, retries, asks answered, defaulted and withdrawn, and the exit shapes the
//! `fractal/effect` program API decodes. These tests fail, never skip, when no `bun` is on `PATH`.
use super::*;
use crate::model::{AnswerInput, PROGRAM_ASKER, PersonStepResponse};
use crate::reconcile::program::{JOURNAL_KIND, Liveness, ProgramEngine, RESULT_KEY, Wait};
use serde_json::json;
use std::time::Instant;

/// A hand-written bundle against HostContext v1. Mode `child` starts one linked child run, mode
/// `ask` opens three concurrent asks; then both wait for a gate file in a journaled step, so a
/// test can replace the host after the waits resolved.
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
const wait = (ctx, key, request) => ctx.journal.get(key) ?? request()
const gate = (ctx) =>
  step(ctx, 'gate', async () => {
    const out = await ctx.spawn(['sh', '-c', 'while [ ! -e "$GATE" ]; do sleep 0.05; done; echo opened'], { env: { GATE: ctx.payload.gate } })
    return out.stdout.trim()
  })
const DECISION = {
  version: 1, type: 'decision', question: 'Ship it?', why_person: 'The owner decides releases.',
  answers: [
    { id: 'ship', label: 'Ship', outcome: 'accept', consequence: 'It ships.' },
    { id: 'hold', label: 'Hold', outcome: 'decline', consequence: 'Nothing ships.' },
  ],
}
export default {
  [Symbol.for('fractal.program.v1')]: async (ctx) => {
    if (ctx.payload.mode.startsWith('turn')) {
      const resource = { key: 'turn', operation: 'agent.turn', payload: {
        agent: ctx.payload.mode === 'turn-remote' ? 'agent/peer.remote' : 'agent/node.publisher', instructions: 'Produce ${literal} without interpolation.',
        context: '{"input":"${ST_MISSION_RUN}"}',
        products: [{ path: 'answer', kind: 'custom.st3.program-test', fields: { head: 'h1' } }],
      } }
      if (ctx.payload.mode === 'turn-cancel') {
        const controller = new AbortController()
        const pending = ctx.resource(resource, controller.signal)
        await gate(ctx)
        controller.abort()
        return await pending
      }
      return await ctx.resource(resource)
    }
    if (ctx.payload.mode === 'child') {
      const published = await wait(ctx, 'publish', () => ctx.child({ key: 'publish', mission: 'program-child', input: { head: 'h1' } }))
      if (published._tag === 'Failure') return published
      return { _tag: 'Success', value: { published: published.value, gate: await gate(ctx) } }
    }
    if (ctx.payload.mode === 'uncaught-ask') {
      void wait(ctx, 'still-open', () => ctx.ask({ key: 'still-open', for: 'person/owner', title: 'Another question', request: null }))
      const exit = await wait(ctx, 'withdrawn', () => ctx.ask({ key: 'withdrawn', for: 'person/owner', title: 'Still needed?', request: null }))
      return exit
    }
    const asks = await Promise.all([
      wait(ctx, 'approve', () => ctx.ask({ key: 'approve', for: 'person/owner', title: 'Ship it?', request: DECISION, deadline: '2m', default: 'ship' })),
      wait(ctx, 'note', () => ctx.ask({ key: 'note', for: 'person/owner', title: 'Anything to add?', request: null })),
      wait(ctx, 'withdrawn', () => ctx.ask({ key: 'withdrawn', for: 'person/owner', title: 'Still needed?', request: null })),
    ])
    return { _tag: 'Success', value: { asks, gate: await gate(ctx) } }
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
    gate: std::path::PathBuf,
}

/// One journal entry: key, status, attempt, child.
type Entry = (String, String, u64, Option<String>);

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_memory("node").unwrap());
    store.set_ask_deadlines(true);
    let bundle = dir.path().join("program.mjs");
    std::fs::write(&bundle, BUNDLE).unwrap();
    let hash = store.put_blob(BUNDLE.as_bytes()).unwrap();
    apply_source(
        &store,
        &format!(
            r#"version 2
agent "publisher" {{ workspace "/tmp"; command "true"; restart "never" }}
agent "other" {{ workspace "/tmp"; command "true"; restart "never" }}
agent "remote" {{ host "peer"; workspace "/tmp"; command "true"; restart "never" }}
mission "program-child" state="ready" {{
  goal "Publish one head at a time."
  concurrent-runs max=1
  input "head" kind="text"
  step "publish" {{
    assigned-to "agent/publisher"
    produces {{ resource "mission-run/${{ST_MISSION_RUN}}/published" {{ kind "custom.st3.program-test" }} }}
  }}
}}
mission "program-parent" state="ready" {{
  goal "Wait on child runs and asks."
  concurrent-runs max=4
  authority {{ start "program-child"; }}
  input "mode" kind="text"
  input "gate" kind="text"
  step "program" {{
    retry {{ attempts 2 }}
    program "{0}" sha256="{hash}"
  }}
}}
mission "program-uncaught-ask" state="ready" {{
  goal "Fail an uncaught ask cancellation and run cleanup."
  input "mode" kind="text"
  input "gate" kind="text"
  step "program" {{ program "{0}" sha256="{hash}" }}
  finally {{ step "cleanup" {{ agentless }} }}
}}
mission "program-unauthorized" state="ready" {{
  goal "Start a child run it holds no authority to start."
  input "mode" kind="text"
  input "gate" kind="text"
  step "program" {{
    program "{0}" sha256="{hash}"
  }}
}}"#,
            bundle.display()
        ),
        "program-children-definition",
    );
    let reconciler = Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), "node".into(), Arc::new(Notify::new()));
    // As the daemon's engine does: a change off the reconciler thread marks the step for the next
    // pass. Nothing signals a follow-reads evaluation: tests call that themselves.
    let incremental = reconciler.incremental.clone();
    let wake = Arc::new(move |step: &str| incremental.touch(step));
    let engine = ProgramEngine::new(store.clone(), wake, dir.path().join("programs"), Liveness::default());
    assert!(reconciler.program_engine.set(engine.clone()).is_ok());
    Fixture { store, reconciler, engine, dir }
}

impl Fixture {
    fn start(&self, name: &str, mode: &str) -> Started {
        self.start_mission("program-parent", name, mode)
    }

    fn start_mission(&self, mission: &str, name: &str, mode: &str) -> Started {
        let gate = self.dir.path().join(format!("{name}.gate"));
        let inputs = BTreeMap::from([("mode".to_owned(), mode.to_owned()), ("gate".to_owned(), gate.display().to_string())]);
        let run = self
            .store
            .create_mission_run(&MissionRunRequest {
                mission: mission.into(),
                revision: None,
                workspace: self.dir.path().display().to_string(),
                requester: Some("person/owner".into()),
                mode: None,
                inputs,
                idempotency_key: format!("program-parent-{name}"),
            })
            .unwrap();
        let step = run.steps.iter().find(|step| step.step == "program").unwrap().subject.clone();
        Started { run: run.subject, step, gate }
    }

    /// A run of the child mission that nothing links to a program.
    fn holder(&self) -> String {
        self.store
            .create_mission_run(&MissionRunRequest {
                mission: "program-child".into(),
                revision: None,
                workspace: self.dir.path().display().to_string(),
                requester: Some("person/owner".into()),
                mode: None,
                inputs: BTreeMap::from([("head".to_owned(), "h0".to_owned())]),
                idempotency_key: "program-child-holder".into(),
            })
            .unwrap()
            .subject
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

    fn status(&self, step: &str) -> String {
        self.store.step_run(step).unwrap().unwrap().status
    }

    fn run_status(&self, run: &str) -> String {
        self.store.mission_run_status(run).unwrap().unwrap()
    }

    fn journal(&self, step: &str) -> Vec<Entry> {
        let mut claims = self.store.claims_for(step, Some(JOURNAL_KIND)).unwrap();
        claims.sort_by_key(|claim| claim.store_index);
        claims
            .iter()
            .map(|claim| {
                let fields = &claim.body["fields"];
                (
                    fields["key"].as_str().unwrap().to_owned(),
                    fields["status"].as_str().unwrap().to_owned(),
                    fields["attempt"].as_u64().unwrap(),
                    fields["child"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    /// The result journaled for `key` at `attempt`: `(status, value or error)`.
    fn result(&self, step: &str, key: &str, attempt: u64) -> Option<(String, Value)> {
        self.store.claims_for(step, Some(JOURNAL_KIND)).unwrap().into_iter().find_map(|claim| {
            let fields = &claim.body["fields"];
            let status = fields["status"].as_str()?;
            (fields["key"] == key && fields["attempt"].as_u64() == Some(attempt) && status != "started").then(|| {
                let result = if status == "failed" { fields["error"].clone() } else { fields["value"].clone() };
                (status.to_owned(), result)
            })
        })
    }

    /// The child or ask subject the program journaled for `key` at `attempt`.
    fn child(&self, step: &str, key: &str, attempt: u64) -> Option<String> {
        self.journal(step)
            .into_iter()
            .find_map(|(entry, status, at, child)| (entry == key && status == "started" && at == attempt).then_some(child).flatten())
    }

    /// The program's child runs, by the step the program runs as.
    fn children(&self, step: &str) -> Vec<MissionRunView> {
        self.store.mission_run_children_for_step(step).unwrap()
    }

    /// Complete a child run's only step, after writing its declared product.
    fn complete_child(&self, child: &str) {
        let id = child.strip_prefix("mission-run/").unwrap();
        self.store
            .append_claim(&ClaimInput {
                subject: format!("resource/mission-run/{id}/published"),
                kind: "resource.observed".into(),
                actor: Some("agent/publisher".into()),
                fields: BTreeMap::from([
                    ("kind".into(), json!("custom.st3.program-test")),
                    ("head".into(), json!("h1")),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        let step = self.store.mission_run(child).unwrap().unwrap().steps.remove(0).subject;
        self.store.set_step_state(&step, "completed", None).unwrap();
    }

    fn kill_host(&self) -> u32 {
        let host = self.engine.host_pid().unwrap();
        // SAFETY: the host is this test's own child process.
        assert_eq!(unsafe { libc::kill(host as i32, libc::SIGKILL) }, 0);
        host
    }
}

fn count(journal: &[Entry], key: &str, status: &str) -> usize {
    journal.iter().filter(|(entry, at, _, _)| entry == key && at == status).count()
}

#[test]
fn a_child_run_is_adopted_on_resume_and_its_result_is_journaled_and_replayed() {
    let fixture = fixture();
    let parent = fixture.start("adopt", "child");
    fixture.until("the child run starts", || fixture.child(&parent.step, "publish", 1).is_some());
    let child = fixture.child(&parent.step, "publish", 1).unwrap();
    // The child is linked to the program step under the key's idempotency key.
    assert_eq!(fixture.store.mission_run_subject_for_idempotency_key(&format!("program:{}:1:publish", parent.step)), child);
    assert_eq!(fixture.children(&parent.step).iter().map(|run| run.subject.clone()).collect::<Vec<_>>(), std::slice::from_ref(&child));
    // A new host resumes the program, which asks for the same key again: the daemon adopts the
    // child it started instead of starting another.
    let host = fixture.kill_host();
    fixture.until("the resumed program waits on the child again", || {
        fixture.engine.host_pid().is_some_and(|pid| pid != host) && count(&fixture.journal(&parent.step), "publish", "started") == 3
    });
    fixture.reconciler.reconcile_once().unwrap();
    assert_eq!(fixture.children(&parent.step).len(), 1);
    fixture.complete_child(&child);
    fixture.until("the child's end is journaled", || fixture.result(&parent.step, "publish", 1).is_some());
    let completed = json!({
        "run": child.as_str(),
        "status": "completed",
        "executions": [],
        "products": {
            (format!("resource/{child}/published")): {"kind": "custom.st3.program-test", "head": "h1"},
        },
    });
    assert_eq!(fixture.result(&parent.step, "publish", 1), Some(("completed".to_owned(), completed.clone())));
    fixture.until("the program waits on its gate", || count(&fixture.journal(&parent.step), "gate", "started") == 1);
    // A host replaced now replays the child's result from the journal: no new request.
    let before = count(&fixture.journal(&parent.step), "publish", "started");
    let host = fixture.kill_host();
    fixture.until("the program waits on its gate again", || {
        fixture.engine.host_pid().is_some_and(|pid| pid != host) && count(&fixture.journal(&parent.step), "gate", "started") == 2
    });
    std::fs::write(&parent.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&parent.step) == "completed");
    let journal = fixture.journal(&parent.step);
    assert_eq!(count(&journal, "publish", "started"), before);
    assert_eq!(count(&journal, "publish", "completed"), 1);
    assert_eq!(fixture.children(&parent.step).len(), 1);
    assert_eq!(
        fixture.result(&parent.step, RESULT_KEY, 1),
        Some(("completed".to_owned(), json!({"published": completed, "gate": "opened"})))
    );
}

/// The parent run starts its program's children: the creation is written as the parent, and a
/// mission whose authority lacks `start` on the child's mission fails its step with the refusal
/// (docs/st3/program-engine.md §3).
#[test]
fn a_programs_child_is_started_by_its_parent_run_within_its_authority() {
    let fixture = fixture();
    // Before any child holds `program-child`'s one slot, so the refusal is the authority's.
    let denied = fixture.start_mission("program-unauthorized", "denied", "child");
    fixture.until("the unauthorized program's step fails", || fixture.status(&denied.step) == "failed");
    assert!(fixture.children(&denied.step).is_empty());
    let reason = fixture.store.step_run(&denied.step).unwrap().unwrap().blocked_reason.unwrap_or_default();
    assert!(reason.contains("could not start its child run `publish`"), "{reason}");

    let parent = fixture.start("starter", "child");
    fixture.until("the child run starts", || fixture.child(&parent.step, "publish", 1).is_some());
    let child = fixture.child(&parent.step, "publish", 1).unwrap();
    let created = fixture.store.claims_for(&child, Some("mission-run.created")).unwrap();
    assert_eq!(created[0].actor.as_deref(), Some(parent.run.as_str()));
    assert_eq!(fixture.store.mission_run(&child).unwrap().unwrap().requester, "person/owner");
}

#[test]
fn a_child_of_a_full_mission_waits_and_starts_when_the_holder_ends() {
    let fixture = fixture();
    let holder = fixture.holder();
    let parent = fixture.start("capacity", "child");
    fixture.until("the program asks for its child", || !fixture.engine.requests(&parent.run).is_empty());
    // An evaluation that finds the mission full notes the runs that hold it.
    fixture.reconciler.reconcile_once().unwrap();
    assert!(fixture.children(&parent.step).is_empty());
    assert_eq!(fixture.child(&parent.step, "publish", 1), None);
    assert_eq!(fixture.status(&parent.step), "working");
    // The holder's end alone, with no pass, starts the child: `concurrent-runs max=1` serializes
    // the section, and the waiting start runs next.
    fixture.store.request_mission_run_cancellation(&holder, "make room").unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    let child = fixture.child(&parent.step, "publish", 1).expect("the child starts once the holder ended");
    assert_eq!(fixture.children(&parent.step).iter().map(|run| run.subject.clone()).collect::<Vec<_>>(), std::slice::from_ref(&child));
    fixture.complete_child(&child);
    std::fs::write(&parent.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&parent.step) == "completed");
}

#[test]
fn a_child_started_after_its_step_left_is_journaled_and_cancelled() {
    let fixture = fixture();
    let holder = fixture.holder();
    let parent = fixture.start("late", "child");
    fixture.until("the program asks for its child", || !fixture.engine.requests(&parent.run).is_empty());
    // An evaluation's view of the request, taken before the step leaves its attempt.
    let request = fixture.engine.requests(&parent.run).remove(0);
    fixture.store.set_step_state(&parent.step, "failed", Some("the active execution timeout expired")).unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    assert!(fixture.engine.requests(&parent.run).is_empty());
    // That evaluation now starts the child, after the step's own cancellation found nothing.
    fixture.store.request_mission_run_cancellation(&holder, "make room").unwrap();
    let run = fixture.store.mission_run(&parent.run).unwrap().unwrap();
    let Wait::Run { mission, input } = &request.wait else { panic!("a child run request") };
    assert!(fixture.reconciler.settle_program_child(&fixture.engine, &run, &request, mission, input).unwrap());
    let late = fixture.child(&parent.step, "publish", 1).expect("the late child is journaled");
    let late = fixture.store.mission_run(&late).unwrap().unwrap();
    assert_ne!((late.status.as_str(), late.phase.as_str()), ("running", "normal"), "the late child is cancelled");
}

#[test]
fn children_are_cancelled_with_their_run_or_their_step_and_a_retry_starts_a_new_one() {
    let fixture = fixture();
    // Capacity 1: the second parent's child waits until the first one's is cancelled.
    let cancelled = fixture.start("cancelled", "child");
    fixture.until("the first child starts", || fixture.child(&cancelled.step, "publish", 1).is_some());
    let first = fixture.child(&cancelled.step, "publish", 1).unwrap();
    // Cancelling the parent run cascades to its child.
    fixture.store.request_mission_run_cancellation(&cancelled.run, "not wanted").unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    assert_eq!(fixture.run_status(&first), "cancelled");

    let retried = fixture.start("retried", "child");
    fixture.until("the second parent's child starts", || fixture.child(&retried.step, "publish", 1).is_some());
    let attempt_one = fixture.child(&retried.step, "publish", 1).unwrap();
    // The program step fails while its run goes on (it has another attempt): its child's
    // cancellation is requested in that evaluation.
    fixture.store.set_step_state(&retried.step, "failed", Some("the active execution timeout expired")).unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    let cancelling = fixture.store.mission_run(&attempt_one).unwrap().unwrap();
    assert_ne!((cancelling.status.as_str(), cancelling.phase.as_str()), ("running", "normal"));
    assert_eq!(fixture.run_status(&retried.run), "running");
    // The retried attempt starts from the top with a new child.
    fixture.until("attempt 2 starts its child", || fixture.child(&retried.step, "publish", 2).is_some());
    let attempt_two = fixture.child(&retried.step, "publish", 2).unwrap();
    assert_ne!(attempt_two, attempt_one);
    assert_eq!(fixture.run_status(&attempt_one), "cancelled");
    assert_eq!(fixture.result(&retried.step, "publish", 1), None);
    // A child that fails is a typed failure in its parent.
    let child_step = fixture.store.mission_run(&attempt_two).unwrap().unwrap().steps.remove(0).subject;
    fixture.store.set_step_state(&child_step, "failed", Some("the publisher refused the head")).unwrap();
    fixture.until("the parent step fails", || fixture.status(&retried.step) == "failed");
    let (status, error) = fixture.result(&retried.step, "publish", 2).unwrap();
    assert_eq!(status, "failed");
    assert_eq!(error["_tag"], "ChildFailed");
    assert_eq!(error["run"], attempt_two.as_str());
    assert_eq!(error["status"], "failed");
    assert!(error["reason"].as_str().is_some_and(|reason| !reason.is_empty()), "{error}");
    assert_eq!(error["executions"], json!([]), "the unclaimed child has no executions");
    assert_eq!(error.as_object().unwrap().len(), 5, "{error}");
}

#[test]
fn asks_are_answered_defaulted_or_withdrawn_and_their_answers_replay() {
    let fixture = fixture();
    let parent = fixture.start("asks", "ask");
    let asks = || ["approve", "note", "withdrawn"].map(|key| fixture.child(&parent.step, key, 1));
    fixture.until("the three asks are open", || asks().iter().all(Option::is_some));
    let [approve, note, withdrawn] = asks().map(Option::unwrap);
    // The step waits on a person, and its program stays loaded.
    fixture.until("the step waits on a person", || fixture.status(&parent.step) == "waiting-person");
    fixture.reconciler.reconcile_once().unwrap();
    assert_eq!(fixture.engine.requests(&parent.run).len(), 3);
    for ask in [&approve, &note, &withdrawn] {
        let claim = fixture.store.latest_claim(ask, Some("work.person-asked")).unwrap().unwrap();
        assert_eq!(claim.actor.as_deref(), Some(PROGRAM_ASKER));
        assert_eq!(claim.body["fields"]["origin_step"], parent.step.as_str());
    }
    // One answer resolves its key while the other asks stay open.
    fixture
        .store
        .finish_person_step(
            &PersonStepResponse {
                delegation: None,
                subject: note.clone(),
                actor: "person/owner".into(),
                summary: "Add the changelog.".into(),
                evidence: Vec::new(),
                episode: None,
                answer: Some(AnswerInput { id: None, text: Some("Add the changelog.".into()), option_notes: Vec::new() }),
                idempotency_key: "note-answer".into(),
            },
            false,
        )
        .unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    assert_eq!(
        fixture.result(&parent.step, "note", 1),
        Some(("completed".to_owned(), json!({"summary": "Add the changelog.", "resolved_by": "person/owner"})))
    );
    assert_eq!(fixture.status(&parent.step), "waiting-person");
    // A host replaced now replays the answer and asks the open keys again: no new asks.
    let host = fixture.kill_host();
    fixture.until("the resumed program waits again", || {
        fixture.engine.host_pid().is_some_and(|pid| pid != host) && fixture.engine.requests(&parent.run).len() == 2
    });
    fixture.reconciler.reconcile_once().unwrap();
    assert_eq!(asks(), [Some(approve.clone()), Some(note.clone()), Some(withdrawn.clone())]);
    assert_eq!(count(&fixture.journal(&parent.step), "note", "started"), 2);
    // An ask withdrawn while the program waits fails its key with `AskCancelled`.
    fixture
        .store
        .finish_person_step(
            &PersonStepResponse {
                delegation: None,
                subject: withdrawn.clone(),
                actor: PROGRAM_ASKER.into(),
                summary: "No longer needed.".into(),
                evidence: Vec::new(),
                episode: None,
                answer: None,
                idempotency_key: "withdraw".into(),
            },
            true,
        )
        .unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    assert_eq!(
        fixture.result(&parent.step, "withdrawn", 1),
        Some(("failed".to_owned(), json!({"_tag": "AskCancelled", "ask": withdrawn.as_str()})))
    );
    // The deadline resolves the last ask with its default answer.
    fixture.store.reconcile_person_asks_at(now_ms() + 3 * 60_000).unwrap();
    std::fs::write(&parent.gate, "").unwrap();
    fixture.until("the program completes", || fixture.status(&parent.step) == "completed");
    let defaulted = json!({
        "answer": {"type": "decision", "outcome": "accept", "id": "ship", "label": "Ship"},
        "summary": fixture.store.latest_claim(&approve, Some("work.person-done")).unwrap().unwrap().body["fields"]["summary"],
        "resolved_by": "deadline",
    });
    assert_eq!(fixture.result(&parent.step, "approve", 1), Some(("completed".to_owned(), defaulted.clone())));
    let (_, value) = fixture.result(&parent.step, RESULT_KEY, 1).unwrap();
    assert_eq!(value["asks"][0], json!({"_tag": "Success", "value": defaulted}));
    assert_eq!(value["asks"][2], json!({"_tag": "Failure", "error": {"_tag": "AskCancelled", "ask": withdrawn.as_str()}}));
}

fn publish_child_replacement(fixture: &Fixture) {
    apply_source(&fixture.store, r#"version 2
mission "program-child" state="ready" {
  goal "Publish the corrected head."
  concurrent-runs max=1
  input "head" kind="text"
  step "publish" {
    assigned-to "agent/publisher"
    produces { resource "mission-run/${ST_MISSION_RUN}/published" { kind "custom.st3.program-test" } }
  }
}"#, "superseding-child");
}

#[test]
fn a_program_waits_for_its_superseding_child_instead_of_failing_on_source_cancellation() {
    let fixture = fixture();
    let parent = fixture.start("superseding-child", "child");
    fixture.until("the child starts", || fixture.child(&parent.step, "publish", 1).is_some());
    let child = fixture.child(&parent.step, "publish", 1).unwrap();
    publish_child_replacement(&fixture);
    let source = fixture.store.mission_run(&child).unwrap().unwrap();
    let successor = source.superseded_by.clone().unwrap_or_else(|| panic!(
        "child publication did not supersede {} (status={}, phase={}, fault={:?})",
        source.subject, source.status, source.phase, source.supersession_fault,
    ));
    fixture.until("the replacement is admitted", || fixture.store.mission_run(&successor).unwrap().unwrap().phase == "normal");
    assert_eq!(fixture.run_status(&child), "cancelled");
    assert_eq!(fixture.run_status(&parent.run), "running");
    assert_eq!(fixture.result(&parent.step, "publish", 1), None);
    fixture.complete_child(&successor);
    fixture.until("the child result resolves", || fixture.result(&parent.step, "publish", 1).is_some());
    let (status, result) = fixture.result(&parent.step, "publish", 1).unwrap();
    assert_eq!(status, "completed");
    assert_eq!(result["run"], successor);
    std::fs::write(&parent.gate, "").unwrap();
    fixture.until("the parent program completes", || fixture.status(&parent.step) == "completed");
}

#[test]
fn a_cancelled_program_step_cancels_the_latest_superseding_child() {
    let fixture = fixture();
    let parent = fixture.start("cancel-superseding-child", "child");
    fixture.until("the child starts", || fixture.child(&parent.step, "publish", 1).is_some());
    let child = fixture.child(&parent.step, "publish", 1).unwrap();
    publish_child_replacement(&fixture);
    let source = fixture.store.mission_run(&child).unwrap().unwrap();
    let successor = source.superseded_by.clone().unwrap_or_else(|| panic!(
        "child publication did not supersede {} (status={}, phase={}, fault={:?})",
        source.subject, source.status, source.phase, source.supersession_fault,
    ));
    fixture.store.set_step_state(&parent.step, "cancelled", Some("the work is withdrawn")).unwrap();
    fixture.reconciler.evaluate_changed_mission_runs().unwrap();
    fixture.until("the replacement child is cancelled", || fixture.run_status(&successor) == "cancelled");
}

#[test]
fn an_uncaught_ask_cancellation_fails_the_program_and_run_and_runs_cleanup() {
    let fixture = fixture();
    let parent = fixture.start_mission("program-uncaught-ask", "uncaught", "uncaught-ask");
    fixture.until("both asks are waiting", || {
        fixture.child(&parent.step, "withdrawn", 1).is_some()
            && fixture.child(&parent.step, "still-open", 1).is_some()
            && fixture.status(&parent.step) == "waiting-person"
    });
    let withdrawn = fixture.child(&parent.step, "withdrawn", 1).unwrap();
    let still_open = fixture.child(&parent.step, "still-open", 1).unwrap();
    fixture.store.finish_person_step(&PersonStepResponse {
        delegation: None,
        subject: withdrawn.clone(),
        actor: PROGRAM_ASKER.into(),
        summary: "No longer needed.".into(),
        evidence: Vec::new(),
        episode: None,
        answer: None,
        idempotency_key: "withdraw-uncaught".into(),
    }, true).unwrap();
    fixture.until("uncaught cancellation ends the run", || fixture.run_status(&parent.run) == "failed");
    let cancelled = json!({"_tag": "AskCancelled", "ask": withdrawn});
    assert_eq!(fixture.result(&parent.step, "withdrawn", 1), Some(("failed".into(), cancelled.clone())));
    // The typed program exit is durable as well as retained in the step's reason.
    assert_eq!(fixture.result(&parent.step, RESULT_KEY, 1), Some(("failed".into(), cancelled.clone())));
    let failed = fixture.store.step_run(&parent.step).unwrap().unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(serde_json::from_str::<Value>(failed.blocked_reason.as_deref().unwrap()).unwrap(), cancelled);
    let run = fixture.store.mission_run(&parent.run).unwrap().unwrap();
    assert_eq!(run.phase, "terminal");
    assert_eq!(run.steps.iter().find(|step| step.step == "cleanup").unwrap().status, "completed");
    assert_eq!(fixture.status(&still_open), "cancelled");
    assert!(fixture.engine.requests(&parent.run).is_empty());
}

fn native_turn_request() -> crate::model::NativeTurn {
    crate::model::NativeTurn {
        agent: "agent/node.publisher".into(),
        instructions: "Produce ${literal} without interpolation.".into(),
        context: "{\"input\":\"${ST_MISSION_RUN}\"}".into(),
        products: vec![crate::model::NativeTurnProduct {
            path: "answer".into(), kind: "custom.st3.program-test".into(),
            fields: BTreeMap::from([("head".into(), json!("h1"))]),
        }],
        fresh_context: false,
    }
}

fn native_parent(fixture: &Fixture, name: &str) -> MissionRunView {
    let parent = fixture.start(name, "turn");
    fixture.store.set_step_state(&parent.step, "working", None).unwrap();
    fixture.store.mission_run(&parent.run).unwrap().unwrap()
}

fn native_work(actor: &str, key: &str) -> crate::model::WorkRequest {
    crate::model::WorkRequest {
        actor: Some(actor.into()), incarnation: Some("current".into()), summary: Some("Actual worker summary".into()),
        reason: None, evidence: vec![], wait_ms: None, idempotency_key: key.into(),
    }
}

fn observe_native_product(fixture: &Fixture, run: &str, head: &str) {
    fixture.store.append_claim(&ClaimInput {
        subject: format!("resource/{run}/answer"), kind: "resource.observed".into(),
        actor: Some("agent/node.publisher".into()),
        fields: BTreeMap::from([("kind".into(), json!("custom.st3.program-test")), ("head".into(), json!(head))]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[test]
fn native_turn_adopts_one_parent_owned_literal_leaf_and_preserves_worker_text() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-adopt");
    let owner = &parent.steps[0];
    let turn = native_turn_request();
    let episode = fixture.store.create_native_turn(&parent, &owner.subject, owner.attempt, "native-adopt-key", &turn).unwrap();
    fixture.store.fixture_replay_graph().unwrap().unwrap();
    let again = fixture.store.create_native_turn(&parent, &owner.subject, owner.attempt, "native-adopt-key", &turn).unwrap();
    assert_eq!(episode.subject, again.subject);
    assert_eq!(fixture.children(&owner.subject).len(), 1);
    assert_eq!(episode.parent_step_run.as_deref(), Some(owner.subject.as_str()));
    assert_eq!(episode.workspace, parent.workspace);
    assert_eq!(episode.requester, parent.requester);
    let leaf = &episode.steps[0];
    assert_eq!(leaf.assigned_to.as_deref(), Some(turn.agent.as_str()));
    assert_eq!(leaf.goals, vec![turn.instructions.clone()]);
    assert!(leaf.constraints.iter().any(|text| text.contains(&turn.context)));
    assert!(leaf.constraints.iter().any(|text| text.contains(&format!("resource/{}/answer", episode.subject))));
    assert!(!leaf.fresh_context);
    let created = fixture.store.latest_claim(&episode.subject, Some("mission-run.created")).unwrap().unwrap();
    assert_eq!(created.actor.as_deref(), Some(parent.subject.as_str()));
    assert!(fixture.store.mission_spec("native-turn", None).unwrap().is_none(), "native work must not publish a graph mission");
    let mut changed = turn;
    changed.context.push('x');
    assert_eq!(fixture.store.create_native_turn(&parent, &owner.subject, owner.attempt, "native-adopt-key", &changed).unwrap_err().code, "idempotency-conflict");
    assert!(fixture.store.create_native_turn(&parent, &owner.subject, owner.attempt + 1, "native-other-key", &changed).is_err());
}

#[test]
fn native_turn_requires_actual_literal_report_and_verified_products_before_success() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-products");
    let episode = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1, "native-products", &native_turn_request()).unwrap();
    let step = &episode.steps[0].subject;
    let wrong = fixture.store.work_action(step, "claim", &native_work("agent/node.other", "native-wrong-actor")).unwrap_err();
    assert_eq!(wrong.code, "work-not-available");
    assert!(!fixture.store.complete_native_turn(&episode.subject, &[]).unwrap());
    fixture.store.work_action(step, "claim", &native_work("agent/node.publisher", "native-product-claim")).unwrap();
    let receipt = fixture.store.step_executions(step).unwrap();
    assert_eq!(receipt.len(), 1);
    assert_eq!(receipt[0].agent, "agent/node.publisher");
    assert_eq!(receipt[0].incarnation_id.as_deref(), Some("current"));
    let submitted = fixture.store.work_action(step, "complete", &native_work("agent/node.publisher", "native-product-submit")).unwrap();
    assert_eq!(submitted.status, "verifying");
    assert!(submitted.worker_reported);
    assert!(!fixture.store.complete_native_turn(&episode.subject, &receipt).unwrap());
    observe_native_product(&fixture, &episode.subject, "wrong");
    assert!(!fixture.store.complete_native_turn(&episode.subject, &receipt).unwrap());
    observe_native_product(&fixture, &episode.subject, "h1");
    assert!(fixture.store.complete_native_turn(&episode.subject, &receipt).unwrap());
    let report = fixture.store.latest_claim(&episode.subject, Some("program.turn.report")).unwrap().unwrap();
    assert_eq!(report.body["fields"]["report"]["summary"], "Actual worker summary");
    assert_eq!(report.body["fields"]["report"]["products"]["answer"]["head"], "h1");
    assert_eq!(report.body["fields"]["report"]["executions"], serde_json::to_value(receipt).unwrap());
    assert_eq!(fixture.run_status(&episode.subject), "completed");
}

#[test]
fn native_turn_cancellation_fences_late_worker_completion_without_stopping_the_seat() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-cancel");
    let episode = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1, "native-cancel", &native_turn_request()).unwrap();
    let step = &episode.steps[0].subject;
    fixture.store.work_action(step, "claim", &native_work("agent/node.publisher", "native-cancel-claim")).unwrap();
    fixture.store.request_mission_run_cancellation(&parent.subject, "cancel parent with no host request map").unwrap();
    assert_eq!(fixture.status(step), "cancelled");
    finish_native_cancellation(&fixture, &episode.subject);
    assert!(fixture.store.work_action(step, "complete", &native_work("agent/node.publisher", "native-late-submit")).is_err());
    observe_native_product(&fixture, &episode.subject, "h1");
    assert!(!fixture.store.complete_native_turn(&episode.subject, &[]).unwrap());
    assert!(fixture.store.claims_for("agent/node.publisher", Some("runtime.action.requested")).unwrap().is_empty());
    assert!(fixture.store.latest_claim(&episode.subject, Some("program.turn.report")).unwrap().is_none());
}

#[test]
fn native_turn_freshness_is_opt_in_and_busy_seat_claims_remain_exclusive() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-freshness");
    let first = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1, "native-held", &native_turn_request()).unwrap();
    fixture.store.work_action(&first.steps[0].subject, "claim", &native_work("agent/node.publisher", "native-held-claim")).unwrap();
    let mut fresh = native_turn_request();
    fresh.fresh_context = true;
    let next = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1, "native-fresh-next", &fresh).unwrap();
    assert!(fixture.store.step_run(&next.steps[0].subject).unwrap().unwrap().fresh_context);
    assert_eq!(fixture.store.work_action(&next.steps[0].subject, "claim", &native_work("agent/node.publisher", "native-busy-refused")).unwrap_err().code, "agent-capacity");
    assert_eq!(fixture.status(&first.steps[0].subject), "claimed");
    assert!(fixture.store.claims_for("agent/node.publisher", Some("runtime.action.requested")).unwrap().is_empty());
    fixture.store.request_mission_run_cancellation(&first.subject, "release only the first native episode").unwrap();
    assert_eq!(fixture.store.work_action(&next.steps[0].subject, "claim",
        &native_work("agent/node.publisher", "native-idle-needs-freshness")).unwrap_err().code, "fresh-context-pending");
}

#[test]
fn native_turn_resource_replays_after_host_loss_and_returns_actual_report() {
    let fixture = fixture();
    let parent = fixture.start("native-host-adoption", "turn");
    let subject = crate::store::adhoc_work::native_turn_subject(&format!("program:{}:1:turn", parent.step));
    fixture.until("a real native work episode starts", || fixture.store.mission_run(&subject).unwrap().is_some());
    let old_host = fixture.kill_host();
    fixture.until("host resumes the same native turn", || fixture.engine.host_pid().is_some_and(|pid| pid != old_host));
    let episode = fixture.store.mission_run(&subject).unwrap().unwrap();
    assert_eq!(fixture.children(&parent.step).len(), 1);
    let step = &episode.steps[0].subject;
    fixture.store.work_action(step, "claim", &native_work("agent/node.publisher", "native-host-claim")).unwrap();
    fixture.store.work_action(step, "complete", &native_work("agent/node.publisher", "native-host-submit")).unwrap();
    fixture.reconciler.evaluate_mission_runs().unwrap();
    assert_eq!(fixture.status(step), "verifying");
    assert!(fixture.result(&parent.step, RESULT_KEY, 1).is_none());
    observe_native_product(&fixture, &subject, "h1");
    fixture.until("native turn report returns through the program result", || fixture.result(&parent.step, RESULT_KEY, 1).is_some());
    let (_, report) = fixture.result(&parent.step, RESULT_KEY, 1).unwrap();
    assert_eq!(report["run"], subject);
    assert_eq!(report["step"], *step);
    assert_eq!(report["summary"], "Actual worker summary");
    assert_eq!(report["products"]["answer"]["head"], "h1");
    assert_eq!(report["executions"].as_array().unwrap().len(), 1);
    fixture.until("parent native program run completes", || fixture.run_status(&parent.run) == "completed");
    let child_exit = fixture.reconciler.program_child_exit(&parent.run, "completed").unwrap();
    assert_eq!(child_exit["value"]["result"], report);
    assert!(child_exit["value"].get("products").is_none(), "program results must not fabricate resource products");
}

fn native_scope(fixture: &Fixture, parent: &Started) -> crate::store::resource_membership::Membership {
    use crate::store::resource_membership::{Admission, Coverage, Fit, LaneRequest, Owner};
    fixture.reconciler.ensure_resource_authority().unwrap();
    let lane = LaneRequest {
        id: format!("native-scope:{}", parent.run), name: "native-workers".into(), host: "node".into(),
        max_members: 1, capacity: Some(fractal_runtime::Capacity { cores: 1, memory: 1024 * 1024 }),
        coverage: Coverage::Logical, admission_timeout_ms: None, class: fractal_runtime::RequestClass::Agent,
        disk: None, io: None,
        profile: None, profile_estimate: None, disk_path: None,
        owner: Owner { run: parent.run.clone(), step: parent.step.clone(), attempt: 1, invocation: None },
    };
    let fit = Fit { room: Some(fractal_runtime::Capacity { cores: 4, memory: 1024 * 1024 * 1024 }), ..Fit::default() };
    let Admission::Admitted(grant) = fixture.store.request_resource_membership(&lane, fit, 1).unwrap()
        else { panic!("native fixture scope must admit"); };
    fixture.store.attach_resource_child(&grant, &lane.owner, &parent.run).unwrap();
    fixture.store.resource_membership_record(&lane.id, &lane.owner).unwrap()
}

#[test]
fn native_turn_inherits_same_host_membership_without_reparenting_the_standing_seat() {
    let fixture = fixture();
    let parent = fixture.start("native-scoped", "turn");
    let scope = native_scope(&fixture, &parent);
    let episode = crate::store::adhoc_work::native_turn_subject(&format!("program:{}:1:turn", parent.step));
    fixture.until("native same-host episode attaches", || fixture.store.mission_run(&episode).unwrap().is_some());
    let inherited = fixture.store.resource_context_for_run(&episode).unwrap().unwrap();
    assert_eq!(inherited.request, scope.request);
    assert_eq!(inherited.grant, scope.grant);
    assert!(inherited.child_runs.contains(&episode));
    let leaf = fixture.store.mission_run(&episode).unwrap().unwrap().steps.remove(0);
    fixture.store.work_action(&leaf.subject, "claim", &native_work("agent/node.publisher", "native-scoped-claim")).unwrap();
    fixture.store.work_action(&leaf.subject, "complete", &native_work("agent/node.publisher", "native-scoped-submit")).unwrap();
    observe_native_product(&fixture, &episode, "h1");
    fixture.until("same-host native report returns", || fixture.result(&parent.step, RESULT_KEY, 1).is_some());
    let (_, report) = fixture.result(&parent.step, RESULT_KEY, 1).unwrap();
    assert_eq!(report["executions"][0]["agent"], "agent/node.publisher");
    assert!(fixture.store.claims_for("agent/node.publisher", Some("runtime.action.requested")).unwrap().iter()
        .all(|claim| claim.body["fields"]["action"] != "fresh-context"));
}

#[test]
fn native_turn_refuses_cross_host_inherited_authority_without_claiming_custody() {
    let fixture = fixture();
    let parent = fixture.start("native-cross-host", "turn-remote");
    let scope = native_scope(&fixture, &parent);
    fixture.until("native cross-host refusal returns", || fixture.result(&parent.step, RESULT_KEY, 1).is_some());
    let (status, error) = fixture.result(&parent.step, RESULT_KEY, 1).unwrap();
    assert_eq!(status, "failed");
    assert_eq!(error["_tag"], "ResourceError");
    assert_eq!(error["code"], "resource-operation-refused");
    assert!(error["message"].as_str().unwrap().contains("target host"));
    let episode = crate::store::adhoc_work::native_turn_subject(&format!("program:{}:1:turn", parent.step));
    assert!(fixture.store.mission_run(&episode).unwrap().is_none());
    let current = fixture.store.resource_membership_record(&scope.request.id, &scope.request.owner).unwrap();
    assert!(!current.child_runs.contains(&episode));
    assert!(fixture.store.claims_for("agent/peer.remote", Some("runtime.action.requested")).unwrap().is_empty());
}

#[test]
fn native_turn_product_payload_rejects_non_scalar_or_non_relative_requirements() {
    for path in ["", "/absolute", "../escape", "a/../escape", "a@claim"] {
        let mut turn = native_turn_request();
        turn.products[0].path = path.into();
        assert!(turn.validate().is_err(), "accepted invalid product path {path:?}");
    }
    for value in [json!([]), json!({"nested": true})] {
        let mut turn = native_turn_request();
        turn.products[0].fields.insert("value".into(), value);
        assert!(turn.validate().is_err());
    }
}

#[test]
fn native_fresh_context_never_interrupts_an_unrelated_busy_harness_turn() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-busy-harness");
    let mut turn = native_turn_request();
    turn.fresh_context = true;
    let episode = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1, "native-busy-harness", &turn).unwrap();
    let agent = "agent/node.publisher";
    for (kind, fields) in [
        ("runtime.observed", json!({"status":"running","runtime_id":"node.publisher","incarnation_id":"current"})),
        ("harness.observed", json!({"state":"working","driver":"omp","incarnation_id":"current","ownership_sequence":1,"transition_sequence":1})),
    ] {
        fixture.store.append_claim(&ClaimInput {
            subject: agent.into(), kind: kind.into(), actor: Some(agent.into()),
            fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    assert_eq!(fixture.store.current_harness(agent).unwrap().unwrap().state, "working");
    let member = fixture.store.desired_subject_with_writer(agent).unwrap().unwrap().0.member.unwrap();
    let leaf = fixture.store.step_run(&episode.steps[0].subject).unwrap().unwrap();
    assert!(!fixture.reconciler.prepare_fresh_context(agent, "current", &leaf, &member).unwrap());
    assert!(fixture.reconciler.runtime.stops.lock().unwrap().is_empty());
    assert!(fixture.store.claims_for(agent, Some("runtime.action.requested")).unwrap().is_empty());
}

#[test]
fn cancelling_a_native_parent_leaves_another_parents_held_seat_work_untouched() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let independent = native_parent(&fixture, "independent-seat-work");
    let held = fixture.store.create_native_turn(&independent, &independent.steps[0].subject, 1,
        "independent-native-turn", &native_turn_request()).unwrap();
    fixture.store.work_action(&held.steps[0].subject, "claim",
        &native_work("agent/node.publisher", "independent-native-claim")).unwrap();
    let parent = native_parent(&fixture, "cancel-only-owned");
    let queued = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1,
        "cancel-only-owned-turn", &native_turn_request()).unwrap();
    fixture.store.request_mission_run_cancellation(&parent.subject, "cancel only this native parent").unwrap();
    assert_eq!(fixture.status(&queued.steps[0].subject), "cancelled");
    finish_native_cancellation(&fixture, &queued.subject);
    let unchanged = fixture.store.step_run(&held.steps[0].subject).unwrap().unwrap();
    assert_eq!(unchanged.status, "claimed");
    assert_eq!(unchanged.claimant.as_deref(), Some("agent/node.publisher"));
    assert_eq!(fixture.run_status(&held.subject), "running");
    assert!(fixture.reconciler.runtime.stops.lock().unwrap().is_empty());
}

#[test]
fn native_work_claim_is_fenced_immediately_when_its_parent_attempt_changes() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-parent-retry");
    let owner = &parent.steps[0].subject;
    let episode = fixture.store.create_native_turn(&parent, owner, 1, "native-old-attempt", &native_turn_request()).unwrap();
    fixture.store.set_step_state(owner, "failed", Some("retry parent")).unwrap();
    fixture.store.retry_step(owner, "retry parent", 0).unwrap();
    let error = fixture.store.work_action(&episode.steps[0].subject, "claim",
        &native_work("agent/node.publisher", "native-stale-parent-claim")).unwrap_err();
    assert_eq!(error.code, "stale-native-turn-parent");
    assert!(!fixture.store.complete_native_turn(&episode.subject, &[]).unwrap());
    assert!(fixture.store.latest_claim(&episode.subject, Some("program.turn.report")).unwrap().is_none());
    let current = fixture.store.mission_run(&episode.subject).unwrap().unwrap();
    if current.phase == "normal" { fixture.reconciler.evaluate_native_turn(&current).unwrap(); }
    finish_native_cancellation(&fixture, &episode.subject);
}

fn finish_native_cancellation(fixture: &Fixture, run: &str) {
    let cancelling = fixture.store.mission_run(run).unwrap().unwrap();
    assert_eq!(cancelling.phase, "cleanup-cancelled");
    fixture.reconciler.evaluate_native_turn(&cancelling).unwrap();
    let terminal = fixture.store.mission_run(run).unwrap().unwrap();
    assert_eq!(terminal.status, "cancelled");
    assert_eq!(terminal.phase, "terminal");
    assert!(terminal.steps.iter().all(|step| step.status == "cancelled"));
    assert!(fixture.store.mission_spec("native-turn", None).unwrap().is_none());
}

#[test]
fn direct_native_turn_cancellation_reaches_terminal_without_a_graph_revision() {
    use crate::AdhocWorkStore as _;
    let fixture = fixture();
    let parent = native_parent(&fixture, "native-direct-cancellation");
    let episode = fixture.store.create_native_turn(&parent, &parent.steps[0].subject, 1,
        "native-direct-cancellation", &native_turn_request()).unwrap();
    let step = &episode.steps[0].subject;
    fixture.store.work_action(step, "claim", &native_work("agent/node.publisher", "native-direct-cancel-claim")).unwrap();
    fixture.store.request_mission_run_cancellation(&episode.subject, "cancel only the owned episode").unwrap();
    finish_native_cancellation(&fixture, &episode.subject);
    assert_eq!(fixture.run_status(&parent.subject), "running");
    assert_eq!(fixture.status(&parent.steps[0].subject), "working");
    assert!(fixture.store.work_action(step, "complete",
        &native_work("agent/node.publisher", "native-direct-cancel-late-submit")).is_err());
    assert!(fixture.reconciler.runtime.stops.lock().unwrap().is_empty());
}

#[test]
fn native_turn_resource_scope_cancellation_returns_its_real_terminal_child_failure() {
    let fixture = fixture();
    let parent = fixture.start("native-resource-scope-cancellation", "turn-cancel");
    let episode = crate::store::adhoc_work::native_turn_subject(&format!("program:{}:1:turn", parent.step));
    fixture.until("native cancellable resource admits work", || fixture.store.mission_run(&episode).unwrap().is_some());
    let leaf = fixture.store.mission_run(&episode).unwrap().unwrap().steps.remove(0);
    fixture.store.work_action(&leaf.subject, "claim",
        &native_work("agent/node.publisher", "native-resource-cancel-claim")).unwrap();
    std::fs::write(&parent.gate, "").unwrap();
    fixture.until("resource cancellation returns the terminal episode outcome",
        || fixture.result(&parent.step, RESULT_KEY, 1).is_some());
    let (status, error) = fixture.result(&parent.step, RESULT_KEY, 1).unwrap();
    assert_eq!(status, "failed");
    assert_eq!(error["_tag"], "ChildFailed");
    assert_eq!(error["run"], episode);
    assert_eq!(error["status"], "cancelled");
    let terminal = fixture.store.mission_run(&episode).unwrap().unwrap();
    assert_eq!(terminal.status, "cancelled");
    assert_eq!(terminal.phase, "terminal");
    assert!(fixture.store.work_action(&leaf.subject, "complete",
        &native_work("agent/node.publisher", "native-resource-cancel-late-submit")).is_err());
    assert!(fixture.store.latest_claim(&episode, Some("program.turn.report")).unwrap().is_none());
    assert!(fixture.reconciler.runtime.stops.lock().unwrap().is_empty());
}
