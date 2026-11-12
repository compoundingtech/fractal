use super::*;

const ADA: &str = "person/ada";
const T0: u128 = 1_900_000_000_000;
const MINUTE: u128 = 60_000;

struct Clock;
impl Clock {
    fn at(at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

fn store() -> Store {
    Store::open_memory("alder").unwrap()
}

fn report(store: &Store, source: &str, presence: Presence, at: u128) -> PersonAvailability {
    let _clock = Clock::at(at);
    store.report_person_presence(ADA, source, presence, at, at).unwrap()
}

fn set(store: &Store, change: AvailabilityChange, at: u128) -> PersonAvailability {
    let _clock = Clock::at(at);
    store.set_person_availability(ADA, &[change], at).unwrap()
}

#[test]
fn an_override_beats_presence_until_it_expires() {
    let store = store();
    report(&store, "mac", Presence::Present, T0);
    let away = set(&store, AvailabilityChange::Status(Some(Status::Away), Some(T0 + 10 * MINUTE)), T0 + MINUTE);
    assert_eq!((away.status, away.status_source, away.presence), (Status::Away, StatusSource::Override, Some(Presence::Present)));
    assert_eq!(away.status_until, rfc3339(T0 + 10 * MINUTE));
    report(&store, "mac", Presence::Present, T0 + 9 * MINUTE);
    let expired = store.person_availability(ADA, T0 + 10 * MINUTE).unwrap();
    assert_eq!((expired.status, expired.status_source, expired.status_until), (Status::Available, StatusSource::Presence, None));
    set(&store, AvailabilityChange::Status(Some(Status::Away), None), T0 + 11 * MINUTE);
    let cleared = set(&store, AvailabilityChange::Status(None, None), T0 + 12 * MINUTE);
    assert_eq!((cleared.status, cleared.status_source), (Status::Available, StatusSource::Presence));
}

#[test]
fn missing_or_stale_presence_never_counts_as_present() {
    let store = store();
    let nobody = store.person_availability(ADA, T0).unwrap();
    assert_eq!((nobody.status, nobody.presence, nobody.presence_observed_at), (Status::Away, Some(Presence::Unknown), None));
    let present = report(&store, "mac", Presence::Present, T0);
    assert_eq!((present.status, present.status_source, present.presence), (Status::Available, StatusSource::Presence, Some(Presence::Present)));
    let fresh = store.person_availability(ADA, T0 + PRESENCE_STALE_MS - 1).unwrap();
    assert_eq!(fresh.presence, Some(Presence::Present));
    let coarse = fresh.clone().without_presence();
    assert_eq!((coarse.status, coarse.presence, coarse.presence_observed_at), (Status::Available, None, None));
    let stale = store.person_availability(ADA, T0 + PRESENCE_STALE_MS).unwrap();
    assert_eq!((stale.status, stale.presence), (Status::Away, Some(Presence::Unknown)));
    // A report observed long ago, or ahead of this member's clock, is refused.
    let _clock = Clock::at(T0);
    assert!(store.report_person_presence(ADA, "mac", Presence::Present, T0 - PRESENCE_STALE_MS, T0).is_err());
    assert!(store.report_person_presence(ADA, "mac", Presence::Present, T0 + PRESENCE_FUTURE_SKEW_MS + 1, T0).is_err());
    assert!(store.report_person_presence(ADA, "mac", Presence::Unknown, T0, T0).is_err());
}

#[test]
fn every_fresh_source_folds_and_present_anywhere_wins() {
    let store = store();
    report(&store, "phone", Presence::Locked, T0);
    let both = report(&store, "mac", Presence::Present, T0 + MINUTE);
    assert_eq!((both.status, both.presence), (Status::Available, Some(Presence::Present)));
    assert_eq!(both.presence_observed_at, rfc3339(T0 + MINUTE));
    let idle = report(&store, "mac", Presence::Idle, T0 + 2 * MINUTE);
    assert_eq!((idle.status, idle.presence), (Status::Away, Some(Presence::Idle)));
    // Each source counts only through its newest report: once both are stale nothing speaks;
    // a fresh locked report from the phone then folds alone.
    let phone_only = store.person_availability(ADA, T0 + 2 * MINUTE + PRESENCE_STALE_MS).unwrap();
    assert_eq!(phone_only.presence, Some(Presence::Unknown), "the phone's report is older and stale too");
    report(&store, "phone", Presence::Locked, T0 + 10 * MINUTE);
    let phone = store.person_availability(ADA, T0 + 2 * MINUTE + PRESENCE_STALE_MS).unwrap();
    assert_eq!(phone.presence, Some(Presence::Locked));
}

#[test]
fn unchanged_reports_are_recorded_once_per_refresh_interval() {
    let store = store();
    let count = |store: &Store| store.claims_for(ADA, Some(fractal_schema::availability::PRESENCE_KIND)).unwrap().len();
    report(&store, "mac", Presence::Present, T0);
    report(&store, "mac", Presence::Present, T0 + MINUTE);
    assert_eq!(count(&store), 1);
    report(&store, "phone", Presence::Present, T0 + MINUTE);
    assert_eq!(count(&store), 2, "another source records its own report");
    report(&store, "mac", Presence::Idle, T0 + 2 * MINUTE);
    assert_eq!(count(&store), 3, "a change records at once");
    report(&store, "mac", Presence::Idle, T0 + 2 * MINUTE + PRESENCE_REFRESH_MS);
    assert_eq!(count(&store), 4, "an unchanged report refreshes after the interval");
}

#[test]
fn interruptions_and_the_deadline_clock_expire_to_their_defaults() {
    let store = store();
    let initial = store.person_availability(ADA, T0).unwrap();
    assert_eq!((initial.interruptions, initial.deadline_clock), (Interruptions::All, DeadlineClock::Run));
    set(&store, AvailabilityChange::Interruptions(Interruptions::Urgent, Some(T0 + 5 * MINUTE)), T0);
    let held = set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldAll, Some(T0 + 8 * MINUTE)), T0 + MINUTE);
    assert_eq!((held.interruptions, held.deadline_clock), (Interruptions::Urgent, DeadlineClock::HoldAll));
    assert_eq!((held.deadline_clock_since, held.deadline_clock_until), (rfc3339(T0 + MINUTE), rfc3339(T0 + 8 * MINUTE)));
    let later = store.person_availability(ADA, T0 + 8 * MINUTE).unwrap();
    assert_eq!((later.interruptions, later.deadline_clock, later.deadline_clock_since), (Interruptions::All, DeadlineClock::Run, None));
    let _clock = Clock::at(T0);
    assert!(store.set_person_availability(ADA, &[AvailabilityChange::Interruptions(Interruptions::None, Some(T0))], T0).is_err());
    assert!(store.set_person_availability(ADA, &[], T0).is_err());
}

#[test]
fn only_the_person_writes_their_availability() {
    let store = store();
    let _clock = Clock::at(T0);
    assert!(store.set_person_availability("agent/worker", &[AvailabilityChange::Status(Some(Status::Away), None)], T0).is_err());
    let forged = |actor: &str| ClaimInput {
        subject: ADA.into(), kind: STATUS_KIND.into(), actor: Some(actor.into()),
        fields: BTreeMap::from([("status".into(), json!("away"))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    };
    for actor in ["agent/worker", "person/robin"] {
        assert_eq!(store.append_claim(&forged(actor)).unwrap_err().code, "availability-forbidden");
    }
    assert!(store.append_client_claim(&forged(ADA)).is_err(), "the ordinary claim surface cannot write availability");
    assert_eq!(store.person_availability(ADA, T0).unwrap().status_source, StatusSource::Presence);
}

fn hold(start: u128, end: Option<u128>, all: bool) -> ClockHold {
    ClockHold { start, end, all }
}

#[test]
fn held_time_extends_deadlines_hold_routine_spares_urgent_asks() {
    let asked = T0;
    let deadline = T0 + 60 * MINUTE;
    assert_eq!(effective_deadline(asked, deadline, false, &[]), Some(deadline));
    // Held 20 minutes while the clock ran: the deadline moves by exactly the held time.
    let closed = [hold(T0 + 10 * MINUTE, Some(T0 + 30 * MINUTE), false)];
    assert_eq!(effective_deadline(asked, deadline, false, &closed), Some(deadline + 20 * MINUTE));
    assert_eq!(effective_deadline(asked, deadline, true, &closed), Some(deadline), "hold-routine spares urgent asks");
    let all = [hold(T0 + 10 * MINUTE, Some(T0 + 30 * MINUTE), true)];
    assert_eq!(effective_deadline(asked, deadline, true, &all), Some(deadline + 20 * MINUTE), "hold-all holds every ask");
    // An open hold pauses the clock; resuming adds everything held.
    let open = [hold(T0 + 10 * MINUTE, None, true)];
    assert_eq!(effective_deadline(asked, deadline, false, &open), None);
    // A hold after the deadline changes nothing; one spanning it holds only its overlap.
    assert_eq!(effective_deadline(asked, deadline, false, &[hold(deadline, None, true)]), Some(deadline));
    let spanning = [hold(T0 + 50 * MINUTE, Some(T0 + 90 * MINUTE), true)];
    assert_eq!(effective_deadline(asked, deadline, false, &spanning), Some(deadline + 40 * MINUTE));
    let two = [hold(T0 + 5 * MINUTE, Some(T0 + 15 * MINUTE), true), hold(T0 + 20 * MINUTE, Some(T0 + 25 * MINUTE), false)];
    assert_eq!(effective_deadline(asked, deadline, false, &two), Some(deadline + 15 * MINUTE));
    assert!(urgent_ask(&json!({"deadline_ms": 3_600_000})));
    assert!(!urgent_ask(&json!({"deadline_ms": 3_600_001})));
    assert!(!urgent_ask(&json!({"deadline_unix_ms": 1})), "a fleet-default deadline is routine");
}

#[test]
fn the_clock_history_records_holds_since_an_ask_and_resumes() {
    let store = store();
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldRoutine, None), T0);
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldAll, None), T0 + 20 * MINUTE);
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::Run, None), T0 + 30 * MINUTE);
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldAll, Some(T0 + 50 * MINUTE)), T0 + 40 * MINUTE);
    let asked = T0 + 10 * MINUTE;
    let holds = store.deadline_clock_holds(ADA, asked).unwrap();
    assert_eq!(holds, vec![
        hold(asked, Some(T0 + 20 * MINUTE), false),
        hold(T0 + 20 * MINUTE, Some(T0 + 30 * MINUTE), true),
        hold(T0 + 40 * MINUTE, Some(T0 + 50 * MINUTE), true),
    ]);
    let deadline = asked + 60 * MINUTE;
    assert_eq!(effective_deadline(asked, deadline, false, &holds), Some(deadline + 30 * MINUTE));
    assert_eq!(effective_deadline(asked, deadline, true, &holds), Some(deadline + 20 * MINUTE));
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldAll, None), T0 + 60 * MINUTE);
    let open = store.deadline_clock_holds(ADA, asked).unwrap();
    assert_eq!(effective_deadline(asked, deadline, true, &open), None, "an open hold-all pauses the clock");
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::Run, None), T0 + 75 * MINUTE);
    let resumed = store.deadline_clock_holds(ADA, asked).unwrap();
    assert_eq!(effective_deadline(asked, deadline, true, &resumed), Some(deadline + 35 * MINUTE));
}
