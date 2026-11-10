//! The objectives a fleet declares (docs/st3/objectives.md): each `objective/NAME` is an interval
//! over claims, measured here from the replicated claims every member holds.
//!
//! Each objective keeps its samples in memory, one per subject under its prefix, with the store
//! index of the newest claim it has read; an evaluation reads only the claims after it, through
//! `claims_kind_index`. A new revision of the declaration starts again from the claims. A sample
//! that stopped or was cancelled before the window is dropped from memory.
//!
//! An open sample is censored, not dropped: for each target it enters the percentile at its age
//! once that age is over the target's `within`, since it is certain to be over.
//!
//! A missed row opens one `operational.failure` for the declaration's owner, on the member that
//! owner is declared on: every member evaluates the same claims, and one delivers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value, json};

use crate::model::{ClaimRecord, FractalError};
use crate::objective::{Breach, Indicator, Interval, Objective, Window, percentile_row};
use crate::objective_faults::{ObjectiveFaults, ObjectiveResponse, check_with_response};
use crate::objectives::{clamp_percent, duration, nearest_rank, plural};
use crate::store::Store;
use crate::store::declared_objectives::DeclaredObjective;
use smallclaims::error::internal;

/// Claims read per page while catching up.
const PAGE: usize = 1_000;
/// Open samples over target an explanation names; the rest are counted.
const LISTED_OPEN: usize = 5;

/// Every declared objective this store evaluates, by subject.
#[derive(Default)]
struct Evaluations(Mutex<BTreeMap<String, Tracked>>);

struct Tracked {
    revision: String,
    claim_id: String,
    objective: Result<Objective, String>,
    /// The store index of the newest claim read.
    cursor: u64,
    samples: BTreeMap<String, Sample>,
    /// Claims of the interval's kinds whose time, set or member could not be read.
    unreadable: u64,
    /// The rows the last evaluation produced, for the breach response.
    ids: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct Sample {
    /// The earliest start, and the label and exclude fields of its claim.
    start: Option<(u64, BTreeMap<String, String>)>,
    expect: Option<Expect>,
    /// Each member's stop times; without `member=`, under "".
    stops: BTreeMap<String, BTreeSet<u64>>,
    /// The earliest cancel.
    cancel: Option<u64>,
    /// Pauses (true) and resumes (false), in time order.
    pauses: Vec<(u64, bool)>,
    /// The latest time any of its claims carries.
    last_seen: u64,
}

/// The newest expect claim, by its time and then its claim id, so every member picks the same one.
#[derive(Debug)]
struct Expect {
    at: u64,
    id: String,
    set: Vec<String>,
    fields: BTreeMap<String, String>,
}

/// What one sample (or one member of it) is at `now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Closed { value: u64, stop: u64 },
    Open { age: u64 },
    Cancelled { at: u64 },
    Excluded { last_seen: u64 },
    /// Claims under the prefix with no start claim yet.
    Unstarted,
}

/// One point a row counts: a sample, or one member of it under `label "member"`.
struct Point<'a> {
    subject: &'a str,
    member: Option<&'a str>,
    class: Class,
}

/// The declared objectives' rows, evaluated at `now`.
pub fn rows(store: &Store, now: u64) -> anyhow::Result<Vec<Value>> {
    let declared = store.declared_objectives()?;
    let evaluations = store.fractal.extensions.get::<Evaluations>();
    let mut tracked = evaluations.0.lock().unwrap_or_else(PoisonError::into_inner);
    tracked.retain(|subject, _| declared.iter().any(|declaration| &declaration.subject == subject));
    let mut rows = Vec::new();
    for declaration in declared {
        let entry = tracked
            .entry(declaration.subject.clone())
            .or_insert_with(|| Tracked::new(&declaration));
        if entry.revision != declaration.revision {
            *entry = Tracked::new(&declaration);
        }
        let name = declaration.name();
        let interval = match &entry.objective {
            Ok(objective) => match (&objective.indicator, objective.window) {
                (Indicator::Interval(interval), Some(window)) if objective.declarable() => Some((interval.clone(), window)),
                _ => None,
            },
            Err(_) => None,
        };
        let Some((interval, window)) = interval else {
            let row = entry.unreadable_row(&declaration.subject, name);
            entry.ids = BTreeSet::from([name.to_owned()]);
            rows.push(row);
            continue;
        };
        entry.read(store, &interval)?;
        entry.prune(&interval, window, now);
        let response = response(store, entry.objective.as_ref().ok().and_then(|objective| objective.on_breach.as_ref()));
        let produced = entry.rows(&declaration.subject, &interval, window, now, &response);
        entry.ids = produced.iter().filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
        rows.extend(produced);
    }
    Ok(rows)
}

/// Respond to `rows`, the rows just evaluated, and report a failure once until a check succeeds
/// again.
pub fn check_and_report(store: &Store, rows: &[Value]) {
    static FAILING: AtomicBool = AtomicBool::new(false);
    match check(store, rows) {
        Ok(_) => FAILING.store(false, Ordering::Relaxed),
        Err(error) => {
            if !FAILING.swap(true, Ordering::Relaxed) {
                eprintln!("st3: responding to declared objectives failed: {error:?}");
            }
        }
    }
}

/// For each declared objective with `on-breach` whose owner is declared on this member, open a
/// fault for each of its missed rows and end it when the row is met again.
pub fn check(store: &Store, rows: &[Value]) -> Result<ObjectiveFaults, FractalError> {
    let responders: Vec<(String, Breach, BTreeSet<String>)> = {
        let evaluations = store.fractal.extensions.get::<Evaluations>();
        let tracked = evaluations.0.lock().unwrap_or_else(PoisonError::into_inner);
        tracked
            .values()
            .filter_map(|tracked| {
                let objective = tracked.objective.as_ref().ok()?;
                Some((objective.name.clone(), objective.on_breach.clone()?, tracked.ids.clone()))
            })
            .collect()
    };
    let mut total = ObjectiveFaults::default();
    for (name, breach, ids) in responders {
        if store.agent_host(&breach.owner).map_err(internal)?.as_deref() != Some(store.origin()) {
            continue;
        }
        let check = check_with_response(
            store,
            rows,
            &ids,
            Some(ObjectiveResponse { reviewer: &breach.owner, condition: &breach.condition, title: &name }),
        )?;
        total.opened += check.opened;
        total.recovered += check.recovered;
        total.unowned += check.unowned;
    }
    Ok(total)
}

impl Tracked {
    fn new(declaration: &DeclaredObjective) -> Self {
        Self {
            revision: declaration.revision.clone(),
            claim_id: declaration.claim_id.clone(),
            objective: declaration.objective.clone(),
            cursor: 0,
            samples: BTreeMap::new(),
            unreadable: 0,
            ids: BTreeSet::new(),
        }
    }

    /// Read the claims after the cursor.
    fn read(&mut self, store: &Store, interval: &Interval) -> anyhow::Result<()> {
        let kinds = interval.kinds();
        loop {
            let page = store.claims_by_kinds_after(&kinds, Some(&interval.subject_prefix), self.cursor, PAGE)?;
            for claim in &page.claims {
                self.cursor = self.cursor.max(claim.store_index);
                self.observe(interval, claim);
            }
            if page.next_cursor.is_none() {
                return Ok(());
            }
        }
    }

    fn observe(&mut self, interval: &Interval, claim: &ClaimRecord) {
        let sample = self.samples.entry(claim.subject.clone()).or_default();
        if sample.observe(interval, claim).is_none() {
            self.unreadable += 1;
        }
    }

    /// Drop the samples that stopped, were cancelled or were excluded before the window.
    fn prune(&mut self, interval: &Interval, window: Window, now: u64) {
        let horizon = now.saturating_sub(window.ms);
        self.samples.retain(|_, sample| match sample.class(interval, None, now) {
            Class::Closed { stop, .. } => stop >= horizon,
            Class::Cancelled { at } => at >= horizon,
            Class::Excluded { last_seen } => last_seen >= horizon,
            Class::Open { .. } | Class::Unstarted => true,
        });
    }

    fn short_revision(&self) -> &str {
        &self.revision[..self.revision.len().min(12)]
    }

    /// The row of a declaration this build cannot evaluate.
    fn unreadable_row(&self, subject: &str, name: &str) -> Value {
        let why = match &self.objective {
            Err(error) => format!("does not decode on this build: {error}"),
            Ok(objective) => format!(
                "is a `{}` indicator named `{}`, and a fleet declares only an interval named NAMESPACE/NAME",
                objective.indicator.kind(),
                objective.name
            ),
        };
        json!({
            "id": name, "kind": "objective", "about": format!("{subject} cannot be evaluated"),
            "target": "unreadable", "current": "unreadable declaration", "window": "now",
            "remaining_percent": null, "status": "no-data",
            "explain": format!(
                "{subject} at revision {} (claim {}) {why}. Nothing is measured until it is published again.",
                self.short_revision(), self.claim_id,
            ),
        })
    }

    fn rows(&self, subject: &str, interval: &Interval, window: Window, now: u64, response: &str) -> Vec<Value> {
        let Ok(objective) = &self.objective else {
            return Vec::new();
        };
        let points: Vec<Point<'_>> = self
            .samples
            .iter()
            .map(|(subject, sample)| Point { subject, member: None, class: sample.class(interval, None, now) })
            .collect();
        let unstarted = points.iter().filter(|point| point.class == Class::Unstarted).count() as u64;
        // Each label's rows, by value.
        let mut groups: Vec<(String, Vec<Point<'_>>)> = Vec::new();
        for label in &interval.labels {
            let mut by_value: BTreeMap<String, Vec<Point<'_>>> = BTreeMap::new();
            for (sample_subject, sample) in &self.samples {
                if label == "member" {
                    for member in sample.members() {
                        by_value.entry(member.to_owned()).or_default().push(Point {
                            subject: sample_subject,
                            member: Some(member),
                            class: sample.class(interval, Some(member), now),
                        });
                    }
                } else if let Some(value) = sample.label(label) {
                    by_value.entry(value.to_owned()).or_default().push(Point {
                        subject: sample_subject,
                        member: None,
                        class: sample.class(interval, None, now),
                    });
                }
            }
            groups.extend(by_value.into_iter().map(|(value, points)| (format!("{{{label}={value}}}"), points)));
        }
        let context = Context {
            objective,
            subject,
            revision: self.short_revision(),
            window,
            now,
            unreadable: self.unreadable,
            unstarted,
            response,
        };
        let mut rows = Vec::new();
        for target in &objective.targets {
            let (Some(p), Some(within)) = (target.percentile(), target.within_ms()) else {
                continue;
            };
            let id = percentile_row(&objective.name, p);
            rows.push(context.row(&id, p, within, &points));
            for (suffix, points) in &groups {
                rows.push(context.row(&format!("{id}{suffix}"), p, within, points));
            }
        }
        rows
    }
}

impl Sample {
    /// Fold one claim in; None when its time, set or member cannot be read.
    fn observe(&mut self, interval: &Interval, claim: &ClaimRecord) -> Option<()> {
        let fields = claim.body.get("fields").and_then(Value::as_object);
        let accepted = u64::try_from(claim.accepted_at_unix_ms).unwrap_or(u64::MAX);
        let time = |at: &Option<String>| match at {
            Some(field) => fields.and_then(|fields| fields.get(field)).and_then(time_ms),
            None => Some(accepted),
        };
        let kind = claim.kind.as_str();
        let at = if kind == interval.start.kind {
            let at = time(&interval.start.at)?;
            if self.start.as_ref().is_none_or(|(start, _)| at < *start) {
                self.start = Some((at, kept_fields(interval, fields)));
            }
            at
        } else if let Some((event, set)) = interval.expect.as_ref().filter(|(event, _)| event.kind == kind) {
            let at = time(&event.at)?;
            let set = fields.and_then(|fields| fields.get(set)).and_then(string_list)?;
            if self.expect.as_ref().is_none_or(|expect| (at, claim.id.as_str()) > (expect.at, expect.id.as_str())) {
                self.expect = Some(Expect { at, id: claim.id.clone(), set, fields: kept_fields(interval, fields) });
            }
            at
        } else if kind == interval.stop.0.kind {
            let at = time(&interval.stop.0.at)?;
            let member = match &interval.stop.1 {
                Some(field) => fields.and_then(|fields| fields.get(field)).and_then(text)?,
                None => String::new(),
            };
            self.stops.entry(member).or_default().insert(at);
            at
        } else if interval.cancel.as_deref() == Some(kind) {
            self.cancel = Some(self.cancel.map_or(accepted, |cancel| cancel.min(accepted)));
            accepted
        } else if let Some((pause, _)) = interval.pause.as_ref().filter(|(pause, resume)| pause == kind || resume == kind) {
            let event = (accepted, pause == kind);
            let at = self.pauses.partition_point(|existing| *existing <= event);
            self.pauses.insert(at, event);
            accepted
        } else {
            return None;
        };
        self.last_seen = self.last_seen.max(at);
        Some(())
    }

    /// The members a sample stops on: its expected set, else the members that stopped.
    fn members(&self) -> Vec<&str> {
        match &self.expect {
            Some(expect) => expect.set.iter().map(String::as_str).collect(),
            None => self.stops.keys().map(String::as_str).filter(|member| !member.is_empty()).collect(),
        }
    }

    fn label(&self, field: &str) -> Option<&str> {
        let start = self.start.as_ref().and_then(|(_, fields)| fields.get(field));
        start.or_else(|| self.expect.as_ref().and_then(|expect| expect.fields.get(field))).map(String::as_str)
    }

    fn excluded(&self, interval: &Interval) -> bool {
        interval.excludes.iter().any(|exclude| {
            let start = self.start.as_ref().and_then(|(_, fields)| fields.get(&exclude.field));
            let expect = self.expect.as_ref().and_then(|expect| expect.fields.get(&exclude.field));
            [start, expect].into_iter().flatten().any(|value| exclude.values.contains(value))
        })
    }

    /// The earliest stop of `member` at or after `start`.
    fn stop_of(&self, member: &str, start: u64) -> Option<u64> {
        self.stops.get(member)?.range(start..).next().copied()
    }

    /// The sample at `now`, or with `member`, that member's part of it.
    fn class(&self, interval: &Interval, member: Option<&str>, now: u64) -> Class {
        let Some((start, _)) = &self.start else {
            return Class::Unstarted;
        };
        let start = *start;
        if self.excluded(interval) {
            return Class::Excluded { last_seen: self.last_seen };
        }
        let close = match (member, &interval.expect, &self.expect) {
            (Some(member), ..) => self.stop_of(member, start),
            (None, Some(_), None) => None,
            (None, Some(_), Some(expect)) if expect.set.is_empty() => Some(expect.at.max(start)),
            (None, Some(_), Some(expect)) => expect
                .set
                .iter()
                .map(|member| self.stop_of(member, start))
                .collect::<Option<Vec<_>>>()
                .and_then(|stops| stops.into_iter().max()),
            (None, None, _) => self.stops.keys().filter_map(|member| self.stop_of(member, start)).min(),
        };
        if let Some(cancel) = self.cancel
            && close.is_none_or(|close| cancel < close)
        {
            return Class::Cancelled { at: cancel };
        }
        match close {
            Some(stop) => Class::Closed { value: (stop - start).saturating_sub(self.paused(start, stop)), stop },
            None => Class::Open { age: now.saturating_sub(start).saturating_sub(self.paused(start, now.max(start))) },
        }
    }

    /// The time paused between `start` and `end`; a pause with no resume runs to `end`.
    fn paused(&self, start: u64, end: u64) -> u64 {
        let mut total = 0;
        let mut since = None;
        for &(at, pause) in &self.pauses {
            let at = at.clamp(start, end);
            match (pause, since) {
                (true, None) => since = Some(at),
                (false, Some(from)) => {
                    total += at - from;
                    since = None;
                }
                _ => {}
            }
        }
        total + since.map_or(0, |from| end - from)
    }
}

/// The start or expect fields a sample keeps: its labels' and its excludes'.
fn kept_fields(interval: &Interval, fields: Option<&Map<String, Value>>) -> BTreeMap<String, String> {
    let Some(fields) = fields else {
        return BTreeMap::new();
    };
    interval
        .labels
        .iter()
        .chain(interval.excludes.iter().map(|exclude| &exclude.field))
        .filter_map(|name| Some((name.clone(), text(fields.get(name)?)?)))
        .collect()
}

/// A time from an RFC 3339 string or Unix milliseconds, either possibly JSON-encoded in a string.
fn time_ms(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
            .or_else(|| text.parse().ok())
            .or_else(|| serde_json::from_str::<Value>(text).ok().filter(Value::is_string).and_then(|value| time_ms(&value))),
        _ => None,
    }
}

/// A field's text: a string (a JSON-encoded one decoded), a number or a boolean.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(serde_json::from_str::<String>(text).unwrap_or_else(|_| text.clone())),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// A list of strings, or a string holding one as JSON.
fn string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => items.iter().map(|item| item.as_str().map(str::to_owned)).collect(),
        Value::String(text) => serde_json::from_str::<Value>(text).ok().filter(Value::is_array).and_then(|value| string_list(&value)),
        _ => None,
    }
}

/// A span as its largest whole unit: `7d`, `90m`.
fn span(ms: u64) -> String {
    for (unit, size) in [("d", 86_400_000), ("h", 3_600_000), ("m", 60_000), ("s", 1_000)] {
        if ms >= size && ms.is_multiple_of(size) {
            return format!("{}{unit}", ms / size);
        }
    }
    format!("{ms}ms")
}

/// What a miss does, as the explanation says it.
fn response(store: &Store, breach: Option<&Breach>) -> String {
    let Some(breach) = breach else {
        return "No response is declared: a miss is shown and nothing is delivered.".to_owned();
    };
    let backup = breach
        .backup
        .as_ref()
        .map(|backup| format!(" {backup} is the held backup: recorded and shown, never the reviewer."))
        .unwrap_or_default();
    let owner = &breach.owner;
    let live = store.fault_reviewer_is_declared(owner);
    let host = store.agent_host(owner);
    match (live, host) {
        (Ok(true), Ok(Some(host))) => format!(
            "A missed row opens one operational.failure `{}` on daemon/{host} addressed to {owner}, delivered by {host}, the member {owner} is declared on{}.{backup}",
            breach.condition,
            if host == store.origin() { " (this member)" } else { "" },
        ),
        (Ok(_), Ok(_)) => format!(
            "A miss would open an operational.failure `{}` for {owner}, but {owner} is not a live agent declaration, so nothing is delivered.{backup}",
            breach.condition,
        ),
        (Err(error), _) | (_, Err(error)) => format!("The response to {owner} could not be read: {error}.{backup}"),
    }
}

struct Context<'a> {
    objective: &'a Objective,
    subject: &'a str,
    revision: &'a str,
    window: Window,
    now: u64,
    unreadable: u64,
    unstarted: u64,
    response: &'a str,
}

impl Context<'_> {
    /// The row of percentile `p` within `within` over `points`.
    fn row(&self, id: &str, p: u64, within: u64, points: &[Point<'_>]) -> Value {
        let horizon = self.now.saturating_sub(self.window.ms);
        let (mut closed, mut open_within, mut cancelled, mut excluded) = (0u64, 0u64, 0u64, 0u64);
        let mut population = Vec::new();
        let mut open_over: Vec<(&Point<'_>, u64)> = Vec::new();
        for point in points {
            match point.class {
                Class::Closed { value, stop } if stop >= horizon => {
                    closed += 1;
                    population.push(value);
                }
                Class::Open { age } if age > within => {
                    open_over.push((point, age));
                    population.push(age);
                }
                Class::Open { .. } => open_within += 1,
                Class::Cancelled { at } if at >= horizon => cancelled += 1,
                Class::Excluded { last_seen } if last_seen >= horizon => excluded += 1,
                _ => {}
            }
        }
        open_over.sort_by_key(|point| std::cmp::Reverse(point.1));
        let window = span(self.window.ms);
        let goal_within = duration(within);
        let target = format!("p{p} ≤ {goal_within}");
        let about = &self.objective.about;
        let goal = format!("Goal: {p} of every 100 stop within {goal_within}, so {} in 100 may run over.", 100 - p);
        let mut counted = format!(
            "In the last {window}: {closed} closed, {} open over {goal_within}, {open_within} open within it, {cancelled} cancelled, {excluded} excluded, {} unreadable {}.",
            open_over.len(),
            self.unreadable,
            if self.unreadable == 1 { "claim" } else { "claims" },
        );
        if self.unstarted > 0 {
            counted.push_str(&format!(" {} {} claims but no start yet.", plural(self.unstarted, "subject"), if self.unstarted == 1 { "has" } else { "have" }));
        }
        let declared = format!("Declared as {} at revision {}.", self.subject, self.revision);
        let listed = if open_over.is_empty() {
            String::new()
        } else {
            let names = open_over
                .iter()
                .take(LISTED_OPEN)
                .map(|(point, age)| match point.member {
                    Some(member) => format!("{} on {member} (open {})", point.subject, duration(*age)),
                    None => format!("{} (open {})", point.subject, duration(*age)),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let more = open_over.len().saturating_sub(LISTED_OPEN);
            format!(
                " Open over target, counted at their age: {names}{}.",
                if more > 0 { format!(", and {more} more") } else { String::new() }
            )
        };
        let n = population.len() as u64;
        let base = json!({
            "id": id, "kind": "objective", "about": about, "target": target, "window": window,
        });
        let mut row = base;
        let Some(value) = (n > 0).then(|| nearest_rank(&population, p)) else {
            row["current"] = json!("no samples");
            row["remaining_percent"] = Value::Null;
            row["status"] = json!("no-data");
            row["explain"] = json!(format!(
                "{about}. {goal} {declared} {counted} No sample stopped in the window and none is open over target, so there is no budget to spend or save. {}",
                self.response,
            ));
            return row;
        };
        let over = population.iter().filter(|value| **value > within).count() as u64;
        let share = over as f64 / n as f64;
        let allowed = (100 - p) as f64 / 100.0;
        let remaining = clamp_percent(100.0 * (1.0 - share / allowed));
        let value = duration(value);
        let open = if open_over.is_empty() { String::new() } else { format!(" ({} open over)", open_over.len()) };
        let enough = n >= self.window.min_samples;
        row["current"] = json!(if enough {
            format!("p{p} {value} over {}{open}", plural(n, "sample"))
        } else {
            format!("p{p} {value} over {}{open}, needs {}", plural(n, "sample"), self.window.min_samples)
        });
        row["remaining_percent"] = if enough { json!(remaining) } else { Value::Null };
        let (status, verdict) = match (enough, share <= allowed) {
            (false, _) => ("no-data", format!("It needs {} before it is met or missed.", plural(self.window.min_samples, "sample"))),
            (true, true) => ("met", "The goal is met.".to_owned()),
            (true, false) => ("missed", format!("The goal is missed in the last {window}.")),
        };
        row["status"] = json!(status);
        row["explain"] = json!(format!(
            "{about}. {goal} {declared} {counted} An open sample over {goal_within} counts at its age, since it is certain to be over; a younger one is left out until it stops. Over {goal_within}: {over} of {} ({:.1}%), against an allowance of {}%, so {remaining:.0}% of the error budget is left. The p{p} is {value}. {verdict}{listed} {}",
            plural(n, "sample"),
            share * 100.0,
            100 - p,
            self.response,
        ));
        row
    }
}

#[cfg(test)]
#[path = "declared_objectives_tests.rs"]
mod tests;
