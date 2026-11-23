//! Pure measurements of the canonical committed projection. Never folds raw claims: a new
//! projection revision replaces this input, so corrections and retractions change old windows.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Days, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use fractal_schema::github_capture::{FactFamily, LifecycleFact, LifecycleKind, MergeFact, Projection, PullRequestFact, ReadyCycle, Source};
use serde::Serialize;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut { Now, Watermark }

#[derive(Debug, Serialize)]
pub struct Window {
    pub start_ms: u64,
    pub end_ms: u64,
    pub calendar_end_ms: Option<u64>,
    /// Calendar-incomplete today, independently of acquisition lag.
    pub partial_today: bool,
    pub complete: bool,
    pub observed_merges: u64,
    pub merged_count: Option<u64>,
    pub ready_to_merged_ms: Vec<u64>,
    pub unknown_cycles: u64,
    pub unknown: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Measurement {
    pub source: Source,
    pub projection_revision: String,
    pub roster_revision: String,
    pub frontier_ms: u64,
    pub source_lag_ms: u64,
    pub cut_ms: u64,
    pub cut: &'static str,
    pub timezone: String,
    pub rolling_hour: Window,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolling_window: Option<Window>,
    pub days: Vec<Window>,
}

/// Fourteen civil days, including today, the diagnostic hour and the authored rolling window.
/// Named IANA zones use their real offsets at each boundary.
pub fn measure(projection: &Projection, now_ms: u64, cut: Cut, timezone: &str, latency: bool, rolling_ms: u64, author_logins: &[String]) -> anyhow::Result<Measurement> {
    let timezone: Tz = timezone.parse()?;
    let cut_ms = match cut { Cut::Now => now_ms, Cut::Watermark => now_ms.min(projection.frontier_ms) };
    let instant = timestamp(cut_ms)?;
    let today = instant.with_timezone(&timezone).date_naive();
    let mut days = Vec::with_capacity(14);
    let index = Index::new(projection, author_logins);
    for ago in (0..14).rev() {
        let date = today.checked_sub_days(Days::new(ago)).ok_or_else(|| anyhow::anyhow!("calendar underflow"))?;
        let next = date.succ_opt().ok_or_else(|| anyhow::anyhow!("calendar overflow"))?;
        let start = midnight(timezone, date)?;
        let end = midnight(timezone, next)?;
        let mut bar = index.window(start, end.min(cut_ms), end > cut_ms, latency);
        bar.calendar_end_ms = Some(end);
        days.push(bar);
    }
    Ok(Measurement {
        source: projection.source.clone(), projection_revision: projection.revision.clone(),
        roster_revision: projection.roster.revision.clone(), frontier_ms: projection.frontier_ms,
        source_lag_ms: now_ms.saturating_sub(projection.frontier_ms), cut_ms,
        cut: match cut { Cut::Now => "now", Cut::Watermark => "watermark" },
        timezone: timezone.to_string(), rolling_hour: index.window(cut_ms.saturating_sub(3_600_000), cut_ms, false, latency), days,
        rolling_window: (rolling_ms != 3_600_000).then(|| index.window(cut_ms.saturating_sub(rolling_ms), cut_ms, false, latency)),
    })
}

fn timestamp(ms: u64) -> anyhow::Result<DateTime<Utc>> {
    let ms = i64::try_from(ms)?;
    DateTime::from_timestamp_millis(ms).ok_or_else(|| anyhow::anyhow!("timestamp outside calendar range"))
}

/// Earliest required bound for a query serving the rolling hour and fourteen civil bars.
pub fn history_start(cut_ms: u64, timezone: &str) -> anyhow::Result<u64> {
    let zone: Tz = timezone.parse()?;
    let today = timestamp(cut_ms)?.with_timezone(&zone).date_naive();
    let first = today.checked_sub_days(Days::new(13)).ok_or_else(|| anyhow::anyhow!("calendar underflow"))?;
    Ok(midnight(zone, first)?.min(cut_ms.saturating_sub(3_600_000)))
}

fn midnight(zone: Tz, date: NaiveDate) -> anyhow::Result<u64> {
    let local = date.and_hms_opt(0, 0, 0).ok_or_else(|| anyhow::anyhow!("invalid midnight"))?;
    let time = match zone.from_local_datetime(&local) {
        LocalResult::Single(time) => time,
        // Repeated midnight begins at the first occurrence.
        LocalResult::Ambiguous(first, second) => first.min(second),
        // A skipped midnight starts at the first representable instant of that civil date.
        LocalResult::None => {
            let mut candidate = local;
            loop {
                candidate = candidate.checked_add_signed(chrono::Duration::seconds(1)).ok_or_else(|| anyhow::anyhow!("calendar overflow"))?;
                if candidate.date() != date { anyhow::bail!("timezone skips calendar date {date}"); }
                if let Some(time) = zone.from_local_datetime(&candidate).earliest() { break time; }
            }
        }
    };
    Ok(u64::try_from(time.timestamp_millis())?)
}

#[derive(Debug, Serialize)]
pub struct DeadlineWindow {
    pub window_ms: u64,
    pub total: u64,
    pub bad: u64,
    pub complete: bool,
}

#[derive(Debug, Serialize)]
pub struct LatencyBudget {
    pub windows: Vec<DeadlineWindow>,
    pub merged_latency_ms: Vec<u64>,
    pub unknown: Vec<String>,
}

/// Each final-ready cycle settles once: at a timely merge, or at its deadline.
/// Late completion never moves a bad event out of a burn window. A close before
/// the deadline cancels an unmerged cycle; drafts and still-pending cycles do not vote.
pub fn latency_budget(projection: &Projection, cut_ms: u64, within_ms: u64, windows: &[u64], author_logins: &[String]) -> LatencyBudget {
    let index = Index::new(projection, author_logins);
    let mut events = Vec::new();
    let mut unknown = Vec::new();
    let history_start = cut_ms.saturating_sub(windows.iter().copied().max().unwrap_or(0));
    for (key, merge) in &index.merges {
        if merge.merged_at_ms >= history_start && merge.merged_at_ms < cut_ms
            && projection.roster.contains(&merge.repository_id, merge.merged_at_ms) && !index.prs.contains_key(key) {
            unknown.push(format!("{}/{}: missing pull request", key.0, key.1));
        }
    }
    for event in &projection.lifecycle {
        if event.kind == LifecycleKind::Ready
            && let Some(deadline) = event.occurred_at_ms.checked_add(within_ms)
            && deadline >= history_start && deadline < cut_ms
            && projection.roster.contains(&event.repository_id, deadline)
            && !index.prs.contains_key(&(event.repository_id.as_str(), event.pull_request_id.as_str())) {
            unknown.push(format!("{}/{}: missing pull request for ready cycle", event.repository_id, event.pull_request_id));
        }
    }
    for (key, pr) in &index.prs {
        if pr.created_at_ms >= cut_ms { continue; }
        if pr.merged_at_ms.or(pr.closed_at_ms).is_some_and(|at| at < history_start) { continue; }
        // Unknown evidence can affect this budget only if the PR could settle in
        // an effective membership interval. Retired/unrelated repositories do not vote.
        if !projection.roster.members.iter().any(|member| {
            member.repository_id == pr.repository_id
                && history_start.max(pr.created_at_ms).max(member.effective_from_ms)
                    < cut_ms.min(member.effective_until_ms.unwrap_or(cut_ms))
                        .min(pr.merged_at_ms.or(pr.closed_at_ms).map_or(cut_ms, |at| at.saturating_add(1)))
        }) { continue; }
        if !author_logins.is_empty() {
            match &pr.author_login {
                Some(author) if !author_logins.iter().any(|login| login.eq_ignore_ascii_case(author)) => continue,
                None => { unknown.push(format!("{}/{}: missing author login for selector", key.0, key.1)); continue; }
                _ => {}
            }
        }
        if index.conflicts.contains(key) {
            unknown.push(format!("{}/{}: conflicting canonical images", key.0, key.1));
            continue;
        }
        let merged = pr.merged_at_ms.filter(|at| *at < cut_ms);
        if merged.is_some() && index.merges.get(key).map(|merge| merge.merged_at_ms) != merged {
            unknown.push(format!("{}/{}: merge image disagrees with pull request", key.0, key.1));
            continue;
        }
        let at = merged.or(pr.closed_at_ms.filter(|at| *at < cut_ms)).unwrap_or(cut_ms.saturating_sub(1));
        let ready = match index.ready_state(key, pr, at) {
            ReadyCycle::Ready(ready) => ready,
            ReadyCycle::Draft if merged.is_none() => continue,
            ReadyCycle::Draft | ReadyCycle::Unknown => { unknown.push(format!("{}/{}: final ready cycle unproven", key.0, key.1)); continue; }
        };
        let Some(deadline) = ready.checked_add(within_ms) else {
            unknown.push(format!("{}/{}: deadline overflow", key.0, key.1));
            continue;
        };
        let settled = match merged {
            Some(merged) => (merged.min(deadline), merged > deadline),
            None if pr.closed_at_ms.is_some_and(|closed| closed < deadline && closed < cut_ms) => continue,
            None if deadline < cut_ms => (deadline, true),
            None => continue,
        };
        if projection.roster.contains(&pr.repository_id, settled.0) { events.push(settled); }
    }
    let mut merged_latency_ms = Vec::new();
    let windows = windows.iter().enumerate().map(|(position, &window_ms)| {
        let start = cut_ms.saturating_sub(window_ms);
        let diagnostic = index.window(start, cut_ms, false, true);
        let complete = diagnostic.complete && unknown.is_empty();
        if position == 0 { merged_latency_ms = diagnostic.ready_to_merged_ms; }
        let mut total = 0;
        let mut bad = 0;
        for &(at, failed) in &events {
            if at >= start && at < cut_ms { total += 1; bad += u64::from(failed); }
        }
        DeadlineWindow { window_ms, total, bad, complete }
    }).collect();
    LatencyBudget { windows, merged_latency_ms, unknown }
}

struct Index<'a> {
    projection: &'a Projection,
    author_logins: &'a [String],
    prs: BTreeMap<(&'a str, &'a str), &'a PullRequestFact>,
    events: BTreeMap<(&'a str, &'a str), Vec<&'a LifecycleFact>>,
    merges: BTreeMap<(&'a str, &'a str), &'a MergeFact>,
    ready_cycles: BTreeMap<(&'a str, &'a str), ReadyCycle>,
    conflicts: BTreeSet<(&'a str, &'a str)>,
}

impl<'a> Index<'a> {
    fn new(projection: &'a Projection, author_logins: &'a [String]) -> Self {
        let ready_cycles = projection.ready_cycles.iter()
            .map(|((repository, pr), ready)| ((repository.as_str(), pr.as_str()), *ready)).collect();
        let mut index = Self { projection, author_logins, prs: BTreeMap::new(), events: BTreeMap::new(), merges: BTreeMap::new(), ready_cycles, conflicts: BTreeSet::new() };
        for pr in &projection.pull_requests {
            let key = (pr.repository_id.as_str(), pr.id.as_str());
            if index.prs.insert(key, pr).is_some_and(|old| old != pr) { index.conflicts.insert(key); }
        }
        for event in &projection.lifecycle {
            index.events.entry((&event.repository_id, &event.pull_request_id)).or_default().push(event);
        }
        for events in index.events.values_mut() {
            events.sort_by(|left, right| (left.occurred_at_ms, &left.id).cmp(&(right.occurred_at_ms, &right.id)));
        }
        for merge in &projection.merges {
            let key = (merge.repository_id.as_str(), merge.pull_request_id.as_str());
            if index.merges.insert(key, merge).is_some_and(|old| old != merge) { index.conflicts.insert(key); }
        }
        index
    }

    fn covered(&self, repo: &str, family: FactFamily, start: u64, end: u64) -> bool {
        if start >= end { return true; }
        let mut intervals: Vec<_> = self.projection.coverage.iter().filter(|coverage| {
            coverage.repository_id == repo && coverage.family == family && coverage.complete && coverage.authoritative && coverage.error.is_none()
        }).collect();
        intervals.sort_by_key(|coverage| coverage.history_from_ms);
        let mut through = start;
        for coverage in intervals {
            if coverage.history_from_ms > through { break; }
            through = through.max(coverage.through_ms);
            if through >= end { return true; }
        }
        false
    }

    fn window(&self, start: u64, end: u64, partial_today: bool, latency: bool) -> Window {
        let mut result = Window { start_ms: start, end_ms: end, calendar_end_ms: None, partial_today, complete: true, observed_merges: 0,
            merged_count: None, ready_to_merged_ms: Vec::new(), unknown_cycles: 0, unknown: Vec::new() };
        if end > self.projection.frontier_ms { result.unknown.push("source frontier precedes requested cut".into()); }
        if self.projection.roster.members.iter().map(|member| member.effective_from_ms).min()
            .is_none_or(|earliest| start < earliest) {
            result.unknown.push("requested window precedes earliest roster evidence".into());
        }
        // Membership is intersected with each measurement window; historical dates are never invented.
        for member in &self.projection.roster.members {
            let from = start.max(member.effective_from_ms);
            let until = end.min(member.effective_until_ms.unwrap_or(end));
            if from >= until { continue; }
            if latency && !self.covered(&member.repository_id, FactFamily::Lifecycle, from, until) {
                result.unknown.push(format!("{}: missing Lifecycle coverage [{from},{until})", member.repository_id));
            }
            for family in [FactFamily::Merge, FactFamily::PullRequest] {
                if !self.covered(&member.repository_id, family, from, until) {
                    result.unknown.push(format!("{}: missing {family:?} coverage [{from},{until})", member.repository_id));
                }
            }
        }
        for (key, merge) in &self.merges {
            if merge.merged_at_ms < start || merge.merged_at_ms >= end || !self.projection.roster.contains(&merge.repository_id, merge.merged_at_ms) { continue; }
            result.observed_merges += 1;
            if self.conflicts.contains(key) {
                result.unknown.push(format!("{}/{}: conflicting canonical images", key.0, key.1));
                if latency { result.unknown_cycles += 1; }
                continue;
            }
            let Some(pr) = self.prs.get(key) else {
                result.unknown.push(format!("{}/{}: missing pull request", key.0, key.1));
                if latency { result.unknown_cycles += 1; }
                continue;
            };
            if !self.author_logins.is_empty() {
                let Some(author) = &pr.author_login else {
                    result.unknown.push(format!("{}/{}: missing author login for selector", key.0, key.1));
                    if latency { result.unknown_cycles += 1; }
                    continue;
                };
                if !self.author_logins.iter().any(|login| login.eq_ignore_ascii_case(author)) {
                    result.observed_merges -= 1;
                    continue;
                }
            }
            if pr.merged_at_ms != Some(merge.merged_at_ms) {
                result.unknown.push(format!("{}/{}: merge image disagrees with pull request", key.0, key.1));
                if latency { result.unknown_cycles += 1; }
                continue;
            }
            if latency {
                match self.last_ready(key, pr, merge.merged_at_ms) {
                    Some(ready) => result.ready_to_merged_ms.push(merge.merged_at_ms - ready),
                    None => { result.unknown_cycles += 1; result.unknown.push(format!("{}/{}: final ready cycle unproven", key.0, key.1)); }
                }
            }
        }
        result.complete = result.unknown.is_empty();
        if result.complete { result.merged_count = Some(result.observed_merges); }
        result.ready_to_merged_ms.sort_unstable();
        result
    }

    fn last_ready(&self, key: &(&str, &str), pr: &PullRequestFact, at: u64) -> Option<u64> {
        match self.ready_state(key, pr, at) { ReadyCycle::Ready(ready) => Some(ready), _ => None }
    }

    fn ready_state(&self, key: &(&str, &str), pr: &PullRequestFact, at: u64) -> ReadyCycle {
        if pr.created_at_ms > at || !self.covered(&pr.repository_id, FactFamily::Lifecycle, pr.created_at_ms, at) { return ReadyCycle::Unknown; }
        if let Some(state) = self.ready_cycles.get(key) {
            return match *state {
                ReadyCycle::Ready(ready) if ready < pr.created_at_ms || ready > at => ReadyCycle::Unknown,
                state => state,
            };
        }
        let mut state = match pr.created_as_draft {
            Some(false) => ReadyCycle::Ready(pr.created_at_ms),
            Some(true) => ReadyCycle::Draft,
            None => ReadyCycle::Unknown,
        };
        let mut previous: Option<(u64, LifecycleKind)> = None;
        if let Some(events) = self.events.get(key) {
            for event in events {
                if event.occurred_at_ms < pr.created_at_ms || event.occurred_at_ms > at { continue; }
                if previous.is_some_and(|(time, kind)| time == event.occurred_at_ms && kind != event.kind) { return ReadyCycle::Unknown; }
                match event.kind {
                    LifecycleKind::Ready => { if !matches!(state, ReadyCycle::Ready(_)) { state = ReadyCycle::Ready(event.occurred_at_ms); } }
                    LifecycleKind::Draft => state = ReadyCycle::Draft,
                }
                previous = Some((event.occurred_at_ms, event.kind));
            }
        }
        state
    }
}

#[cfg(test)]
#[path = "measurement_tests.rs"]
mod tests;
