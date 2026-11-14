//! Person availability, folded at read time from the person's claims. Claims remain
//! authoritative: each axis is its kind's canonical latest claim, presence is derived from each
//! device's newest raw state, and the deadline-clock history is the record of held time.
use super::*;
use fractal_schema::availability::{
    DEADLINE_CLOCK_KIND, INTERRUPTIONS_KIND, PRESENCE_KIND, STATUS_KIND,
};

/// An `active` device is present until this long after its last input, then idle. Every
/// device gets the same threshold; Boss used ten minutes of HID idle time.
pub const PRESENCE_IDLE_MS: u128 = 10 * 60_000;
/// A device's newest state counts for this long after it was observed. When no device's does,
/// the person is unreported: available, so agents ask as usual.
pub const PRESENCE_COUNTS_MS: u128 = 24 * 60 * 60_000;
/// An active device's unchanged report is recorded again only once it moves the last input
/// forward by this much, so frequent reports do not grow the log.
pub const PRESENCE_REFRESH_MS: u128 = 5 * 60_000;
/// A report must have been observed within this long of arriving.
pub const PRESENCE_REPORT_MAX_AGE_MS: u128 = 15 * 60_000;
/// Client clocks may run ahead of this member's by at most this much.
pub const PRESENCE_FUTURE_SKEW_MS: u128 = 2 * 60_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status { Available, Away }

/// Where the status comes from: an override the person set, their devices' presence, or neither
/// (no device has reported within [`PRESENCE_COUNTS_MS`]), which reads as available.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusSource { Override, Presence, Unreported }

/// Presence the daemon derives from device states; ordered so the first wins a fold.
/// `Unreported` when no device's state counts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Presence { Present, Idle, Locked, Unreported }

/// A device's raw state, as the device reports it. It applies no thresholds of its own.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceState { Active, Inactive, Locked, Sleeping }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Interruptions { All, Urgent, None }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeadlineClock { Run, HoldRoutine, HoldAll }

/// The effective availability of one person at one instant. Times are RFC 3339 UTC.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonAvailability {
    pub person: String,
    pub status: Status,
    pub status_source: StatusSource,
    pub status_until: Option<String>,
    /// The presence derived from the person's devices, for the person's own reads. Other
    /// readers get `None`: they see the effective status, never presence.
    pub presence: Option<Presence>,
    pub presence_observed_at: Option<String>,
    pub interruptions: Interruptions,
    pub interruptions_until: Option<String>,
    pub deadline_clock: DeadlineClock,
    pub deadline_clock_since: Option<String>,
    pub deadline_clock_until: Option<String>,
}

impl PersonAvailability {
    /// The view for anyone but the person: effective values only, no raw presence.
    #[must_use]
    pub fn without_presence(self) -> Self {
        Self { presence: None, presence_observed_at: None, ..self }
    }
}

/// One axis change; each becomes one claim authored by the person.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvailabilityChange {
    /// `None` clears the override, so presence derives status again.
    Status(Option<Status>, Option<u128>),
    Interruptions(Interruptions, Option<u128>),
    DeadlineClock(DeadlineClock, Option<u128>),
}

/// One device's raw state as reported: when it was observed and, for input, when the person
/// last touched it. Times are this member's Unix milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceReport {
    pub state: DeviceState,
    pub last_input_at: Option<u128>,
    pub observed_at: u128,
}

impl DeviceReport {
    /// A recorded report, observed no later than it was accepted.
    fn from_claim(claim: &ClaimRecord) -> Option<Self> {
        let fields = &claim.body["fields"];
        let observed_at = fields["observed_at_unix_ms"].as_u64().map_or(0, u128::from).min(claim.accepted_at_unix_ms);
        Some(Self {
            state: parse(fields.get("state"))?,
            last_input_at: fields["last_input_at_unix_ms"].as_u64().map(|at| u128::from(at).min(observed_at)),
            observed_at,
        })
    }

    /// When an active device's person last acted on it: its last input, else when it was observed.
    fn last_input(&self) -> u128 {
        self.last_input_at.unwrap_or(self.observed_at)
    }

    /// The presence this report gives at `now`, or `None` once it no longer counts.
    pub fn presence_at(&self, now: u128) -> Option<Presence> {
        if now.saturating_sub(self.observed_at) >= PRESENCE_COUNTS_MS {
            return None;
        }
        Some(match self.state {
            DeviceState::Active if now.saturating_sub(self.last_input()) < PRESENCE_IDLE_MS => Presence::Present,
            DeviceState::Active | DeviceState::Inactive => Presence::Idle,
            DeviceState::Locked | DeviceState::Sleeping => Presence::Locked,
        })
    }

    /// When a device that is present at `now` turns idle unless it reports newer input.
    pub fn idle_at(&self, now: u128) -> Option<u128> {
        (self.presence_at(now) == Some(Presence::Present)).then(|| self.last_input() + PRESENCE_IDLE_MS)
    }
}

/// The presence folded over a person's devices at one instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldedPresence {
    pub presence: Presence,
    /// When the device that gives `presence` was observed.
    pub observed_at: Option<u128>,
    /// Whether any device's state still counts; without one the person is unreported.
    pub reported: bool,
}

/// What a presence report returns: the person's availability, and when the reporting device
/// should report again even if its state has not changed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PresenceReported {
    pub availability: PersonAvailability,
    /// While the device is present: the instant it turns idle unless it reports newer input,
    /// so it reports then with its current last input. `None`: report on the next state change
    /// or the next input.
    pub report_again_at: Option<String>,
}

/// An interval during which a person's deadline clock was held; `end` is `None` while open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockHold {
    pub start: u128,
    pub end: Option<u128>,
    /// `hold-all`; otherwise `hold-routine`.
    pub all: bool,
}

fn word<T: Serialize>(value: T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(value: Option<&Value>) -> Option<T> {
    value.and_then(|value| serde_json::from_value(value.clone()).ok())
}

pub fn rfc3339(at: u128) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(i64::try_from(at).ok()?)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn until(claim: &ClaimRecord) -> Option<u128> {
    claim.body["fields"]["until_unix_ms"].as_u64().map(u128::from)
}

/// The claim when its value has not expired at `now`.
fn active(claim: Option<ClaimRecord>, now: u128) -> Option<ClaimRecord> {
    claim.filter(|claim| until(claim).is_none_or(|until| now < until))
}

/// The status an override claim sets; `None` for a claim that clears the override.
fn override_status(claim: &ClaimRecord) -> Option<Status> {
    parse(claim.body["fields"].get("status"))
}

fn interruptions_of(claim: &ClaimRecord) -> Option<Interruptions> {
    parse(claim.body["fields"].get("interruptions"))
}

fn deadline_clock_of(claim: &ClaimRecord) -> Option<DeadlineClock> {
    parse(claim.body["fields"].get("deadline_clock"))
}

/// The one status rule: an override in force wins; otherwise `available` while some device is
/// present, `away` while devices report but none is present, and `available` when no device's
/// state counts. Nobody reporting is not evidence of absence.
fn effective_status(status_override: Option<Status>, folded: FoldedPresence) -> (Status, StatusSource) {
    match (status_override, folded) {
        (Some(status), _) => (status, StatusSource::Override),
        (None, FoldedPresence { reported: false, .. }) => (Status::Available, StatusSource::Unreported),
        (None, FoldedPresence { presence: Presence::Present, .. }) => (Status::Available, StatusSource::Presence),
        (None, _) => (Status::Away, StatusSource::Presence),
    }
}

/// An ask is urgent when its asker gave it priority `critical`; see
/// [`fractal_schema::availability::urgent_priority`].
pub fn urgent_ask(ask_fields: &Value) -> bool {
    fractal_schema::availability::urgent_priority(ask_fields["priority"].as_str())
}

/// The deadline an ask asked at `asked_at` with stored `deadline` really has: the stored
/// deadline plus every held interval since it was asked, counting `hold-routine` only for a
/// routine ask. `None` while a hold that applies is open with no end.
pub fn effective_deadline(asked_at: u128, deadline: u128, urgent: bool, holds: &[ClockHold]) -> Option<u128> {
    let mut cursor = asked_at.min(deadline);
    let mut remaining = deadline - cursor;
    for hold in holds.iter().filter(|hold| hold.all || !urgent) {
        let end = hold.end.map_or(u128::MAX, |end| end);
        if end <= cursor {
            continue;
        }
        let start = hold.start.max(cursor);
        if cursor + remaining <= start {
            break;
        }
        remaining -= start - cursor;
        hold.end?;
        cursor = end;
    }
    Some(cursor + remaining)
}

/// Fold a person's deadline-clock claims, newest first, into the holds in force since `since`.
/// The walk stops at the first claim accepted at or before `since`: the value then in force.
pub fn clock_holds(newest_first: &[ClaimRecord], since: u128) -> Vec<ClockHold> {
    let mut holds = Vec::new();
    let mut next_change: Option<u128> = None;
    for claim in newest_first {
        let mode = parse::<DeadlineClock>(claim.body["fields"].get("deadline_clock"));
        let start = claim.accepted_at_unix_ms;
        let end = match (until(claim), next_change) {
            (Some(until), Some(next)) => Some(until.min(next)),
            (until, next) => until.or(next),
        };
        if mode.is_some_and(|mode| mode != DeadlineClock::Run) && end.is_none_or(|end| end > since.max(start)) {
            holds.push(ClockHold { start: start.max(since), end, all: mode == Some(DeadlineClock::HoldAll) });
        }
        next_change = Some(start);
        if start <= since {
            break;
        }
    }
    holds.reverse();
    holds
}

/// A person's deadline-clock holds since `since`, read through `connection`: inside the
/// caller's transaction or snapshot, so the holds agree with the asks it reads beside them.
pub fn deadline_clock_holds_in(connection: &Connection, person: &str, since: u128) -> rusqlite::Result<Vec<ClockHold>> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| smallclaims::store::claims_in_order_query(true));
    smallclaims::touched::note_read(|| person.to_owned());
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h0ce7795286d6 owner=fractal_store::store::availability TODO=trace-statement-and-value-representation */ &QUERY)?;
    let mut rows = statement.query(params![person, DEADLINE_CLOCK_KIND])?;
    let mut claims = Vec::new();
    while let Some(row) = rows.next()? {
        let claim = claim_from_row(row)?;
        let more = claim.accepted_at_unix_ms > since;
        claims.push(claim);
        if !more {
            break;
        }
    }
    Ok(clock_holds(&claims, since))
}

/// Each device's newest report, folded at `now`: `present` anywhere wins, then `idle`, then
/// `locked`. A device whose newest report no longer counts says nothing, and older reports from
/// it never speak for it.
pub fn fold_presence(newest_first: &[ClaimRecord], now: u128) -> FoldedPresence {
    let mut seen = BTreeSet::new();
    fold_reports(
        newest_first.iter()
            .filter(|claim| claim.body["fields"]["source"].as_str().is_some_and(|source| seen.insert(source.to_owned())))
            .filter_map(DeviceReport::from_claim),
        now,
    )
}

/// Fold each device's newest report at `now`; see [`fold_presence`].
fn fold_reports(newest_per_device: impl IntoIterator<Item = DeviceReport>, now: u128) -> FoldedPresence {
    let mut folded = FoldedPresence { presence: Presence::Unreported, observed_at: None, reported: false };
    for report in newest_per_device {
        let Some(presence) = report.presence_at(now) else { continue };
        folded.reported = true;
        if presence < folded.presence || (presence == folded.presence && folded.observed_at.is_none_or(|at| report.observed_at > at)) {
            folded.presence = presence;
            folded.observed_at = Some(report.observed_at);
        }
    }
    folded
}

/// The longest range one history read covers, and the most buckets it answers.
pub const HISTORY_MAX_RANGE_MS: u128 = 31 * 24 * 60 * 60_000;
pub const HISTORY_MAX_BUCKETS: u128 = 2016;

/// Milliseconds spent in each status during a bucket.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusDurations { pub available: u64, pub away: u64 }

/// Milliseconds the status came from each source during a bucket.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusSourceDurations {
    #[serde(rename = "override")]
    pub overridden: u64,
    pub presence: u64,
    pub unreported: u64,
}

/// Milliseconds of each derived presence during a bucket.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PresenceDurations { pub present: u64, pub idle: u64, pub locked: u64, pub unreported: u64 }

/// Milliseconds of each interruptions setting during a bucket.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterruptionsDurations { pub all: u64, pub urgent: u64, pub none: u64 }

/// Milliseconds of each deadline-clock mode during a bucket; keys are the wire words.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeadlineClockDurations {
    pub run: u64,
    #[serde(rename = "hold-routine")]
    pub hold_routine: u64,
    #[serde(rename = "hold-all")]
    pub hold_all: u64,
}

/// One bucket of availability history. Each axis's durations sum to `covered_ms`, the part of
/// the bucket that lies within the answered range.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryBucket {
    pub start: String,
    pub covered_ms: u64,
    pub status: StatusDurations,
    pub status_source: StatusSourceDurations,
    /// The person's own read only; `None` for anyone else, like [`PersonAvailability::presence`].
    pub presence: Option<PresenceDurations>,
    pub interruptions: InterruptionsDurations,
    pub deadline_clock: DeadlineClockDurations,
}

/// A person's availability over `[from, to)` in buckets of `bucket_seconds` starting at `from`,
/// folded from the same claims and by the same rules as [`PersonAvailability`] at each instant.
/// `to` is the requested end clamped to the read's instant, so no bucket covers the future.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AvailabilityHistory {
    pub person: String,
    pub from: String,
    pub to: String,
    pub bucket_seconds: u64,
    pub buckets: Vec<HistoryBucket>,
}

impl AvailabilityHistory {
    /// The view for anyone but the person: effective values only, no presence.
    #[must_use]
    pub fn without_presence(mut self) -> Self {
        for bucket in &mut self.buckets {
            bucket.presence = None;
        }
        self
    }
}

/// One kind's claims, oldest first, swept forward in time.
struct Sweep<'a> {
    claims: &'a [ClaimRecord],
    next: usize,
}

impl<'a> Sweep<'a> {
    fn new(claims: &'a [ClaimRecord]) -> Self {
        Self { claims, next: 0 }
    }

    /// The next claim accepted at or before `at` that the sweep has not passed yet.
    fn pass(&mut self, at: u128) -> Option<&'a ClaimRecord> {
        let claim = self.claims.get(self.next).filter(|claim| claim.accepted_at_unix_ms <= at)?;
        self.next += 1;
        Some(claim)
    }

    /// The claim in force at `at`: the newest accepted by then, while its value has not expired.
    fn in_force(&mut self, at: u128) -> Option<&'a ClaimRecord> {
        while self.pass(at).is_some() {}
        let claims = self.claims;
        self.next.checked_sub(1)
            .and_then(|index| claims.get(index))
            .filter(|claim| until(claim).is_none_or(|until| at < until))
    }

    fn next_accepted(&self) -> Option<u128> {
        self.claims.get(self.next).map(|claim| claim.accepted_at_unix_ms)
    }

    /// The first instant after `at` when this axis can change: its next claim, or the expiry
    /// of the newest claim so far.
    fn next_change(&self, at: u128) -> Option<u128> {
        let expiry = self.next.checked_sub(1).and_then(|index| until(&self.claims[index])).filter(|until| *until > at);
        expiry.into_iter().chain(self.next_accepted()).min()
    }
}

fn add(slot: &mut u64, ms: u128) {
    *slot = slot.saturating_add(u64::try_from(ms).unwrap_or(u64::MAX));
}

/// Fold one person's availability claims, each kind oldest first, into buckets of `bucket_ms`
/// over `[from, end)`. Each kind's claims must reach back to the one in force at `from`, and
/// the presence reports to every report accepted after `from - PRESENCE_COUNTS_MS`. At every
/// instant the fold applies exactly the rules [`AvailabilityStore::person_availability`] applies
/// to the claims accepted by then, sweeping from one change to the next: a claim's acceptance,
/// an `until`, an active device turning idle, or a report ceasing to count.
pub fn history_buckets(
    status: &[ClaimRecord],
    interruptions: &[ClaimRecord],
    deadline_clock: &[ClaimRecord],
    presence: &[ClaimRecord],
    from: u128,
    end: u128,
    bucket_ms: u128,
) -> Vec<HistoryBucket> {
    let bucket_ms = bucket_ms.max(1);
    let mut buckets: Vec<HistoryBucket> = (0..end.saturating_sub(from).div_ceil(bucket_ms))
        .map(|index| {
            let start = from + index * bucket_ms;
            HistoryBucket {
                start: rfc3339(start).unwrap_or_default(),
                covered_ms: u64::try_from((start + bucket_ms).min(end) - start).unwrap_or(u64::MAX),
                status: StatusDurations::default(),
                status_source: StatusSourceDurations::default(),
                presence: Some(PresenceDurations::default()),
                interruptions: InterruptionsDurations::default(),
                deadline_clock: DeadlineClockDurations::default(),
            }
        })
        .collect();
    let (mut status, mut interruptions, mut deadline_clock, mut reports) =
        (Sweep::new(status), Sweep::new(interruptions), Sweep::new(deadline_clock), Sweep::new(presence));
    // Each device's newest report so far, as the live fold reads it.
    let mut devices: BTreeMap<&str, DeviceReport> = BTreeMap::new();
    let mut at = from;
    'sweep: while at < end {
        while let Some(claim) = reports.pass(at) {
            let Some(source) = claim.body["fields"]["source"].as_str() else { continue };
            // A newest report the live fold cannot read silences its device there too.
            match DeviceReport::from_claim(claim) {
                Some(report) => devices.insert(source, report),
                None => devices.remove(source),
            };
        }
        let folded = fold_reports(devices.values().copied(), at);
        let status_override = status.in_force(at)
            .filter(|claim| !claim.body["fields"]["status"].is_null())
            .and_then(override_status);
        let (state, source) = effective_status(status_override, folded);
        let interrupt = interruptions.in_force(at).and_then(interruptions_of).unwrap_or(Interruptions::All);
        let clock = deadline_clock.in_force(at).and_then(deadline_clock_of).unwrap_or(DeadlineClock::Run);
        let device_change = devices.values()
            .flat_map(|report| {
                let idle = (report.state == DeviceState::Active).then(|| report.last_input() + PRESENCE_IDLE_MS);
                idle.into_iter().chain([report.observed_at + PRESENCE_COUNTS_MS])
            })
            .filter(|change| *change > at)
            .min();
        let next = [status.next_change(at), interruptions.next_change(at), deadline_clock.next_change(at), reports.next_accepted(), device_change]
            .into_iter()
            .flatten()
            .fold(end, u128::min);
        while at < next {
            let index = (at - from) / bucket_ms;
            let span = next.min(from + (index + 1) * bucket_ms) - at;
            // Every instant before `end` lies in one of the buckets made above.
            let Some(bucket) = usize::try_from(index).ok().and_then(|index| buckets.get_mut(index)) else { break 'sweep };
            match state {
                Status::Available => add(&mut bucket.status.available, span),
                Status::Away => add(&mut bucket.status.away, span),
            }
            match source {
                StatusSource::Override => add(&mut bucket.status_source.overridden, span),
                StatusSource::Presence => add(&mut bucket.status_source.presence, span),
                StatusSource::Unreported => add(&mut bucket.status_source.unreported, span),
            }
            if let Some(durations) = bucket.presence.as_mut() {
                match folded.presence {
                    Presence::Present => add(&mut durations.present, span),
                    Presence::Idle => add(&mut durations.idle, span),
                    Presence::Locked => add(&mut durations.locked, span),
                    Presence::Unreported => add(&mut durations.unreported, span),
                }
            }
            match interrupt {
                Interruptions::All => add(&mut bucket.interruptions.all, span),
                Interruptions::Urgent => add(&mut bucket.interruptions.urgent, span),
                Interruptions::None => add(&mut bucket.interruptions.none, span),
            }
            match clock {
                DeadlineClock::Run => add(&mut bucket.deadline_clock.run, span),
                DeadlineClock::HoldRoutine => add(&mut bucket.deadline_clock.hold_routine, span),
                DeadlineClock::HoldAll => add(&mut bucket.deadline_clock.hold_all, span),
            }
            at += span;
        }
    }
    buckets
}

pub trait AvailabilityStore {
    fn person_availability(&self, person: &str, now: u128) -> Result<PersonAvailability, FractalError>;
    /// Each change becomes one claim naming `source`, the daemon-assigned writer.
    fn set_person_availability(&self, person: &str, source: &str, changes: &[AvailabilityChange], now: u128) -> Result<PersonAvailability, FractalError>;
    /// Records `report` only when it is a transition for `source`'s device. A report observed
    /// before the device's newest recorded report is never recorded: it answers as that newer
    /// report, so a delayed or retried older state cannot replace it.
    fn report_person_presence(&self, person: &str, source: &str, report: DeviceReport, now: u128) -> Result<PresenceReported, FractalError>;
    fn deadline_clock_holds(&self, person: &str, since: u128) -> Result<Vec<ClockHold>>;
    /// The person's availability over `[from, to)` in buckets of `bucket_seconds`, with `to`
    /// clamped to `now`. Refuses an empty range, a range over [`HISTORY_MAX_RANGE_MS`] and more
    /// than [`HISTORY_MAX_BUCKETS`] buckets.
    fn person_availability_history(&self, person: &str, from: u128, to: u128, bucket_seconds: u64, now: u128) -> Result<AvailabilityHistory, FractalError>;
}

fn person_subject(person: &str) -> Result<(), FractalError> {
    fractal_schema::availability::validate_actor(person, STATUS_KIND, Some(person))
        .map_err(|error| FractalError::new(error.code, error.message))
}

impl Store {
    /// The person's device reports accepted within the last [`PRESENCE_COUNTS_MS`], newest
    /// first: every report that can still count, since one is observed no later than accepted.
    fn counting_presence(&self, person: &str, now: u128) -> Result<Vec<ClaimRecord>> {
        let mut claims = Vec::new();
        self.claims_in_order(person, PRESENCE_KIND, true, |claim| {
            let recent = now.saturating_sub(claim.accepted_at_unix_ms) < PRESENCE_COUNTS_MS;
            if recent {
                claims.push(claim.clone());
            }
            recent
        })?;
        Ok(claims)
    }

    /// The person's claims of `kind` accepted before `end`, oldest first: every claim accepted
    /// after `since`, and with `with_prior` also the newest accepted at or before it, the value
    /// then in force.
    fn history_claims(&self, person: &str, kind: &str, since: u128, end: u128, with_prior: bool) -> Result<Vec<ClaimRecord>> {
        let mut claims = Vec::new();
        self.claims_in_order(person, kind, true, |claim| {
            let at = claim.accepted_at_unix_ms;
            if at < end && (at > since || with_prior) {
                claims.push(claim.clone());
            }
            at > since
        })?;
        claims.reverse();
        Ok(claims)
    }
}

impl AvailabilityStore for Store {
    fn person_availability(&self, person: &str, now: u128) -> Result<PersonAvailability, FractalError> {
        person_subject(person)?;
        self.read_snapshot(|_| {
            let status = active(self.latest_claim(person, Some(STATUS_KIND))?, now)
                .filter(|claim| !claim.body["fields"]["status"].is_null());
            let interruptions = active(self.latest_claim(person, Some(INTERRUPTIONS_KIND))?, now);
            let clock = active(self.latest_claim(person, Some(DEADLINE_CLOCK_KIND))?, now)
                .filter(|claim| claim.body["fields"]["deadline_clock"] != "run");
            let folded = fold_presence(&self.counting_presence(person, now)?, now);
            let (status_value, status_source) = effective_status(status.as_ref().and_then(override_status), folded);
            Ok(PersonAvailability {
                person: person.to_owned(),
                status: status_value,
                status_source,
                status_until: status.as_ref().and_then(until).and_then(rfc3339),
                presence: Some(folded.presence),
                presence_observed_at: folded.observed_at.and_then(rfc3339),
                interruptions: interruptions.as_ref().and_then(interruptions_of).unwrap_or(Interruptions::All),
                interruptions_until: interruptions.as_ref().and_then(until).and_then(rfc3339),
                deadline_clock: clock.as_ref().and_then(deadline_clock_of).unwrap_or(DeadlineClock::Run),
                deadline_clock_since: clock.as_ref().and_then(|claim| rfc3339(claim.accepted_at_unix_ms)),
                deadline_clock_until: clock.as_ref().and_then(until).and_then(rfc3339),
            })
        }).map_err(internal)
    }

    fn person_availability_history(&self, person: &str, from: u128, to: u128, bucket_seconds: u64, now: u128) -> Result<AvailabilityHistory, FractalError> {
        person_subject(person)?;
        let bucket_ms = u128::from(bucket_seconds) * 1000;
        if bucket_ms == 0 || from >= to {
            return Err(FractalError::new("invalid-availability", "from must be before to, and bucket_seconds positive"));
        }
        if to - from > HISTORY_MAX_RANGE_MS {
            return Err(FractalError::new("invalid-availability", "an availability history covers at most 31 days"));
        }
        if (to - from).div_ceil(bucket_ms) > HISTORY_MAX_BUCKETS {
            return Err(FractalError::new("invalid-availability", format!("an availability history answers at most {HISTORY_MAX_BUCKETS} buckets")));
        }
        // Nothing is known about the future: the answer ends at the read's instant.
        let end = to.min(now).max(from);
        self.read_snapshot(|_| {
            let axis = |kind| self.history_claims(person, kind, from, end, true);
            // A report accepted a day or more before `from` cannot count at `from` or later.
            let presence = self.history_claims(person, PRESENCE_KIND, from.saturating_sub(PRESENCE_COUNTS_MS), end, false)?;
            Ok(AvailabilityHistory {
                person: person.to_owned(),
                from: rfc3339(from).unwrap_or_default(),
                to: rfc3339(end).unwrap_or_default(),
                bucket_seconds,
                buckets: history_buckets(&axis(STATUS_KIND)?, &axis(INTERRUPTIONS_KIND)?, &axis(DEADLINE_CLOCK_KIND)?, &presence, from, end, bucket_ms),
            })
        }).map_err(internal)
    }

    fn set_person_availability(&self, person: &str, source: &str, changes: &[AvailabilityChange], now: u128) -> Result<PersonAvailability, FractalError> {
        person_subject(person)?;
        if changes.is_empty() {
            return Err(FractalError::new("invalid-availability", "name at least one of status, interruptions or deadline_clock"));
        }
        let mut inputs = Vec::with_capacity(changes.len());
        for change in changes {
            let (kind, name, value, until) = match *change {
                AvailabilityChange::Status(status, until) => (STATUS_KIND, "status", status.map(word), until),
                AvailabilityChange::Interruptions(value, until) => (INTERRUPTIONS_KIND, "interruptions", Some(word(value)), until),
                AvailabilityChange::DeadlineClock(value, until) => (DEADLINE_CLOCK_KIND, "deadline_clock", Some(word(value)), until),
            };
            if until.is_some_and(|until| until <= now) {
                return Err(FractalError::new("invalid-availability", format!("{name} until must be in the future")));
            }
            let mut fields = BTreeMap::from([(name.to_owned(), json!(value)), ("source".to_owned(), json!(source))]);
            if let Some(until) = until {
                fields.insert("until_unix_ms".into(), json!(until));
            }
            inputs.push(ClaimInput {
                subject: person.into(), kind: kind.into(), actor: Some(person.into()), fields,
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            });
        }
        for input in &inputs {
            self.append_claim(input)?;
        }
        self.person_availability(person, now)
    }

    fn report_person_presence(&self, person: &str, source: &str, report: DeviceReport, now: u128) -> Result<PresenceReported, FractalError> {
        person_subject(person)?;
        if report.observed_at > now + PRESENCE_FUTURE_SKEW_MS || now.saturating_sub(report.observed_at) >= PRESENCE_REPORT_MAX_AGE_MS {
            return Err(FractalError::new("invalid-presence", "observed_at must be within the last 15 minutes and at most 2 minutes ahead of this member's clock"));
        }
        if report.last_input_at.is_some_and(|at| at == 0 || at > report.observed_at) {
            return Err(FractalError::new("invalid-presence", "last_input_at must be positive and no later than observed_at"));
        }
        // Record transitions only: a new state, a different derived presence, or an active
        // device's input moving forward far enough to matter. Anything else repeats the record,
        // as does a report observed before the recorded one, which must not become the newest.
        let recorded = self.counting_presence(person, now).map_err(internal)?
            .iter()
            .find(|claim| claim.body["fields"]["source"] == source)
            .and_then(|claim| {
                let observed_at = u128::from(claim.body["fields"]["observed_at_unix_ms"].as_u64()?);
                DeviceReport::from_claim(claim).map(|report| (observed_at, report))
            });
        let changes = recorded.is_none_or(|(observed_at, recorded)| {
            report.observed_at >= observed_at
                && (recorded.state != report.state
                    || recorded.presence_at(now) != report.presence_at(now)
                    || (report.state == DeviceState::Active
                        && report.last_input().saturating_sub(recorded.last_input()) >= PRESENCE_REFRESH_MS))
        });
        if changes {
            let mut fields = BTreeMap::from([
                ("state".into(), json!(word(report.state))),
                ("source".into(), json!(source)),
                ("observed_at_unix_ms".into(), json!(report.observed_at)),
            ]);
            if let Some(at) = report.last_input_at {
                fields.insert("last_input_at_unix_ms".into(), json!(at));
            }
            self.append_claim(&ClaimInput {
                subject: person.into(), kind: PRESENCE_KIND.into(), actor: Some(person.into()), fields,
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            })?;
        }
        let effective = if changes { Some(report) } else { recorded.map(|(_, report)| report) };
        Ok(PresenceReported {
            availability: self.person_availability(person, now)?,
            report_again_at: effective.and_then(|report| report.idle_at(now)).and_then(rfc3339),
        })
    }

    fn deadline_clock_holds(&self, person: &str, since: u128) -> Result<Vec<ClockHold>> {
        Ok(deadline_clock_holds_in(&self.readers.get(), person, since)?)
    }
}

#[cfg(test)]
mod tests;
