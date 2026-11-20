//! The objectives a fleet declares (docs/st3/objectives.md): each `objective/NAME` is an interval
//! or a count over claims, measured here from the replicated claims every member holds.
//!
//! Each objective keeps its samples (or events) in memory, one per subject under its prefix, with
//! the store index of the newest claim it has read; an evaluation reads only the claims after it,
//! through `claims_kind_index`. A new revision of the declaration starts again from the claims. A
//! sample that stopped or was cancelled before the window is dropped from memory, as is an event
//! before the window: a count's event time is a field its retries repeat, so a retry is either
//! deduplicated inside the window or ignored before it.
//!
//! A sample or event an `exclude` names, or one an `only` leaves out, is counted apart in its
//! row's explanation, so nothing the claims hold disappears without a trace.
//!
//! An open sample is censored, not dropped: for each target it enters the percentile at its age
//! once that age is over the target's `within`, since it is certain to be over.
//!
//! A missed row opens one `operational.failure` for the declaration's owner, on the member that
//! owner is declared on: every member evaluates the same claims, and one delivers. The fault of a
//! row the objective no longer judges, as a label value whose samples left the window or a
//! count's label row, which is never met or missed, ends there too.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value, json};

use crate::model::{ClaimRecord, FractalError};
use crate::objective::{Bound, Breach, Count, Exclude, Indicator, Interval, Objective, Only, Window, percentile_row};
use crate::objective_faults::{ObjectiveFaults, ObjectiveResponse, check_with_response, end_unjudged};
use crate::objectives::{clamp_percent, duration, nearest_rank, plural};
use crate::store::Store;
use crate::store::declared_objectives::DeclaredObjective;
use smallclaims::error::internal;
use crate::store::objective_series::time_ms;

#[path = "declared_objectives_budget.rs"]
mod budget;
pub(crate) use budget::{BudgetEvidence, classify_budget};

#[path = "declared_github_objectives.rs"]
mod github;

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
    /// A count's events in the window, by subject.
    events: BTreeMap<String, Counted>,
    numeric: budget::Observations,
    /// Whether a count has read an event claim for this revision.
    seen: bool,
    /// Each label's values on the event claims a count has read for this revision: a value's row
    /// stays, reading 0, once its events leave the window. At most the distinct values the
    /// claims under the prefix carry.
    labels: BTreeMap<String, BTreeSet<String>>,
    /// Claims of the indicator's kinds whose time, set or member could not be read.
    unreadable: u64,
    /// The rows the last evaluation judges, for the breach response: every row but a count's label
    /// rows, which are never met or missed.
    ids: BTreeSet<String>,
}

/// What a declared objective measures.
enum Measure {
    Interval(Box<Interval>),
    Count(Box<Count>),
    Ratio(Box<crate::objective::Ratio>),
    Gauge(Box<crate::objective::Gauge>),
}

/// A subject's event: its earliest event claim's time and id (the id breaks a tie, so every member
/// keeps the same claim), and the label, exclude and `only` fields of it.
#[derive(Debug)]
struct Counted {
    at: u64,
    id: String,
    fields: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct Sample {
    /// The earliest start by its time, then its claim id, so every member keeps the same claim;
    /// and the label, exclude and `only` fields of that claim.
    start: Option<(u64, String, BTreeMap<String, String>)>,
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
    /// Named by an `exclude`, or left out by an `only`: counted apart, never in a percentile.
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
/// Bucket explicitly selected claims, or the events/closed samples of one declared objective.
#[allow(clippy::too_many_arguments)]
pub fn claim_series(
    store: &Store,
    objective: Option<&str>,
    kinds: &[String],
    subject_prefix: Option<&str>,
    group_by: Option<&str>,
    from: u64,
    to: u64,
    bucket_seconds: u64,
) -> Result<Value, FractalError> {
    let (width, bucket_count) = series_bounds(from, to, bucket_seconds)?;
    if let Some(id) = objective {
        if !kinds.is_empty() || subject_prefix.is_some() || group_by.is_some() {
            return Err(FractalError::new("invalid-series", "objective cannot be combined with kind, subject_prefix or group_by"));
        }
        return objective_series(store, id, from, to, bucket_seconds, width, bucket_count);
    }
    if kinds.is_empty() || kinds.iter().any(String::is_empty) || group_by == Some("") {
        return Err(FractalError::new("invalid-series", "require at least one nonempty kind, and a nonempty group_by when supplied"));
    }
    let claims = store.series_claims(kinds, subject_prefix, from, to).map_err(internal)?;
    let mut counts = vec![0u64; bucket_count];
    let mut groups = group_by.map(|_| vec![BTreeMap::<String, (Value, u64)>::new(); bucket_count]);
    for claim in claims.into_iter().filter(crate::api::records::generic_claim_visible) {
        let fields = claim.body.get("fields").and_then(Value::as_object);
        let at = ["at", "occurred_at_unix_ms", "occurred_at"].into_iter()
            .find_map(|field| fields.and_then(|fields| fields.get(field)))
            .map_or_else(|| u64::try_from(claim.accepted_at_unix_ms).ok(), time_ms);
        let Some(at) = at.filter(|at| *at >= from && *at < to) else {
            continue;
        };
        let index = ((at - from) / width) as usize;
        counts[index] += 1;
        if let (Some(field), Some(groups)) = (group_by, &mut groups) {
            let value = fields.and_then(|fields| fields.get(field))
                .map_or(Value::Null, smallclaims::hash::canonical_json_value);
            let key = serde_json::to_string(&value).map_err(internal)?;
            groups[index].entry(key).or_insert((value, 0)).1 += 1;
        }
    }
    let buckets: Vec<Value> = counts.into_iter().enumerate().map(|(index, count)| {
        let start = from + index as u64 * width;
        let mut bucket = json!({"start": start, "end": start.saturating_add(width).min(to), "count": count});
        if let Some(groups) = &mut groups {
            bucket["groups"] = Value::Array(std::mem::take(&mut groups[index]).into_values()
                .map(|(value, count)| json!({"value": value, "count": count})).collect());
        }
        bucket
    }).collect();
    let mut response = json!({"from": from, "to": to, "bucket_seconds": bucket_seconds, "buckets": buckets});
    if let Some(field) = group_by {
        response["group_by"] = json!(field);
    }
    Ok(response)
}

fn series_bounds(from: u64, to: u64, bucket_seconds: u64) -> Result<(u64, usize), FractalError> {
    let invalid = || FractalError::new("invalid-series", "require from < to, at most 31 days, positive bucket_seconds, and at most 1000 buckets");
    let width = bucket_seconds.saturating_mul(1_000);
    if width == 0 {
        return Err(invalid());
    }
    let span = to.checked_sub(from).filter(|span| *span > 0 && *span <= 31 * 86_400_000).ok_or_else(invalid)?;
    let bucket_count = span.div_ceil(width);
    if bucket_count > 1_000 {
        return Err(invalid());
    }
    Ok((width, bucket_count as usize))
}

fn objective_series(store: &Store, id: &str, from: u64, to: u64, bucket_seconds: u64, width: u64, bucket_count: usize) -> Result<Value, FractalError> {
    let declaration = store.declared_objectives().map_err(internal)?.into_iter().find(|declaration| declaration.name() == id);
    let unsupported = || FractalError::new("unsupported-objective-series", format!("`{id}` is not a declared count or interval objective"));
    let declaration = match declaration {
        Some(declaration) => declaration,
        None if crate::slo::shipped().iter().any(|objective| objective.name == id) => return Err(unsupported()),
        None => return Err(FractalError::new("unknown-objective", format!("unknown objective `{id}`"))),
    };
    let objective = declaration.objective.as_ref().map_err(|_| unsupported())?;
    if !objective.declarable() {
        return Err(unsupported());
    }
    let mut tracked = Tracked::new(&declaration);
    let mut counts = vec![0u64; bucket_count];
    let mut tally = |at: u64| {
        if at >= from && at < to {
            counts[((at - from) / width) as usize] += 1;
        }
    };
    match &objective.indicator {
        Indicator::Count(count) => {
            let claims = store.objective_series_claims(&[(&count.event, Some(&count.at))], &[&count.event], &count.subject_prefix, from, to).map_err(internal)?;
            for claim in claims.iter().filter(|claim| crate::api::records::generic_claim_visible(claim)) {
                tracked.observe_count(count, claim, 0);
            }
            for event in tracked.events.values().filter(|event| !left_out(count, &event.fields)) {
                tally(event.at);
            }
        }
        Indicator::Interval(interval) => {
            // A completion is a stop, an empty expected set, or (when expect precedes start)
            // the start itself. Each seek is bounded; all context for candidates is retained.
            let mut events = vec![(interval.stop.0.kind.as_str(), interval.stop.0.at.as_deref()), (interval.start.kind.as_str(), interval.start.at.as_deref())];
            if let Some((expect, _)) = &interval.expect {
                events.push((expect.kind.as_str(), expect.at.as_deref()));
            }
            let claims = store.objective_series_claims(&events, &interval.kinds(), &interval.subject_prefix, from, to).map_err(internal)?;
            for claim in claims.iter().filter(|claim| crate::api::records::generic_claim_visible(claim)) {
                tracked.observe(interval, claim);
            }
            for sample in tracked.samples.values() {
                if let Class::Closed { stop, .. } = sample.class(interval, None, to) {
                    tally(stop);
                }
            }
        }
        _ => return Err(unsupported()),
    }
    let buckets: Vec<Value> = counts.into_iter().enumerate().map(|(index, count)| {
        let start = from + index as u64 * width;
        json!({"start": start, "end": start.saturating_add(width).min(to), "count": count})
    }).collect();
    Ok(json!({"from": from, "to": to, "bucket_seconds": bucket_seconds, "buckets": buckets}))
}


/// The declared objectives' rows, evaluated at `now`.
pub fn rows(store: &Store, now: u64, native_rows: &[Value]) -> anyhow::Result<Vec<Value>> {
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
        if entry.objective.as_ref().is_ok_and(|objective| objective.declarable() &&
            matches!(objective.indicator, Indicator::GithubMergedCount(_) | Indicator::GithubReadyToMerged(_))) {
            continue;
        }
        if let Ok(objective) = &entry.objective
            && let Indicator::Native { objective: source } = &objective.indicator
        {
            let response = objective.on_breach.as_ref().map_or_else(
                || response(store, None),
                |breach| format!(
                    "A burning or exhausted native row opens one member-local operational.failure `{}` on daemon/{} addressed to {}; delivery requires that owner to remain a live declared reviewer, regardless of its host.",
                    breach.condition, store.origin(), breach.owner,
                ),
            );
            let mut produced = Vec::new();
            for source_row in native_rows {
                if source_row["kind"] != "objective" { continue; }
                let Some(id) = source_row["id"].as_str() else { continue };
                if !native_owns(source, id) { continue; }
                let mut row = source_row.clone();
                row["id"] = Value::String(format!("{}{}", objective.name, &id[source.len()..]));
                row["about"] = Value::String(objective.about.clone());
                row["explain"] = Value::String(format!(
                    "{} Bound as {} at revision {} to native objective {source}. {response}",
                    source_row["explain"].as_str().unwrap_or(""), declaration.subject, entry.short_revision(),
                ));
                produced.push(row);
            }
            if produced.is_empty() {
                // A wholly missing source is unknown, not proof its previously burning children ended.
                let mut ids = entry.ids.clone();
                ids.extend(crate::objective_faults::owned_rows(store, &objective.name)?);
                if ids.is_empty() { ids.insert(objective.name.clone()); }
                for id in ids {
                    produced.push(json!({
                        "id": id, "kind": "objective", "about": objective.about,
                        "target": "native objective", "current": "native source unavailable", "window": "now",
                        "remaining_percent": null, "status": "no-data",
                        "explain": format!("Native objective {source} has no source row on this member. {response}"),
                    }));
                }
            }
            entry.ids = produced.iter().filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
            rows.extend(produced);
            continue;
        }
        let measure = match &entry.objective {
            Ok(objective) if objective.declarable() => match (&objective.indicator, objective.window) {
                (Indicator::Interval(interval), Some(window)) => Some((Measure::Interval(interval.clone()), window)),
                (Indicator::Count(count), Some(window)) => Some((Measure::Count(count.clone()), window)),
                (Indicator::Ratio(ratio), _) => objective.budget.as_ref().map(|budget| (
                    Measure::Ratio(ratio.clone()),
                    Window { ms: budget.horizon_ms, min_samples: budget.min_samples },
                )),
                (Indicator::Gauge(gauge), _) => objective.budget.as_ref().map(|budget| (
                    Measure::Gauge(gauge.clone()),
                    Window { ms: budget.horizon_ms, min_samples: budget.min_samples },
                )),
                _ => None,
            },
            _ => None,
        };
        let Some((measure, window)) = measure else {
            let row = entry.unreadable_row(&declaration.subject, name);
            entry.ids = BTreeSet::from([name.to_owned()]);
            rows.push(row);
            continue;
        };
        let response = response(store, entry.objective.as_ref().ok().and_then(|objective| objective.on_breach.as_ref()));
        let (produced, judged): (Vec<Value>, BTreeSet<String>) = match &measure {
            Measure::Interval(interval) => {
                entry.read(store, &interval.kinds(), &interval.subject_prefix, |entry, claim| entry.observe(interval, claim))?;
                entry.prune(interval, window, now);
                let produced = entry.rows(&declaration.subject, interval, window, now, &response);
                let judged = produced.iter().filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
                (produced, judged)
            }
            Measure::Count(count) => {
                let horizon = now.saturating_sub(window.ms);
                entry.read(store, &[count.event.as_str()], &count.subject_prefix, |entry, claim| entry.observe_count(count, claim, horizon))?;
                entry.events.retain(|_, event| event.at >= horizon);
                let produced = entry.count_rows(&declaration.subject, count, window, &response);
                // Only the total is judged; the first row is it.
                let judged = produced.first().and_then(|row| row["id"].as_str()).map(str::to_owned).into_iter().collect();
                (produced, judged)
            }
            Measure::Ratio(ratio) => {
                entry.read(store, &[&ratio.event], &ratio.subject_prefix, |entry, claim| {
                    entry.observe_numeric(claim, &ratio.at, &ratio.field, &ratio.labels, &ratio.excludes, &ratio.only);
                })?;
                entry.numeric.prune(now.saturating_sub(window.ms), false);
                let produced = entry.budget_rows(&declaration.subject, None, now, &response);
                let judged = produced.iter().filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
                (produced, judged)
            }
            Measure::Gauge(gauge) => {
                entry.read(store, &[&gauge.event], &gauge.subject_prefix, |entry, claim| {
                    entry.observe_numeric(claim, &gauge.at, &gauge.field, &gauge.labels, &gauge.excludes, &gauge.only);
                })?;
                let policy = entry.objective.as_ref().expect("decoded objective").budget.as_ref().expect("gauge budget");
                let retention = window.ms
                    .max(policy.fast.long_ms)
                    .max(policy.fast.short_ms)
                    .max(policy.slow.long_ms)
                    .max(policy.slow.short_ms)
                    .max(gauge.cadence_ms.saturating_mul(2));
                // Keep every evaluated window and its immediate predecessor. Measurement
                // clips that predecessor's held value to max-age; older points cannot help.
                entry.numeric.prune(now.saturating_sub(retention), true);
                let produced = entry.budget_rows(&declaration.subject, Some(gauge), now, &response);
                let judged = produced.iter().filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
                (produced, judged)
            }
        };
        entry.ids = judged;
        rows.extend(produced);
    }
    Ok(rows)
}

/// Source-backed objectives use only the committed domain projection, never the generic
/// earliest-claim-wins interval/count cache. The state directory belongs to the caller.
pub fn github_rows(store: &Store, state_dir: &std::path::Path, now: u64) -> anyhow::Result<Vec<Value>> {
    let declared = store.declared_objectives()?;
    let evaluations = store.fractal.extensions.get::<Evaluations>();
    let mut tracked = evaluations.0.lock().unwrap_or_else(PoisonError::into_inner);
    let mut rows = Vec::new();
    for declaration in declared {
        let entry = tracked.entry(declaration.subject.clone()).or_insert_with(|| Tracked::new(&declaration));
        if entry.revision != declaration.revision { *entry = Tracked::new(&declaration); }
        let Ok(objective) = &entry.objective else { continue; };
        if !objective.declarable() || !matches!(objective.indicator, Indicator::GithubMergedCount(_) | Indicator::GithubReadyToMerged(_)) { continue; }
        let row = github::row(store, state_dir, objective, &declaration.revision, now);
        entry.ids = if objective.on_breach.is_some() { BTreeSet::from([objective.name.clone()]) } else { BTreeSet::new() };
        rows.push(row);
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
/// fault for each of its missed rows and end it when the row is met again, or once the objective
/// no longer judges the row.
pub fn check(store: &Store, rows: &[Value]) -> Result<ObjectiveFaults, FractalError> {
    let responders: Vec<(String, Breach, BTreeSet<String>, bool)> = {
        let evaluations = store.fractal.extensions.get::<Evaluations>();
        let tracked = evaluations.0.lock().unwrap_or_else(PoisonError::into_inner);
        tracked
            .values()
            .filter_map(|tracked| {
                let objective = tracked.objective.as_ref().ok()?;
                let native = matches!(objective.indicator, Indicator::Native { .. });
                Some((objective.name.clone(), objective.on_breach.clone()?, tracked.ids.clone(), native))
            })
            .collect()
    };
    let mut total = ObjectiveFaults::default();
    for (name, breach, ids, native) in responders {
        // Replicated claim evidence has one delivery host; native evidence belongs to this member.
        if !native && store.agent_host(&breach.owner).map_err(internal)?.as_deref() != Some(store.origin()) {
            continue;
        }
        let owned = crate::objective_faults::owned_rows(store, &name)?;
        let check = check_with_response(
            store,
            rows,
            &ids,
            ObjectiveResponse { reviewer: &breach.owner, condition: &breach.condition, title: &name },
        )?;
        total.opened += check.opened;
        total.recovered += check.recovered
            + end_unjudged(store, |id| owned.contains(id) || (!native && owns(&name, id)), &ids)?;
        total.unowned += check.unowned;
    }
    Ok(total)
}

/// Native objectives can expose independent child rows (transactions, remote paths, CPU modes).
fn native_owns(name: &str, id: &str) -> bool {
    id.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('{'))
}

/// Whether row `id` is one of objective `name`'s: `NAME` or `NAME/pNN`, either with a
/// `{label=value}`. A deeper objective's rows, `NAME/OTHER`, are not.
fn owns(name: &str, id: &str) -> bool {
    let Some(rest) = id.strip_prefix(name) else {
        return false;
    };
    let rest = match rest.strip_prefix("/p") {
        Some(percentile) => match percentile.trim_start_matches(|c: char| c.is_ascii_digit()) {
            rest if rest.len() == percentile.len() => return false,
            rest => rest,
        },
        None => rest,
    };
    rest.is_empty() || rest.starts_with('{')
}

impl Tracked {
    fn new(declaration: &DeclaredObjective) -> Self {
        Self {
            revision: declaration.revision.clone(),
            claim_id: declaration.claim_id.clone(),
            objective: declaration.objective.clone(),
            cursor: 0,
            samples: BTreeMap::new(),
            events: BTreeMap::new(),
            numeric: budget::Observations::default(),
            seen: false,
            labels: BTreeMap::new(),
            unreadable: 0,
            ids: BTreeSet::new(),
        }
    }

    /// Read the claims of `kinds` under `prefix` after the cursor.
    fn read(&mut self, store: &Store, kinds: &[&str], prefix: &str, mut observe: impl FnMut(&mut Self, &ClaimRecord)) -> anyhow::Result<()> {
        loop {
            let page = store.claims_by_kinds_after(kinds, Some(prefix), self.cursor, PAGE)?;
            for claim in &page.claims {
                self.cursor = self.cursor.max(claim.store_index);
                observe(self, claim);
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

    /// Fold one event claim in: a subject counts once, at its earliest event claim, and an event
    /// before `horizon` is not kept, though its label values are. A claim whose time cannot be
    /// read is unreadable and changes nothing.
    fn observe_count(&mut self, count: &Count, claim: &ClaimRecord, horizon: u64) {
        let fields = claim.body.get("fields").and_then(Value::as_object);
        let Some(at) = fields.and_then(|fields| fields.get(&count.at)).and_then(time_ms) else {
            self.unreadable += 1;
            return;
        };
        self.seen = true;
        let kept = kept_fields(&count.labels, &count.excludes, &count.only, fields);
        for label in &count.labels {
            let Some(value) = kept.get(label) else {
                continue;
            };
            if !self.labels.get(label).is_some_and(|values| values.contains(value)) {
                self.labels.entry(label.clone()).or_default().insert(value.clone());
            }
        }
        if at < horizon {
            return;
        }
        let event = Counted { at, id: claim.id.clone(), fields: kept };
        match self.events.get_mut(&claim.subject) {
            Some(earliest) if (at, claim.id.as_str()) < (earliest.at, earliest.id.as_str()) => *earliest = event,
            Some(_) => {}
            None => {
                self.events.insert(claim.subject.clone(), event);
            }
        }
    }

    /// Drop the samples that stopped, were cancelled or were excluded or left out before the window.
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
                "is a `{}` indicator named `{}`, and is not a supported fleet declaration named NAMESPACE/NAME",
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
        let starts = kind == interval.start.kind;
        let expects = interval.expect.as_ref().filter(|(event, _)| event.kind == kind);
        let stops = kind == interval.stop.0.kind;
        let at = if starts || expects.is_some() {
            // One kind may both start and set the expected members, or start and stop the sample;
            // read every role before changing the sample, so an unreadable claim changes nothing.
            let start = if starts { Some(time(&interval.start.at)?) } else { None };
            let expect = match expects {
                Some((event, set)) => Some((time(&event.at)?, fields.and_then(|fields| fields.get(set)).and_then(string_list)?)),
                None => None,
            };
            // A `stop` naming the `start` kind takes no `member=`.
            let stop = if stops { Some(time(&interval.stop.0.at)?) } else { None };
            let selected = start.is_some_and(|at| {
                self.start.as_ref().is_none_or(|(start, id, _)| (at, claim.id.as_str()) < (*start, id.as_str()))
            });
            if let Some(at) = start
                && selected
            {
                self.start = Some((at, claim.id.clone(), kept_fields(&interval.labels, &interval.excludes, &interval.only, fields)));
            }
            let latest = start.into_iter().chain(expect.as_ref().map(|(at, _)| *at)).chain(stop).max().unwrap_or_default();
            if let Some((at, set)) = expect
                && self.expect.as_ref().is_none_or(|expect| (at, claim.id.as_str()) > (expect.at, expect.id.as_str()))
            {
                self.expect = Some(Expect { at, id: claim.id.clone(), set, fields: kept_fields(&interval.labels, &interval.excludes, &interval.only, fields) });
            }
            if let Some(at) = stop {
                if starts {
                    // One claim is the whole sample: only the start claim's own stop closes it, so a
                    // sample never pairs one claim's start with another's stop.
                    if selected {
                        self.stops.insert(String::new(), BTreeSet::from([at]));
                    }
                } else {
                    self.stops.entry(String::new()).or_default().insert(at);
                }
            }
            latest
        } else if stops {
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
        let start = self.start.as_ref().and_then(|(_, _, fields)| fields.get(field));
        start.or_else(|| self.expect.as_ref().and_then(|expect| expect.fields.get(field))).map(String::as_str)
    }

    /// Whether an `exclude` names a value of its start or expect claim, or an `only` does not hold
    /// of its start claim.
    fn excluded(&self, interval: &Interval) -> bool {
        let excluded = interval.excludes.iter().any(|exclude| {
            let start = self.start.as_ref().and_then(|(_, _, fields)| fields.get(&exclude.field));
            let expect = self.expect.as_ref().and_then(|expect| expect.fields.get(&exclude.field));
            [start, expect].into_iter().flatten().any(|value| exclude.values.contains(value))
        });
        excluded || self.start.as_ref().is_some_and(|(_, _, fields)| !holds(&interval.only, fields))
    }

    /// The earliest stop of `member` at or after `start`.
    fn stop_of(&self, member: &str, start: u64) -> Option<u64> {
        self.stops.get(member)?.range(start..).next().copied()
    }

    /// The sample at `now`, or with `member`, that member's part of it.
    fn class(&self, interval: &Interval, member: Option<&str>, now: u64) -> Class {
        let Some((start, _, _)) = &self.start else {
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

/// The fields a sample's start or expect claim, or a count's event claim, keeps: its labels', its
/// excludes' and its `only`s'.
fn kept_fields(labels: &[String], excludes: &[Exclude], only: &[Only], fields: Option<&Map<String, Value>>) -> BTreeMap<String, String> {
    let Some(fields) = fields else {
        return BTreeMap::new();
    };
    labels
        .iter()
        .chain(excludes.iter().map(|exclude| &exclude.field))
        .chain(only.iter().map(|only| &only.field))
        .filter_map(|name| Some((name.clone(), text(fields.get(name)?)?)))
        .collect()
}

impl Tracked {
    /// A count's row, then one per value of each label the revision has read, each a part of the
    /// count shown without a verdict.
    fn count_rows(&self, subject: &str, count: &Count, window: Window, response: &str) -> Vec<Value> {
        let Ok(objective) = &self.objective else {
            return Vec::new();
        };
        let limit = match objective.target("per-hour", false).map(|target| target.bound) {
            Some(Bound::Min(min)) => Limit::Min(min),
            Some(Bound::Max(max)) => Limit::Max(max),
            _ => return Vec::new(),
        };
        let row = |id: &str, part: Option<(&str, &str)>, events: &[&Counted]| {
            self.count_row(objective, subject, count, window, limit, response, id, part, events)
        };
        let all = self.events.values().collect::<Vec<_>>();
        let mut rows = vec![row(&objective.name, None, &all)];
        for label in &count.labels {
            // Every value read, so one whose events left the window reads 0.
            let mut by_value: BTreeMap<&str, Vec<&Counted>> =
                self.labels.get(label).into_iter().flatten().map(|value| (value.as_str(), Vec::new())).collect();
            for event in &all {
                if let Some(value) = event.fields.get(label) {
                    by_value.entry(value).or_default().push(event);
                }
            }
            for (value, events) in by_value {
                rows.push(row(&format!("{}{{{label}={value}}}", objective.name), Some((label.as_str(), value)), &events));
            }
        }
        rows
    }

    /// Events in the window and their rate per hour against a `per-hour` maximum or minimum; for
    /// `part`, a label's value, the events and rate alone: the bound is the whole count's, so a
    /// part is never met or missed.
    #[allow(clippy::too_many_arguments)]
    fn count_row(
        &self,
        objective: &Objective,
        subject: &str,
        count: &Count,
        window: Window,
        limit: Limit,
        response: &str,
        id: &str,
        part: Option<(&str, &str)>,
        events: &[&Counted],
    ) -> Value {
        let excluded = events.iter().filter(|event| left_out(count, &event.fields)).count() as u64;
        let counted = events.len() as u64 - excluded;
        let span = span(window.ms);
        let rate = counted as f64 / (window.ms as f64 / 3_600_000.0);
        let about = &objective.about;
        let (target, goal) = match limit {
            Limit::Min(min) => (format!("≥ {min} an hour"), format!("Goal: at least {min} an hour over the last {span}.")),
            Limit::Max(max) => (
                format!("≤ {max} an hour"),
                format!("Goal: at most {max} an hour over the last {span}{}.", if max == 0.0 { ", so one spends the whole allowance" } else { "" }),
            ),
        };
        let declared = format!("Declared as {subject} at revision {}.", self.short_revision());
        let tallied = format!(
            "In the last {span}: {counted} counted, {excluded} excluded or left out, {} unreadable {}. Each subject under {} counts once, at its earliest {} claim.",
            self.unreadable,
            if self.unreadable == 1 { "claim" } else { "claims" },
            count.subject_prefix,
            count.event,
        );
        let mut row = json!({ "id": id, "kind": "objective", "about": about, "target": target, "window": span });
        if !self.seen {
            row["current"] = json!("no events yet");
            row["remaining_percent"] = Value::Null;
            row["status"] = json!("no-data");
            row["explain"] = json!(format!(
                "{about}. {goal} {declared} {tallied} No event has been read for this revision, so nothing is measured yet: once one is, an empty window counts as 0. {response}"
            ));
            return row;
        }
        row["current"] = json!(format!("{counted} in the last {span} ({rate:.2} an hour)"));
        if let Some((label, value)) = part {
            // A part has no goal of its own: the bound is the whole count's, so its row is never
            // met or missed, opens no fault and spends no budget.
            let name = &objective.name;
            row["target"] = json!(format!("part of {name}"));
            row["remaining_percent"] = Value::Null;
            row["status"] = json!("no-data");
            row["explain"] = json!(format!(
                "{about}, those whose {label} is {value}. {declared} {tallied} That is {rate:.2} an hour. {goal} It applies to {name} as a whole: this part of it is shown, never met or missed."
            ));
            return row;
        }
        let (met, remaining, budget) = match limit {
            Limit::Min(min) => {
                // The share of the rate above the minimum: 0% at the minimum, 100% as it grows.
                let remaining = if rate > 0.0 { clamp_percent(100.0 * (1.0 - min / rate)) } else { 0.0 };
                (rate >= min, remaining, format!("{remaining:.0}% of the rate is above the minimum"))
            }
            Limit::Max(max) => {
                let met = rate <= max;
                let remaining = if max == 0.0 { if met { 100.0 } else { 0.0 } } else { clamp_percent(100.0 * (1.0 - rate / max)) };
                (met, remaining, format!("{remaining:.0}% of the allowance is left"))
            }
        };
        row["remaining_percent"] = json!(remaining);
        row["status"] = json!(if met { "met" } else { "missed" });
        row["explain"] = json!(format!(
            "{about}. {goal} {declared} {tallied} That is {rate:.2} an hour, so {budget}. {} {response}",
            if met { "The goal is met.".to_owned() } else { format!("The goal is missed in the last {span}.") },
        ));
        row
    }
}

/// A count's `per-hour` target.
#[derive(Clone, Copy)]
enum Limit {
    Min(f64),
    Max(f64),
}

/// Whether an exclude names one of an event's `fields`' values, or an `only` does not hold of them.
fn left_out(count: &Count, fields: &BTreeMap<String, String>) -> bool {
    count.excludes.iter().any(|exclude| fields.get(&exclude.field).is_some_and(|value| exclude.values.contains(value)))
        || !holds(&count.only, fields)
}

/// Whether each `only` names the value of its field in `fields`; a missing field does not hold.
fn holds(only: &[Only], fields: &BTreeMap<String, String>) -> bool {
    only.iter().all(|only| fields.get(&only.field).is_some_and(|value| only.values.contains(value)))
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
            "In the last {window}: {closed} closed, {} open over {goal_within}, {open_within} open within it, {cancelled} cancelled, {excluded} excluded or left out, {} unreadable {}.",
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
