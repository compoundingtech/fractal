//! Pure measurements of the canonical committed projection. Never folds raw claims: a new
//! projection revision replaces this input, so corrections and retractions change old windows.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Days, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use fractal_schema::github_capture::{FactFamily, LifecycleFact, LifecycleKind, MergeFact, Projection, PullRequestFact, Source};
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
    pub days: Vec<Window>,
}

/// Fourteen civil days, including today, and exactly [cut-1h, cut). Named IANA zones
/// use their real offsets at each boundary, not a fixed 24-hour subtraction.
pub fn measure(projection: &Projection, now_ms: u64, cut: Cut, timezone: &str, latency: bool, author_logins: &[String]) -> anyhow::Result<Measurement> {
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

struct Index<'a> {
    projection: &'a Projection,
    author_logins: &'a [String],
    prs: BTreeMap<(&'a str, &'a str), &'a PullRequestFact>,
    events: BTreeMap<(&'a str, &'a str), Vec<&'a LifecycleFact>>,
    merges: BTreeMap<(&'a str, &'a str), &'a MergeFact>,
    ready_cycles: BTreeMap<(&'a str, &'a str), Option<u64>>,
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

    fn last_ready(&self, key: &(&str, &str), pr: &PullRequestFact, merged: u64) -> Option<u64> {
        if pr.created_at_ms > merged || !self.covered(&pr.repository_id, FactFamily::Lifecycle, pr.created_at_ms, merged) { return None; }
        if let Some(ready) = self.ready_cycles.get(key) {
            return ready.filter(|ready| pr.created_at_ms <= *ready && *ready <= merged);
        }
        let mut ready = match pr.created_as_draft { Some(false) => Some(pr.created_at_ms), _ => None };
        let mut previous: Option<(u64, LifecycleKind)> = None;
        if let Some(events) = self.events.get(key) {
            for event in events {
                if event.occurred_at_ms < pr.created_at_ms || event.occurred_at_ms > merged { continue; }
                // Opposing transitions at the same instant have no provable ordering.
                if previous.is_some_and(|(at, kind)| at == event.occurred_at_ms && kind != event.kind) { return None; }
                match event.kind {
                    LifecycleKind::Ready => { if ready.is_none() { ready = Some(event.occurred_at_ms); } }
                    LifecycleKind::Draft => ready = None,
                }
                previous = Some((event.occurred_at_ms, event.kind));
            }
        }
        ready
    }
}

#[cfg(test)]
#[path = "measurement_tests.rs"]
mod tests;
