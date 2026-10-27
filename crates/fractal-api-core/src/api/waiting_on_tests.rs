//! What a run waits on, read from the published work and missions lists as their refreshers fold
//! them, against runs driven through claims, progress, asks, waits and a stopped seat.
use super::*;
use crate::PersonWorkStore;
use crate::api::published_lists::tests_shared_client_v0_tests::{fold, mission, publish};
use crate::model::{MissionRunRequest, PersonAskRequest, WorkRequest};
use crate::store::mission_list::RosterFleet;
use smallclaims::store::set_thread_clock;

const MINUTE: u128 = 60_000;
const QUIET: u64 = DEFAULT_QUIET_AFTER_MS;

/// Resets the thread's clock however the test ends.
struct Clock;

impl Clock {
    fn at(now: u128) -> Self {
        set_thread_clock(Some(now));
        Self
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        set_thread_clock(None);
    }
}

fn start_time() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
}

fn start(store: &Store, name: &str, key: &str) -> crate::model::MissionRunView {
    store
        .create_mission_run(&MissionRunRequest {
            mission: name.into(),
            revision: None,
            workspace: "/work/garden".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap()
}

fn step(run: &crate::model::MissionRunView, path: &str) -> (String, String) {
    let step = run.steps.iter().find(|step| step.step == path).unwrap();
    (step.subject.clone(), step.assigned_to.clone().unwrap())
}

fn act(store: &Store, subject: &str, actor: &str, action: &str, wait_ms: Option<u64>, key: &str) {
    store
        .work_action(
            subject,
            action,
            &WorkRequest {
                wait_ms,
                wait_on: None,
                actor: Some(actor.into()),
                incarnation: Some(format!("{actor}-1")),
                summary: Some(format!("{action} by {actor}")),
                reason: Some(format!("{action} for the test")),
                evidence: vec!["doc/garden/evidence".into()],
                idempotency_key: key.into(),
            },
        )
        .unwrap();
}

/// Both lists folded as their refreshers fold them, at the current cut.
fn published(store: &Store) -> (Arc<Publication<WorkRows>>, Arc<Publication<MissionRows>>) {
    let (missions, _) = fold(store);
    store.published_work_list().start();
    published_lists::refresh_work_once(store);
    let work = store.published_work_list().newest().expect("a published work list");
    assert_eq!(work.cut, store.index().unwrap(), "the work list is published at the current cut");
    (work, missions)
}

fn waits(store: &Store, run: &str, now: u128, quiet: u64) -> RunWaitingOn {
    let (work, missions) = published(store);
    let steps = store.run_step_states(run).unwrap().expect("the run exists");
    run_waiting_on(&steps, &work, &missions.rows, now, quiet)
}

fn open<'a>(waiting: &'a RunWaitingOn, path: &str) -> &'a StepWait {
    waiting.open.iter().find(|wait| wait.path == path).unwrap_or_else(|| panic!("{path} is open"))
}

#[test]
fn a_run_names_what_each_step_waits_on_and_stalls_once_its_holder_and_its_queue_go_quiet() {
    let t0 = start_time();
    let _clock = Clock::at(t0);
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("graph.db"), "cedar").unwrap();
    mission(&store, "garden/alpha");
    let alpha = start(&store, "garden/alpha", "alpha-1");
    let (plant, ash) = step(&alpha, "plant");
    let (water, _) = step(&alpha, "water");
    store.set_step_state(&plant, "ready", None).unwrap();
    store.set_step_state(&water, "ready", None).unwrap();
    act(&store, &plant, &ash, "claim", None, "claim-plant");
    act(&store, &plant, &ash, "progress", None, "progress-plant");

    // A minute in: the holder is moving, the other step is newly ready.
    let waiting = waits(&store, &alpha.subject, t0 + MINUTE, QUIET);
    assert_eq!(waiting.verdict, RunVerdict::Moving, "{waiting:#?}");
    assert_eq!(waiting.cause, None);
    let planting = open(&waiting, "plant");
    assert_eq!(planting.kind, WaitKind::Working);
    assert_eq!(planting.who.as_deref(), Some(ash.as_str()));
    assert_eq!(planting.detail.as_deref(), Some("progress by agent/garden/ash"));
    assert!(planting.lease_expires_at_unix_ms.is_some(), "a held step shows its lease");
    assert_eq!(open(&waiting, "water").kind, WaitKind::Ready);
    assert!(waiting.finished.is_empty() && waiting.pending.is_empty());

    // Past the quiet limit with no new claim: no progress, and nobody claims the ready step.
    // Claims are stamped a moment after `t0`: read half a minute past the whole minute.
    let late = t0 + 31 * MINUTE + MINUTE / 2;
    let waiting = waits(&store, &alpha.subject, late, QUIET);
    assert_eq!(waiting.verdict, RunVerdict::Stalled);
    assert_eq!(waiting.cause, Some(WaitKind::NoProgress));
    assert_eq!(waiting.open[0].path, "plant", "the most blocking step first");
    assert_eq!(open(&waiting, "water").kind, WaitKind::Unclaimed);
    assert!(
        waiting.reason.starts_with("plant: agent/garden/ash holds it, no progress for 31m"),
        "{}",
        waiting.reason
    );

    // A step that ends moves to the finished steps.
    store.set_step_state(&plant, "completed", None).unwrap();
    let waiting = waits(&store, &alpha.subject, late, QUIET);
    assert_eq!(waiting.finished.iter().map(|step| step.path.as_str()).collect::<Vec<_>>(), ["plant"]);
    assert_eq!(waiting.cause, Some(WaitKind::Unclaimed));
    assert!(waiting.reason.starts_with("water: ready for 31m, nobody claims it"), "{}", waiting.reason);
}

#[test]
fn an_ask_waits_on_its_person_and_a_declared_wait_is_a_hold_not_a_stall() {
    let t0 = start_time();
    let _clock = Clock::at(t0);
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("graph.db"), "cedar").unwrap();
    publish(
        &store,
        r#"version 2
mission "garden/review" state="ready" {
  goal "Review the beds."
  step "review" { assigned-to "agent/garden/ash" }
  step "prune" { assigned-to "agent/garden/birch" }
}"#,
        "publish-review",
    );
    // Only a declared agent's live work can ask a person.
    publish(
        &store,
        "version 2\nagent \"garden/ash\" { workspace \"/tmp\"; command \"true\"; restart always; }",
        "publish-ash",
    );
    let run = start(&store, "garden/review", "review-1");
    let (review, ash) = step(&run, "review");
    let (prune, birch) = step(&run, "prune");
    for (subject, actor) in [(&review, &ash), (&prune, &birch)] {
        store.set_step_state(subject, "ready", None).unwrap();
        act(&store, subject, actor, "claim", None, &format!("claim-{subject}"));
    }
    act(&store, &prune, &birch, "wait", Some(2 * 60 * MINUTE as u64), "wait-prune");
    store
        .ask_person(&PersonAskRequest {
            legacy_request: None,
            person: "person/ada".into(),
            title: "Which beds stay?".into(),
            reason: "The plan names two.".into(),
            actor: ash.clone(),
            step: Some(review.clone()),
            new_run: None,
            incarnation: Some(format!("{ash}-1")),
            request: None,
            idempotency_key: "ask-beds".into(),
        })
        .unwrap();

    let late = t0 + 45 * MINUTE + MINUTE / 2;
    let waiting = waits(&store, &run.subject, late, QUIET);
    assert_eq!(waiting.verdict, RunVerdict::Waiting, "{waiting:#?}");
    assert_eq!(waiting.cause, Some(WaitKind::Ask));
    let reviewing = open(&waiting, "review");
    assert_eq!(reviewing.kind, WaitKind::Ask);
    assert_eq!(reviewing.who.as_deref(), Some("person/ada"));
    assert_eq!(reviewing.detail.as_deref(), Some("Which beds stay?"));
    // The ask is a step of the run too, assigned to the person.
    let ask = waiting.open.iter().find(|wait| wait.path.starts_with("ask-")).expect("the ask is open");
    assert_eq!((ask.kind, ask.who.as_deref()), (WaitKind::Ask, Some("person/ada")));
    // The wait holds prune past the quiet limit, until it ends.
    let pruning = open(&waiting, "prune");
    assert_eq!(pruning.kind, WaitKind::Hold);
    assert!(pruning.until_unix_ms.is_some_and(|until| until > late));
    assert!(waiting.reason.starts_with("review waits on person/ada for 45m"), "{}", waiting.reason);
}

#[test]
fn a_stopped_seat_is_the_cause_of_the_step_it_would_take() {
    let t0 = start_time();
    let _clock = Clock::at(t0);
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("graph.db"), "cedar").unwrap();
    mission(&store, "garden/alpha");
    let alpha = start(&store, "garden/alpha", "alpha-1");
    let (plant, ash) = step(&alpha, "plant");
    let (water, birch) = step(&alpha, "water");
    store.set_step_state(&plant, "ready", None).unwrap();
    store.set_step_state(&water, "ready", None).unwrap();
    act(&store, &plant, &ash, "claim", None, "claim-plant");
    let (work, missions) = published(&store);
    let mut rows = missions.rows.clone();
    rows.roster = Some(Arc::new(RosterFleet::from_cards(
        1,
        &[
            json!({ "id": ash, "name": "ash", "state": "running" }),
            json!({ "id": birch, "name": "birch", "state": "stopped" }),
        ],
    )));
    let steps = store.run_step_states(&alpha.subject).unwrap().unwrap();
    // Within the quiet limit: the stopped seat stalls the run at once.
    let waiting = run_waiting_on(&steps, &work, &rows, t0 + MINUTE, QUIET);
    assert_eq!((waiting.verdict, waiting.cause), (RunVerdict::Stalled, Some(WaitKind::SeatDown)));
    let watering = open(&waiting, "water");
    assert_eq!((watering.who.as_deref(), watering.seat_state.as_deref()), (Some(birch.as_str()), Some("stopped")));
    assert_eq!(watering.detail.as_deref(), Some("agent/garden/birch would take it and is stopped"));
    assert_eq!(open(&waiting, "plant").seat_state.as_deref(), Some("running"));
}

#[tokio::test]
async fn the_reads_serve_only_published_lists() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::published_lists::tests_shared_client_v0_tests::app_state(root.path());
    mission(&state.store, "garden/alpha");
    let alpha = start(&state.store, "garden/alpha", "alpha-1");
    let query = || Query(WaitingOnQuery::default());
    // Lists that were never published, as with ST3_PUBLISHED_LISTS=off: say so, never fold.
    let refused = get_run_waiting_on(State(state.clone()), AxumPath(alpha.subject.clone()), query()).await.unwrap_err();
    assert_eq!(refused.code, WAITING_ON_UNAVAILABLE);
    // Started and not yet published: retry shortly.
    state.store.published_work_list().start();
    let refused = get_run_waiting_on(State(state.clone()), AxumPath(alpha.subject.clone()), query()).await.unwrap_err();
    assert_eq!(refused.code, published_lists::WORK_LIST_NOT_READY);
    published(&state.store);
    let Json(waiting) = get_run_waiting_on(State(state.clone()), AxumPath(alpha.subject.clone()), query()).await.unwrap();
    assert_eq!(waiting.run, alpha.subject);
    assert_eq!(waiting.mission, "mission/garden/alpha");
    assert_eq!(waiting.quiet_after_ms, QUIET);
    assert_eq!(waiting.work_cut, state.store.index().unwrap());
    let missing = get_run_waiting_on(State(state.clone()), AxumPath("mission-run/nowhere".into()), query())
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not-found");
}
