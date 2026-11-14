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

/// One instant of availability: status, its source, presence, interruptions, deadline clock.
type Instant = (Status, StatusSource, Presence, Interruptions, DeadlineClock);

fn live(store: &Store, at: u128) -> Instant {
    let view = store.person_availability(ADA, at).unwrap();
    (view.status, view.status_source, view.presence.unwrap(), view.interruptions, view.deadline_clock)
}

/// The one value that held for a nonzero time.
fn held<T: Copy + std::fmt::Debug, const N: usize>(durations: [(u64, T); N]) -> T {
    let held: Vec<T> = durations.iter().filter(|(ms, _)| *ms > 0).map(|(_, value)| *value).collect();
    assert_eq!(held.len(), 1, "{durations:?}");
    held[0]
}

/// The history fold over every claim the store holds, read for the single millisecond at `at`.
fn history_at(store: &Store, at: u128) -> Instant {
    use fractal_schema::availability::{DEADLINE_CLOCK_KIND, INTERRUPTIONS_KIND, PRESENCE_KIND, STATUS_KIND};
    let claims = |kind| store.claims_for(ADA, Some(kind)).unwrap();
    let buckets = history_buckets(&claims(STATUS_KIND), &claims(INTERRUPTIONS_KIND), &claims(DEADLINE_CLOCK_KIND), &claims(PRESENCE_KIND), at, at + 1, 1);
    let [bucket] = &buckets[..] else { panic!("one bucket: {buckets:?}") };
    assert_eq!(bucket.covered_ms, 1);
    let (status, source, presence) = (bucket.status, bucket.status_source, bucket.presence.unwrap());
    (
        held([(status.available, Status::Available), (status.away, Status::Away)]),
        held([(source.overridden, StatusSource::Override), (source.presence, StatusSource::Presence), (source.unreported, StatusSource::Unreported)]),
        held([(presence.present, Presence::Present), (presence.idle, Presence::Idle), (presence.locked, Presence::Locked), (presence.unreported, Presence::Unreported)]),
        held([(bucket.interruptions.all, Interruptions::All), (bucket.interruptions.urgent, Interruptions::Urgent), (bucket.interruptions.none, Interruptions::None)]),
        held([(bucket.deadline_clock.run, DeadlineClock::Run), (bucket.deadline_clock.hold_routine, DeadlineClock::HoldRoutine), (bucket.deadline_clock.hold_all, DeadlineClock::HoldAll)]),
    )
}

/// One write a replay applies at its instant.
type Write = Box<dyn Fn(&Store, u128)>;
type Event = (u128, Write);

#[test]
fn history_agrees_with_the_live_fold_at_every_instant() {
    let store = store();
    let status = |status, until: Option<u128>| -> Write {
        Box::new(move |store: &Store, at: u128| { set(store, AvailabilityChange::Status(status, until.map(|until| at + until)), at); })
    };
    let clock = |clock, until: Option<u128>| -> Write {
        Box::new(move |store: &Store, at: u128| { set(store, AvailabilityChange::DeadlineClock(clock, until.map(|until| at + until)), at); })
    };
    let reporting = |source: &'static str, state| -> Write {
        Box::new(move |store: &Store, at: u128| { report(store, source, state, at); })
    };
    let events: Vec<Event> = vec![
        (T0, reporting("mac", DeviceState::Active)),
        (T0 + 2 * MINUTE, reporting("phone", DeviceState::Locked)),
        (T0 + 5 * MINUTE, Box::new(|store: &Store, at: u128| { set(store, AvailabilityChange::Interruptions(Interruptions::Urgent, Some(at + 35 * MINUTE)), at); })),
        (T0 + 8 * MINUTE, status(Some(Status::Away), Some(12 * MINUTE))),
        (T0 + 12 * MINUTE, clock(DeadlineClock::HoldRoutine, None)),
        // Off the sampling grid, so the idle threshold that follows is too.
        (T0 + 25 * MINUTE + 7_000, reporting("mac", DeviceState::Active)),
        (T0 + 30 * MINUTE, clock(DeadlineClock::HoldAll, Some(15 * MINUTE))),
        (T0 + 31 * MINUTE, status(Some(Status::Away), None)),
        (T0 + 33 * MINUTE, status(None, None)),
        (T0 + 50 * MINUTE, reporting("mac", DeviceState::Inactive)),
        // Observed ten minutes before it was accepted: it counts for a day from when it was observed.
        (T0 + 60 * MINUTE, Box::new(|store: &Store, at: u128| {
            let _clock = Clock::at(at);
            store.report_person_presence(ADA, "tablet", device(DeviceState::Locked, None, at - 10 * MINUTE), at).unwrap();
        })),
        (T0 + 3 * 60 * MINUTE, reporting("phone", DeviceState::Sleeping)),
        (T0 + 4 * 60 * MINUTE, Box::new(|store: &Store, at: u128| { set(store, AvailabilityChange::Interruptions(Interruptions::None, None), at); })),
    ];
    // Replay the writes in order and record what the live read says at each sample, just before
    // and at every half minute, through the end of the last report's day.
    let mut pending = events.iter().peekable();
    let mut seen = Vec::new();
    for grid in (T0 - MINUTE..=T0 + 28 * 60 * MINUTE).step_by(30_000) {
        for at in [grid - 1, grid] {
            while let Some((when, write)) = pending.next_if(|(when, _)| *when <= at) {
                write(&store, *when);
            }
            seen.push((at, live(&store, at)));
        }
    }
    assert!(pending.next().is_none());
    assert!(seen.iter().any(|(_, (_, source, ..))| *source == StatusSource::Unreported), "the day runs out");
    // History, folded after every write, says what the live read said then.
    for (at, expected) in seen {
        assert_eq!(history_at(&store, at), expected, "T0{:+}ms", i128::try_from(at).unwrap() - i128::try_from(T0).unwrap());
    }
}

#[test]
fn history_buckets_split_at_transitions_and_cover_only_the_past() {
    let store = store();
    report(&store, "mac", DeviceState::Active, T0);
    set(&store, AvailabilityChange::Status(Some(Status::Away), Some(T0 + 25 * MINUTE)), T0 + 15 * MINUTE);
    let m = |minutes: u64| minutes * 60_000;
    // Seventy minutes asked, read at T0+55m: the last bucket covers five minutes and none the future.
    let history = store.person_availability_history(ADA, T0 - 10 * MINUTE, T0 + 60 * MINUTE, 600, T0 + 55 * MINUTE).unwrap();
    assert_eq!((history.from.as_str(), history.to.as_str(), history.bucket_seconds), (rfc3339(T0 - 10 * MINUTE).unwrap().as_str(), rfc3339(T0 + 55 * MINUTE).unwrap().as_str(), 600));
    assert_eq!(history.buckets.iter().map(|bucket| bucket.start.clone()).collect::<Vec<_>>(), (0..7).map(|index| rfc3339(T0 - 10 * MINUTE + index * 10 * MINUTE).unwrap()).collect::<Vec<_>>());
    let status = history.buckets.iter()
        .map(|bucket| (bucket.covered_ms, bucket.status.available, bucket.status.away, bucket.status_source.overridden, bucket.status_source.presence, bucket.status_source.unreported))
        .collect::<Vec<_>>();
    assert_eq!(status, vec![
        (m(10), m(10), 0, 0, 0, m(10)),     // before any report: unreported reads available
        (m(10), m(10), 0, 0, m(10), 0),     // present
        (m(10), 0, m(10), m(5), m(5), 0),   // idle at T0+10m, the override from T0+15m
        (m(10), 0, m(10), m(5), m(5), 0),   // the override expires at T0+25m; still idle
        (m(10), 0, m(10), 0, m(10), 0),
        (m(10), 0, m(10), 0, m(10), 0),
        (m(5), 0, m(5), 0, m(5), 0),
    ]);
    let presence = history.buckets.iter()
        .map(|bucket| bucket.presence.map(|presence| (presence.present, presence.idle, presence.locked, presence.unreported)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(presence, vec![(0, 0, 0, m(10)), (m(10), 0, 0, 0), (0, m(10), 0, 0), (0, m(10), 0, 0), (0, m(10), 0, 0), (0, m(10), 0, 0), (0, m(5), 0, 0)]);
    for bucket in &history.buckets {
        assert_eq!((bucket.interruptions.all, bucket.deadline_clock.run), (bucket.covered_ms, bucket.covered_ms));
    }
    assert!(history.without_presence().buckets.iter().all(|bucket| bucket.presence.is_none()));
    // A bucket size that does not divide the range leaves a short last bucket.
    let ragged = store.person_availability_history(ADA, T0, T0 + 25 * MINUTE, 600, T0 + 60 * MINUTE).unwrap();
    assert_eq!(ragged.buckets.iter().map(|bucket| bucket.covered_ms).collect::<Vec<_>>(), vec![m(10), m(10), m(5)]);
    assert_eq!(ragged.to, rfc3339(T0 + 25 * MINUTE).unwrap());
}

#[test]
fn history_refuses_unbounded_reads_and_reads_an_empty_past_as_unreported() {
    let store = store();
    for (from, to, bucket) in [
        (T0, T0, 60),
        (T0 + 1, T0, 60),
        (T0, T0 + MINUTE, 0),
        (T0, T0 + HISTORY_MAX_RANGE_MS + 1_000, 86_400),
        (T0, T0 + 2017 * MINUTE, 60),
    ] {
        assert_eq!(store.person_availability_history(ADA, from, to, bucket, T0).unwrap_err().code, "invalid-availability", "{from} {to} {bucket}");
    }
    assert!(store.person_availability_history("agent/worker", T0 - MINUTE, T0, 60, T0).is_err());
    assert_eq!(store.person_availability_history(ADA, T0, T0 + HISTORY_MAX_RANGE_MS, 86_400, T0 + HISTORY_MAX_RANGE_MS).unwrap().buckets.len(), 31);
    assert_eq!(store.person_availability_history(ADA, T0, T0 + 2016 * MINUTE, 60, T0 + 2016 * MINUTE).unwrap().buckets.len(), 2016);
    // Entirely in the future: nothing to answer yet.
    let ahead = store.person_availability_history(ADA, T0 + MINUTE, T0 + 2 * MINUTE, 60, T0).unwrap();
    assert!(ahead.buckets.is_empty());
    assert_eq!((ahead.from.as_str(), ahead.to.as_str()), (rfc3339(T0 + MINUTE).unwrap().as_str(), rfc3339(T0 + MINUTE).unwrap().as_str()));
    // No claims at all: available and unreported throughout, as the live read says.
    let empty = store.person_availability_history(ADA, T0 - 60 * MINUTE, T0, 600, T0).unwrap();
    assert_eq!(empty.buckets.len(), 6);
    for bucket in &empty.buckets {
        assert_eq!((bucket.status.available, bucket.status_source.unreported, bucket.presence.unwrap().unreported), (600_000, 600_000, 600_000));
    }
    // A device's state counts for a day after it was observed, then the person is unreported.
    report(&store, "mac", DeviceState::Locked, T0);
    let hour = 60 * MINUTE;
    let day = store.person_availability_history(ADA, T0 + 23 * hour, T0 + 25 * hour, 3600, T0 + 30 * hour).unwrap();
    let rows = day.buckets.iter().map(|bucket| (bucket.status.available, bucket.status.away, bucket.status_source.presence, bucket.status_source.unreported)).collect::<Vec<_>>();
    assert_eq!(rows, vec![(0, 3_600_000, 3_600_000, 0), (3_600_000, 0, 0, 3_600_000)]);
}
