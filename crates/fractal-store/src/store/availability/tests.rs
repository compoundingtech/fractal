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

const MAC: &str = "host/mac";

fn device(state: DeviceState, last_input_at: Option<u128>, observed_at: u128) -> DeviceReport {
    DeviceReport { state, last_input_at, observed_at }
}

fn report(store: &Store, source: &str, state: DeviceState, at: u128) -> PresenceReported {
    let _clock = Clock::at(at);
    store.report_person_presence(ADA, source, device(state, Some(at), at), at).unwrap()
}

fn set(store: &Store, change: AvailabilityChange, at: u128) -> PersonAvailability {
    let _clock = Clock::at(at);
    store.set_person_availability(ADA, MAC, &[change], at).unwrap()
}

fn presence_claims(store: &Store) -> usize {
    store.claims_for(ADA, Some(fractal_schema::availability::PRESENCE_KIND)).unwrap().len()
}

#[test]
fn an_override_beats_presence_until_it_expires() {
    let store = store();
    report(&store, "mac", DeviceState::Active, T0);
    let away = set(&store, AvailabilityChange::Status(Some(Status::Away), Some(T0 + 10 * MINUTE)), T0 + MINUTE);
    assert_eq!((away.status, away.status_source, away.presence), (Status::Away, StatusSource::Override, Some(Presence::Present)));
    assert_eq!(away.status_until, rfc3339(T0 + 10 * MINUTE));
    report(&store, "mac", DeviceState::Active, T0 + 9 * MINUTE);
    let expired = store.person_availability(ADA, T0 + 10 * MINUTE).unwrap();
    assert_eq!((expired.status, expired.status_source, expired.status_until), (Status::Available, StatusSource::Presence, None));
    set(&store, AvailabilityChange::Status(Some(Status::Away), None), T0 + 11 * MINUTE);
    let cleared = set(&store, AvailabilityChange::Status(None, None), T0 + 12 * MINUTE);
    assert_eq!((cleared.status, cleared.status_source), (Status::Available, StatusSource::Presence));
}

#[test]
fn every_person_state_claim_names_its_source() {
    let store = store();
    set(&store, AvailabilityChange::DeadlineClock(DeadlineClock::HoldRoutine, None), T0);
    set(&store, AvailabilityChange::Interruptions(Interruptions::Urgent, None), T0 + 1);
    set(&store, AvailabilityChange::Status(None, None), T0 + 2);
    report(&store, "custom/client/pairing-1", DeviceState::Locked, T0 + 3);
    for kind in fractal_schema::availability::KINDS {
        let claims = store.claims_for(ADA, Some(kind)).unwrap();
        assert!(!claims.is_empty(), "{kind}");
        for claim in claims {
            let source = claim.body["fields"]["source"].as_str().unwrap();
            assert_eq!(source, if kind == fractal_schema::availability::PRESENCE_KIND { "custom/client/pairing-1" } else { MAC });
        }
    }
}

#[test]
fn nobody_reporting_reads_available_and_unreported() {
    let store = store();
    let nobody = store.person_availability(ADA, T0).unwrap();
    assert_eq!(
        (nobody.status, nobody.status_source, nobody.presence, nobody.presence_observed_at),
        (Status::Available, StatusSource::Unreported, Some(Presence::Unreported), None)
    );
    report(&store, "mac", DeviceState::Active, T0);
    let present = store.person_availability(ADA, T0 + PRESENCE_IDLE_MS - 1).unwrap();
    assert_eq!((present.status, present.status_source, present.presence), (Status::Available, StatusSource::Presence, Some(Presence::Present)));
    let coarse = present.clone().without_presence();
    assert_eq!((coarse.status, coarse.presence, coarse.presence_observed_at), (Status::Available, None, None));
    // An active device with no input for the threshold is idle, and the person away.
    let idle = store.person_availability(ADA, T0 + PRESENCE_IDLE_MS).unwrap();
    assert_eq!((idle.status, idle.status_source, idle.presence), (Status::Away, StatusSource::Presence, Some(Presence::Idle)));
    // A device's state counts for a day after it was observed; then the person is unreported again.
    let late = store.person_availability(ADA, T0 + PRESENCE_COUNTS_MS - 1).unwrap();
    assert_eq!((late.status, late.status_source), (Status::Away, StatusSource::Presence));
    let gone = store.person_availability(ADA, T0 + PRESENCE_COUNTS_MS).unwrap();
    assert_eq!((gone.status, gone.status_source, gone.presence), (Status::Available, StatusSource::Unreported, Some(Presence::Unreported)));
    // An override still wins over an unreported person.
    let away = set(&store, AvailabilityChange::Status(Some(Status::Away), None), T0 + PRESENCE_COUNTS_MS);
    assert_eq!((away.status, away.status_source), (Status::Away, StatusSource::Override));
}

#[test]
fn the_daemon_derives_presence_from_raw_device_states() {
    let at = |state, last_input_at| device(state, last_input_at, T0).presence_at(T0 + 5 * MINUTE);
    assert_eq!(at(DeviceState::Active, Some(T0)), Some(Presence::Present));
    assert_eq!(at(DeviceState::Active, None), Some(Presence::Present), "an active report without input counts from when it was observed");
    assert_eq!(at(DeviceState::Active, Some(T0 - 6 * MINUTE)), Some(Presence::Idle));
    assert_eq!(at(DeviceState::Inactive, Some(T0)), Some(Presence::Idle));
    assert_eq!(at(DeviceState::Locked, Some(T0)), Some(Presence::Locked));
    assert_eq!(at(DeviceState::Sleeping, None), Some(Presence::Locked));
    assert_eq!(device(DeviceState::Active, Some(T0), T0).idle_at(T0), Some(T0 + PRESENCE_IDLE_MS));
    assert_eq!(device(DeviceState::Inactive, Some(T0), T0).idle_at(T0), None);
    // A report observed long ago, ahead of this member's clock, or with input after it was observed is refused.
    let store = store();
    let _clock = Clock::at(T0);
    for refused in [
        device(DeviceState::Active, None, T0 - PRESENCE_REPORT_MAX_AGE_MS),
        device(DeviceState::Active, None, T0 + PRESENCE_FUTURE_SKEW_MS + 1),
        device(DeviceState::Active, Some(T0 + 1), T0),
        device(DeviceState::Active, Some(0), T0),
    ] {
        assert!(store.report_person_presence(ADA, "mac", refused, T0).is_err(), "{refused:?}");
    }
    assert_eq!(presence_claims(&store), 0);
}

#[test]
fn every_counting_device_folds_and_present_anywhere_wins() {
    let store = store();
    report(&store, "phone", DeviceState::Locked, T0);
    let both = report(&store, "mac", DeviceState::Active, T0 + MINUTE).availability;
    assert_eq!((both.status, both.presence), (Status::Available, Some(Presence::Present)));
    assert_eq!(both.presence_observed_at, rfc3339(T0 + MINUTE));
    let inactive = report(&store, "mac", DeviceState::Inactive, T0 + 2 * MINUTE).availability;
    assert_eq!((inactive.status, inactive.presence), (Status::Away, Some(Presence::Idle)));
    // Each device counts only through its newest report: the mac's sleep replaces its activity.
    report(&store, "mac", DeviceState::Active, T0 + 3 * MINUTE);
    let asleep = report(&store, "mac", DeviceState::Sleeping, T0 + 4 * MINUTE).availability;
    assert_eq!((asleep.status, asleep.presence), (Status::Away, Some(Presence::Locked)));
}

#[test]
fn only_transitions_are_recorded_and_the_daemon_says_when_to_report_again() {
    let store = store();
    let first = report(&store, "mac", DeviceState::Active, T0);
    assert_eq!(presence_claims(&store), 1);
    assert_eq!(first.report_again_at, rfc3339(T0 + PRESENCE_IDLE_MS), "report when the device would turn idle");
    // Input a minute later changes nothing the daemon derives: not recorded, same deadline.
    let soon = report(&store, "mac", DeviceState::Active, T0 + MINUTE);
    assert_eq!((presence_claims(&store), soon.report_again_at), (1, rfc3339(T0 + PRESENCE_IDLE_MS)));
    // Reporting at that instant with newer input keeps the device present and moves the deadline.
    let again = report(&store, "mac", DeviceState::Active, T0 + PRESENCE_IDLE_MS);
    assert_eq!((presence_claims(&store), again.availability.presence), (2, Some(Presence::Present)));
    assert_eq!(again.report_again_at, rfc3339(T0 + 2 * PRESENCE_IDLE_MS));
    // Active without new input past the threshold is idle: nothing to record, nothing to schedule.
    let idle_at = T0 + 3 * PRESENCE_IDLE_MS;
    let idle = {
        let _clock = Clock::at(idle_at);
        store.report_person_presence(ADA, "mac", device(DeviceState::Active, Some(T0 + PRESENCE_IDLE_MS), idle_at), idle_at).unwrap()
    };
    assert_eq!((presence_claims(&store), idle.availability.presence, idle.report_again_at), (2, Some(Presence::Idle), None));
    // Repeated locked reports record once; another device records its own.
    report(&store, "mac", DeviceState::Locked, idle_at + MINUTE);
    report(&store, "mac", DeviceState::Locked, idle_at + 2 * MINUTE);
    assert_eq!(presence_claims(&store), 3);
    report(&store, "phone", DeviceState::Locked, idle_at + 2 * MINUTE);
    assert_eq!(presence_claims(&store), 4);
}

#[test]
fn a_report_observed_before_the_recorded_one_is_not_recorded() {
    let store = store();
    let t2 = T0 + 2 * MINUTE;
    report(&store, "mac", DeviceState::Active, t2);
    assert_eq!(presence_claims(&store), 1);
    // A delayed locked report observed before the recorded activity arrives after it.
    let late = {
        let _clock = Clock::at(t2 + MINUTE);
        store.report_person_presence(ADA, "mac", device(DeviceState::Locked, None, T0), t2 + MINUTE).unwrap()
    };
    assert_eq!(presence_claims(&store), 1);
    assert_eq!((late.availability.status, late.availability.presence), (Status::Available, Some(Presence::Present)));
    assert_eq!(late.availability.presence_observed_at, rfc3339(t2));
    assert_eq!(late.report_again_at, rfc3339(t2 + PRESENCE_IDLE_MS), "the deadline is the recorded activity's");
}

#[test]
fn clock_ahead_reports_keep_their_original_observation_order() {
    let store = store();
    {
        let _clock = Clock::at(T0);
        store.report_person_presence(ADA, "mac", device(DeviceState::Active, Some(T0 + 2 * MINUTE), T0 + 2 * MINUTE), T0).unwrap();
    }
    let late = {
        let now = T0 + 1_000;
        let _clock = Clock::at(now);
        store.report_person_presence(ADA, "mac", device(DeviceState::Locked, None, T0 + MINUTE), now).unwrap()
    };
    assert_eq!(presence_claims(&store), 1);
    assert_eq!((late.availability.status, late.availability.presence), (Status::Available, Some(Presence::Present)));
    assert_eq!(late.availability.presence_observed_at, rfc3339(T0), "presence still clamps the client clock to acceptance");
    assert_eq!(late.report_again_at, rfc3339(T0 + PRESENCE_IDLE_MS));
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
    assert!(store.set_person_availability(ADA, MAC, &[AvailabilityChange::Interruptions(Interruptions::None, Some(T0))], T0).is_err());
    assert!(store.set_person_availability(ADA, MAC, &[], T0).is_err());
}

#[test]
fn only_the_person_writes_their_availability() {
    let store = store();
    let _clock = Clock::at(T0);
    assert!(store.set_person_availability("agent/worker", MAC, &[AvailabilityChange::Status(Some(Status::Away), None)], T0).is_err());
    let forged = |actor: &str| ClaimInput {
        subject: ADA.into(), kind: STATUS_KIND.into(), actor: Some(actor.into()),
        fields: BTreeMap::from([("status".into(), json!("away")), ("source".into(), json!(MAC))]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    };
    for actor in ["agent/worker", "person/robin"] {
        assert_eq!(store.append_claim(&forged(actor)).unwrap_err().code, "availability-forbidden");
    }
    assert!(store.append_client_claim(&forged(ADA)).is_err(), "the ordinary claim surface cannot write availability");
    assert_eq!(store.person_availability(ADA, T0).unwrap().status_source, StatusSource::Unreported);
}

fn hold(start: u128, end: Option<u128>, all: bool) -> ClockHold {
    ClockHold { start, end, all }
}

#[test]
fn held_time_extends_deadlines_hold_routine_spares_critical_asks() {
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
    // Only priority decides urgency; a short deadline does not make an ask urgent.
    assert!(urgent_ask(&json!({"priority": "critical"})));
    assert!(!urgent_ask(&json!({"priority": "high", "deadline_ms": 60_000})));
    assert!(!urgent_ask(&json!({"deadline_ms": 1})), "an ask without a priority is normal");
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
