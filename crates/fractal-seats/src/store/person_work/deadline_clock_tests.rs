//! An ask's deadline under the asked person's deadline clock: held time moves it, and the
//! reconciler, the person's home and the late-answer fence all use the moved deadline.
use super::*;
use super::tests::{fixture, receive};
use fractal_missions::store::person_work_tests_shared_seats::{TEST_FLEET, gateway_decision};
use fractal_store::store::availability::{AvailabilityChange, AvailabilityStore, DeadlineClock};

const ASKED: u128 = 1_800_000_000_000;
const MINUTE: u128 = 60_000;
const HOUR: u128 = 60 * MINUTE;

/// Fixes this thread's clock, which batched writes carry, until dropped.
struct SimulatedClock;
impl SimulatedClock {
    fn at(at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        Self
    }
}
impl Drop for SimulatedClock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

/// person/avery sets their deadline clock at `at`.
fn set_clock(store: &Store, clock: DeadlineClock, until: Option<u128>, at: u128) {
    smallclaims::store::set_thread_clock(Some(at));
    store.set_person_availability("person/avery", "host/test", &[AvailabilityChange::DeadlineClock(clock, until)], at).unwrap();
}

/// The deadline person/avery's home shows for `subject` at `at`.
fn home_deadline(store: &Store, subject: &str, at: u128) -> Option<u128> {
    smallclaims::store::set_thread_clock(Some(at));
    store.attention_items(Some("person/avery")).unwrap().into_iter()
        .find(|item| item.subject == subject).unwrap().deadline_unix_ms
}

fn stored_deadline(store: &Store, subject: &str) -> u128 {
    u128::from(request(&store.readers.get(), subject).unwrap().unwrap().body["fields"]["deadline_unix_ms"].as_u64().unwrap())
}

/// An open `hold-all` keeps a default from applying however long past its stored deadline, and
/// arms no wake. Once the clock runs again the deadline is the stored one plus the time held.
#[test]
fn hold_all_keeps_an_asks_default_from_applying_until_the_deadline_clock_runs() {
    let _clock = SimulatedClock::at(ASKED);
    let (store, origin, mut input) = fixture();
    store.set_ask_deadlines(true);
    input.request = Some(gateway_decision());
    input.deadline_ms = Some(2 * HOUR as u64);
    input.default_answer = Some("keep-open".into());
    let ask = store.ask_person(&input).unwrap();
    let deadline = stored_deadline(&store, &ask.subject);
    assert_eq!(deadline, ASKED + 2 * HOUR);
    let held = ASKED + 10 * MINUTE;
    set_clock(&store, DeadlineClock::HoldAll, None, held);
    let late = deadline + HOUR;
    smallclaims::store::set_thread_clock(Some(late));
    assert_eq!(store.reconcile_person_asks_at(late).unwrap(), PersonAskPass::default(), "held: nothing resolves, no wake");
    assert_eq!(store.step_run(&ask.subject).unwrap().unwrap().status, "ready");
    assert_eq!(home_deadline(&store, &ask.subject, late), None, "the home shows no deadline while held");
    set_clock(&store, DeadlineClock::Run, None, late);
    let effective = deadline + (late - held);
    assert_eq!(store.reconcile_person_asks_at(late).unwrap(), PersonAskPass { changed: false, next_deadline: Some(effective) });
    assert_eq!(home_deadline(&store, &ask.subject, late), Some(effective));
    smallclaims::store::set_thread_clock(Some(effective - 1));
    assert!(!store.reconcile_person_asks_at(effective - 1).unwrap().changed);
    smallclaims::store::set_thread_clock(Some(effective));
    assert!(store.reconcile_person_asks_at(effective).unwrap().changed);
    let resumed = store.step_run(&origin.subject).unwrap().unwrap();
    assert_eq!(resumed.person_answers[0].resolved_by.as_deref(), Some("deadline"));
    assert_eq!(resumed.person_answers[0].answer.as_ref().unwrap()["id"], "keep-open");
}

/// `hold-routine` holds every ask that is not urgent, however short its deadline; one its asker
/// gave priority `critical` is urgent and resolves on time.
#[test]
fn hold_routine_holds_a_routine_ask_while_an_urgent_one_resolves_at_its_deadline() {
    let _clock = SimulatedClock::at(ASKED);
    let (store, origin, mut input) = fixture();
    store.set_ask_deadlines(true);
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    input.step = None;
    input.deadline_ms = Some(HOUR as u64);
    input.new_run = Some("routine".into());
    input.priority = Some("high".into());
    let routine = store.ask_person(&input).unwrap();
    input.new_run = Some("urgent".into());
    input.idempotency_key = "urgent-release-date".into();
    input.priority = Some("critical".into());
    let urgent = store.ask_person(&input).unwrap();
    assert_eq!((stored_deadline(&store, &routine.subject), stored_deadline(&store, &urgent.subject)), (ASKED + HOUR, ASKED + HOUR));
    set_clock(&store, DeadlineClock::HoldRoutine, None, ASKED + 10 * MINUTE);
    // The first pass starts counting deadlines; only the urgent one arms a wake.
    assert_eq!(store.reconcile_person_asks_at(ASKED + 20 * MINUTE).unwrap(), PersonAskPass { changed: false, next_deadline: Some(ASKED + HOUR) });
    assert_eq!(home_deadline(&store, &urgent.subject, ASKED + 20 * MINUTE), Some(ASKED + HOUR));
    assert_eq!(home_deadline(&store, &routine.subject, ASKED + 20 * MINUTE), None);
    smallclaims::store::set_thread_clock(Some(ASKED + HOUR));
    assert_eq!(store.reconcile_person_asks_at(ASKED + HOUR).unwrap(), PersonAskPass { changed: true, next_deadline: None });
    assert_eq!(store.step_run(&urgent.subject).unwrap().unwrap().status, "completed");
    let later = ASKED + 10 * HOUR;
    smallclaims::store::set_thread_clock(Some(later));
    assert_eq!(store.reconcile_person_asks_at(later).unwrap(), PersonAskPass::default());
    assert_eq!(store.step_run(&routine.subject).unwrap().unwrap().status, "ready");
}

/// An ask records the priority its asker gave it, `normal` by default, and its attention row
/// is urgent exactly when that priority is `critical`. A retry under the same key must name the
/// same priority; an unknown priority, or one on an update, is refused.
#[test]
fn an_ask_records_its_priority_and_only_critical_makes_its_row_urgent() {
    let _clock = SimulatedClock::at(ASKED);
    let (store, origin, mut input) = fixture();
    store.set_step_state(&origin.subject, "completed", None).unwrap();
    input.step = None;
    let mut rows = Vec::new();
    for (name, priority) in [("unnamed", None), ("normal", Some("normal")), ("low", Some("low")), ("high", Some("high")), ("critical", Some("critical"))] {
        input.new_run = Some(name.into());
        input.idempotency_key = format!("priority-{name}");
        input.priority = priority.map(Into::into);
        let ask = store.ask_person(&input).unwrap();
        let claim = request(&store.readers.get(), &ask.subject).unwrap().unwrap();
        let recorded = claim.body["fields"].get("priority").and_then(|priority| priority.as_str());
        assert_eq!(recorded, priority.filter(|priority| *priority != "normal"), "{name}");
        let row = store.attention_items(Some("person/avery")).unwrap().into_iter()
            .find(|item| item.subject == ask.subject).unwrap();
        rows.push((row.priority.clone(), row.is_urgent()));
    }
    assert_eq!(rows, [
        ("normal".to_owned(), false), ("normal".to_owned(), false), ("low".to_owned(), false),
        ("high".to_owned(), false), ("critical".to_owned(), true),
    ]);
    // Naming `normal` is the same ask as naming none; another priority is another question.
    input.new_run = Some("unnamed".into());
    input.idempotency_key = "priority-unnamed".into();
    input.priority = Some("normal".into());
    store.ask_person(&input).unwrap();
    input.priority = Some("critical".into());
    assert_eq!(store.ask_person(&input).unwrap_err().code, "idempotency-conflict");
    input.idempotency_key = "priority-unknown".into();
    input.priority = Some("blocker".into());
    assert_eq!(store.ask_person(&input).unwrap_err().code, "invalid-ask-priority");
    input.new_run = None;
    input.idempotency_key = "priority-update".into();
    input.priority = Some("critical".into());
    input.request = Some(serde_json::json!({"version": 1, "type": "update", "about": origin.run}));
    assert_eq!(store.ask_person(&input).unwrap_err().code, "invalid-ask-priority");
    // `normal` is no priority at all, so an update may name it.
    input.priority = Some("normal".into());
    let named_normal = store.ask_person(&input).err().map(|error| error.code);
    assert_ne!(named_normal, Some("invalid-ask-priority"));
}

/// A hold with `until` ends by itself, with no claim: the deadline moves by exactly the hold's
/// length, and the home shows the moved deadline while the hold is still in force.
#[test]
fn a_deadline_clock_hold_until_ends_by_itself_and_moves_the_deadline_by_its_length() {
    let _clock = SimulatedClock::at(ASKED);
    let (store, origin, mut input) = fixture();
    store.set_ask_deadlines(true);
    input.deadline_ms = Some(2 * HOUR as u64);
    let ask = store.ask_person(&input).unwrap();
    let deadline = stored_deadline(&store, &ask.subject);
    set_clock(&store, DeadlineClock::HoldAll, Some(ASKED + 40 * MINUTE), ASKED + 10 * MINUTE);
    let effective = deadline + 30 * MINUTE;
    smallclaims::store::set_thread_clock(Some(ASKED + 20 * MINUTE));
    assert_eq!(store.reconcile_person_asks_at(ASKED + 20 * MINUTE).unwrap(), PersonAskPass { changed: false, next_deadline: Some(effective) });
    assert_eq!(home_deadline(&store, &ask.subject, ASKED + 20 * MINUTE), Some(effective), "shown while the hold is in force");
    smallclaims::store::set_thread_clock(Some(deadline));
    assert_eq!(store.reconcile_person_asks_at(deadline).unwrap(), PersonAskPass { changed: false, next_deadline: Some(effective) });
    smallclaims::store::set_thread_clock(Some(effective - 1));
    assert!(!store.reconcile_person_asks_at(effective - 1).unwrap().changed);
    smallclaims::store::set_thread_clock(Some(effective));
    assert!(store.reconcile_person_asks_at(effective).unwrap().changed);
    assert_eq!(store.step_run(&origin.subject).unwrap().unwrap().person_answers[0].resolved_by.as_deref(), Some("deadline"));
}

/// An answer stored after the ask's stored deadline but before the deadline the person's held
/// clock moved it to is not fenced, and it beats the default the ask's own member applies at the
/// moved deadline, on every member.
#[test]
fn an_answer_after_the_stored_deadline_but_before_the_held_one_beats_the_default() {
    let _clock = SimulatedClock::at(ASKED);
    let (owner, origin, mut input) = fixture();
    owner.set_ask_deadlines(true);
    input.request = Some(gateway_decision());
    input.deadline_ms = Some(HOUR as u64);
    input.default_answer = Some("keep-open".into());
    let ask = owner.ask_person(&input).unwrap();
    let claim = request(&owner.readers.get(), &ask.subject).unwrap().unwrap();
    let deadline = stored_deadline(&owner, &ask.subject);
    // `hold-all` holds even an urgent ask.
    set_clock(&owner, DeadlineClock::HoldAll, Some(ASKED + 40 * MINUTE), ASKED + 10 * MINUTE);
    let effective = deadline + 30 * MINUTE;
    let other = Store::open_memory("birch").unwrap();
    other.set_ask_deadlines(true);
    other.project_replication_backlog().unwrap();
    receive(&owner, &other);
    smallclaims::store::set_thread_clock(Some(deadline + 5 * MINUTE));
    let answer = PersonStepResponse {
        delegation: None,
        subject: ask.subject.clone(),
        actor: "person/avery".into(),
        summary: String::new(),
        evidence: vec![],
        episode: Some(claim.id.clone()),
        idempotency_key: "land-while-held".into(),
        answer: Some(crate::person_request::AnswerInput { id: Some("land".into()), text: None, option_notes: Vec::new() }),
    };
    other.finish_person_step(&answer, false).unwrap();
    // The owner, not holding that answer, applies the default at the moved deadline.
    smallclaims::store::set_thread_clock(Some(effective));
    assert!(owner.reconcile_person_asks_at(effective).unwrap().changed);
    let exchange = |source: &Store, target: &Store| {
        let exchange = source.export_replication_exchange_answering(
            TEST_FLEET, &target.replication_inventory().unwrap(), &[]).unwrap();
        target.receive_replication_exchange(&exchange.peer, TEST_FLEET, &exchange).unwrap();
        target.validate_replication_backlog().unwrap();
        assert!(target.project_replication_backlog().unwrap());
    };
    exchange(&other, &owner);
    exchange(&owner, &other);
    smallclaims::store::set_thread_clock(Some(effective + 1_000));
    assert!(owner.reconcile_person_asks_at(effective + 1_000).unwrap().changed, "the owner tells the asker");
    for store in [&owner, &other] {
        let resumed = store.step_run(&origin.subject).unwrap().unwrap();
        assert_eq!(resumed.person_answers.len(), 1, "{}", store.origin);
        let standing = &resumed.person_answers[0];
        assert_eq!((standing.respondent.as_str(), standing.resolved_by.as_deref()), ("person/avery", None), "{}", store.origin);
        assert_eq!(standing.answer.as_ref().unwrap()["id"], "land");
        assert!(standing.answered_at_unix_ms > deadline && standing.answered_at_unix_ms < effective);
        assert!(resumed.constraints.iter().any(|constraint| constraint.contains("replaces the default")), "{}", store.origin);
    }
    let told = owner.messages(Some("agent/alder.asker"), true).unwrap().into_iter()
        .filter(|message| message.content.contains("before its deadline")).count();
    assert_eq!(told, 1);
}
