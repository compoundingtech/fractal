//! Person availability, folded at read time from the person's claims. Claims remain
//! authoritative: each axis is its kind's canonical latest claim, presence is each source's
//! newest fresh report, and the deadline-clock history is the record of held time.
use super::*;
use fractal_schema::availability::{
    DEADLINE_CLOCK_KIND, INTERRUPTIONS_KIND, PRESENCE_KIND, STATUS_KIND,
};

/// A report older than this no longer says anything about the person.
pub const PRESENCE_STALE_MS: u128 = 15 * 60_000;
/// An unchanged report from one source is recorded again only after this long, so a client
/// may report on every change and as a heartbeat without one claim per heartbeat.
pub const PRESENCE_REFRESH_MS: u128 = 5 * 60_000;
/// Client clocks may run ahead of this member's by at most this much.
pub const PRESENCE_FUTURE_SKEW_MS: u128 = 2 * 60_000;
/// An ask is urgent when its asker named an explicit deadline of at most this long.
/// `hold-routine` leaves urgent asks' clocks running.
pub const URGENT_ASK_DEADLINE_MS: u64 = 60 * 60_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status { Available, Away }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusSource { Override, Presence }

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Presence { Present, Idle, Locked, Unknown }

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
    /// The folded device presence, for the person's own reads. Other readers get `None`:
    /// they see the effective status, never raw presence.
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

/// An ask is urgent when its asker named an explicit deadline of at most an hour.
pub fn urgent_ask(ask_fields: &Value) -> bool {
    ask_fields["deadline_ms"].as_u64().is_some_and(|ms| ms <= URGENT_ASK_DEADLINE_MS)
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

/// Each source's newest report, folded: any fresh `present` wins, then `idle`, then `locked`.
/// Returns the folded presence and the observation time of the newest report that has it.
pub fn fold_presence(newest_first: &[ClaimRecord], now: u128) -> (Presence, Option<u128>) {
    let mut seen = BTreeSet::new();
    let mut best: (Presence, Option<u128>) = (Presence::Unknown, None);
    for claim in newest_first {
        let fields = &claim.body["fields"];
        let Some(source) = fields["source"].as_str() else { continue };
        if !seen.insert(source.to_owned()) {
            continue;
        }
        let observed = fields["observed_at_unix_ms"].as_u64().map_or(0, u128::from).min(claim.accepted_at_unix_ms);
        let Some(presence) = parse::<Presence>(fields.get("presence")) else { continue };
        if now.saturating_sub(observed) >= PRESENCE_STALE_MS {
            continue;
        }
        if presence < best.0 || (presence == best.0 && best.1.is_none_or(|at| observed > at)) {
            best = (presence, Some(observed));
        }
    }
    best
}

pub trait AvailabilityStore {
    fn person_availability(&self, person: &str, now: u128) -> Result<PersonAvailability, FractalError>;
    fn set_person_availability(&self, person: &str, changes: &[AvailabilityChange], now: u128) -> Result<PersonAvailability, FractalError>;
    fn report_person_presence(&self, person: &str, source: &str, presence: Presence, observed_at: u128, now: u128) -> Result<PersonAvailability, FractalError>;
    fn deadline_clock_holds(&self, person: &str, since: u128) -> Result<Vec<ClockHold>>;
}

fn person_subject(person: &str) -> Result<(), FractalError> {
    fractal_schema::availability::validate_actor(person, STATUS_KIND, Some(person))
        .map_err(|error| FractalError::new(error.code, error.message))
}

impl Store {
    fn recent_presence(&self, person: &str, window: u128, now: u128) -> Result<Vec<ClaimRecord>> {
        let mut claims = Vec::new();
        self.claims_in_order(person, PRESENCE_KIND, true, |claim| {
            let recent = now.saturating_sub(claim.accepted_at_unix_ms) < window;
            if recent {
                claims.push(claim.clone());
            }
            recent
        })?;
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
            let (presence, observed) = fold_presence(&self.recent_presence(person, PRESENCE_STALE_MS, now)?, now);
            let derived = if presence == Presence::Present { Status::Available } else { Status::Away };
            Ok(PersonAvailability {
                person: person.to_owned(),
                status: status.as_ref().and_then(|claim| parse(claim.body["fields"].get("status"))).unwrap_or(derived),
                status_source: if status.is_some() { StatusSource::Override } else { StatusSource::Presence },
                status_until: status.as_ref().and_then(until).and_then(rfc3339),
                presence: Some(presence),
                presence_observed_at: observed.and_then(rfc3339),
                interruptions: interruptions.as_ref()
                    .and_then(|claim| parse(claim.body["fields"].get("interruptions")))
                    .unwrap_or(Interruptions::All),
                interruptions_until: interruptions.as_ref().and_then(until).and_then(rfc3339),
                deadline_clock: clock.as_ref()
                    .and_then(|claim| parse(claim.body["fields"].get("deadline_clock")))
                    .unwrap_or(DeadlineClock::Run),
                deadline_clock_since: clock.as_ref().and_then(|claim| rfc3339(claim.accepted_at_unix_ms)),
                deadline_clock_until: clock.as_ref().and_then(until).and_then(rfc3339),
            })
        }).map_err(internal)
    }

    fn set_person_availability(&self, person: &str, changes: &[AvailabilityChange], now: u128) -> Result<PersonAvailability, FractalError> {
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
            let mut fields = BTreeMap::from([(name.to_owned(), json!(value))]);
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

    fn report_person_presence(&self, person: &str, source: &str, presence: Presence, observed_at: u128, now: u128) -> Result<PersonAvailability, FractalError> {
        person_subject(person)?;
        if presence == Presence::Unknown {
            return Err(FractalError::new("invalid-presence", "report present, idle or locked"));
        }
        if observed_at > now + PRESENCE_FUTURE_SKEW_MS || now.saturating_sub(observed_at) >= PRESENCE_STALE_MS {
            return Err(FractalError::new("invalid-presence", "observed_at must be within the last 15 minutes and at most 2 minutes ahead of this member's clock"));
        }
        // Unchanged only when the recorded report is still fresh by the fold's own measure,
        // min(observed_at, accepted_at); an old-observed report must not swallow a fresh one.
        let unchanged = self.recent_presence(person, PRESENCE_REFRESH_MS, now).map_err(internal)?
            .into_iter()
            .find(|claim| claim.body["fields"]["source"] == source)
            .is_some_and(|claim| {
                let fields = &claim.body["fields"];
                let recorded = fields["observed_at_unix_ms"].as_u64().map_or(0, u128::from).min(claim.accepted_at_unix_ms);
                fields["presence"] == word(presence) && now.saturating_sub(recorded) < PRESENCE_REFRESH_MS
            });
        if !unchanged {
            self.append_claim(&ClaimInput {
                subject: person.into(),
                kind: PRESENCE_KIND.into(),
                actor: Some(person.into()),
                fields: BTreeMap::from([
                    ("presence".into(), json!(word(presence))),
                    ("source".into(), json!(source)),
                    ("observed_at_unix_ms".into(), json!(observed_at)),
                ]),
                evidence: Vec::new(), expected_subject: None, idempotency_key: None,
            })?;
        }
        self.person_availability(person, now)
    }

    fn deadline_clock_holds(&self, person: &str, since: u128) -> Result<Vec<ClockHold>> {
        let mut claims = Vec::new();
        self.claims_in_order(person, DEADLINE_CLOCK_KIND, true, |claim| {
            claims.push(claim.clone());
            claim.accepted_at_unix_ms > since
        })?;
        Ok(clock_holds(&claims, since))
    }
}

#[cfg(test)]
mod tests;
