//! Event-ratio and covered-time budgets over the same incremental replicated-claim cursor.
use super::*;
use crate::objective::{BudgetPolicy, BurnWindow, Gauge, Threshold};

/// Common decision input for native ledgers and replicated-claim indicators.
#[derive(Clone, Copy)]
pub(crate) struct BudgetEvidence {
    pub bad: f64,
    pub total: f64,
    pub eligible: bool,
}
impl BudgetEvidence {
    fn fraction(self) -> Option<f64> { (self.total > 0.0).then(|| self.bad / self.total) }
}

pub(crate) fn classify_budget(
    policy: &BudgetPolicy,
    horizon: BudgetEvidence,
    fast: (BudgetEvidence, BudgetEvidence),
    slow: (BudgetEvidence, BudgetEvidence),
    empty_status: &'static str,
) -> (&'static str, Option<f64>) {
    let remaining = horizon.fraction().filter(|_| horizon.eligible)
        .map(|fraction| (100.0 * (1.0 - fraction / policy.allowance)).clamp(0.0, 100.0));
    let burns = |pair: (BudgetEvidence, BudgetEvidence), rate: f64| {
        pair.0.eligible && pair.1.eligible
            && pair.0.fraction().is_some_and(|fraction| fraction / policy.allowance >= rate)
            && pair.1.fraction().is_some_and(|fraction| fraction / policy.allowance >= rate)
    };
    // Equality spends the allowance but still satisfies the declared inclusive bad-share bound.
    let status = if horizon.eligible && horizon.fraction().is_some_and(|fraction| fraction > policy.allowance) {
        "exhausted"
    } else if burns(fast, policy.fast.burn_rate) || burns(slow, policy.slow.burn_rate) {
        "burning"
    } else if horizon.total == 0.0 {
        empty_status
    } else if !horizon.eligible {
        "no-data"
    } else if !(fast.0.eligible && fast.1.eligible || slow.0.eligible && slow.1.eligible) {
        "not-eligible"
    } else {
        "met"
    };
    (status, remaining)
}

#[derive(Default)]
pub(super) struct Observations {
    // One timestamp per source: retries cannot add weight. The smallest claim ID wins ties.
    sources: BTreeMap<String, BTreeMap<u64, Observation>>,
}
struct Observation {
    id: String,
    value: Option<f64>,
    fields: BTreeMap<String, String>,
    eligible: bool,
}
impl Observations {
    pub(super) fn prune(&mut self, horizon: u64, keep_boundary: bool) {
        for points in self.sources.values_mut() {
            // Keep one expired boundary for source/label identity, never as covered time.
            let boundary = keep_boundary.then(|| points.range(..horizon).next_back().map(|(at, _)| *at)).flatten();
            points.retain(|at, _| *at >= horizon || Some(*at) == boundary);
        }
        if !keep_boundary {
            self.sources.retain(|_, points| !points.is_empty());
        }
        // Retain source identities: a silent known source contributes unknown time, not health.
    }

    fn measure(&self, from: u64, to: u64, threshold: Threshold, gauge: Option<&Gauge>, group: Option<(&str, &str)>) -> Measurement {
        let mut measured = Measurement::default();
        for points in self.sources.values() {
            let matches = |point: &Observation| group.is_none_or(|(key, value)| point.fields.get(key).is_some_and(|field| field == value));
            if let Some(gauge) = gauge {
                let mut iter = points.iter().peekable();
                let mut first = true;
                while let Some((&at, point)) = iter.next() {
                    let next = iter.peek().map_or(to, |(next, _)| **next).min(to);
                    // Membership lasts until the next sample, even after the value expires.
                    // Before discovery, an eligible source is unknown, never healthy; an
                    // excluded source contributes neither covered nor possible time.
                    let membership_start = if first { from } else { at.max(from) };
                    first = false;
                    if !matches(point) || !point.eligible {
                        continue;
                    }
                    measured.possible += next.saturating_sub(membership_start) as f64;
                    let end = at.saturating_add(gauge.max_age_ms).min(next);
                    let start = at.max(from);
                    if end <= start {
                        continue;
                    }
                    let Some(value) = point.value else { continue };
                    let duration = (end - start) as f64;
                    measured.total += duration;
                    measured.bad += if threshold.is_bad(value) { duration } else { 0.0 };
                    measured.samples += 1;
                }
            } else {
                for (_, point) in points.range(from..to) {
                    if !matches(point) { continue; }
                    if !point.eligible { measured.excluded += 1; continue; }
                    let Some(value) = point.value else { measured.unknown += 1; continue };
                    measured.samples += 1;
                    measured.total += 1.0;
                    measured.bad += if threshold.is_bad(value) { 1.0 } else { 0.0 };
                }
            }
        }
        measured
    }
}

#[derive(Default, Debug)]
struct Measurement {
    total: f64,
    bad: f64,
    possible: f64,
    samples: u64,
    excluded: u64,
    unknown: u64,
}
impl Measurement {
    fn fraction(&self) -> Option<f64> { (self.total > 0.0).then(|| self.bad / self.total) }
    fn coverage(&self) -> f64 { if self.possible > 0.0 { self.total / self.possible } else { 0.0 } }
    fn eligible(&self, policy: &BudgetPolicy, gauge: bool) -> bool {
        self.samples >= policy.min_samples && self.total > 0.0
            && if gauge { self.coverage() >= policy.min_coverage } else { self.unknown == 0 }
    }
}

impl Tracked {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn observe_numeric(&mut self, claim: &ClaimRecord, at: &str, field: &str, labels: &[String], excludes: &[Exclude], only: &[Only]) {
        let fields = claim.body.get("fields").and_then(Value::as_object);
        let Some(at) = fields.and_then(|fields| fields.get(at)).and_then(time_ms) else {
            self.unreadable += 1;
            return;
        };
        let value = fields.and_then(|fields| fields.get(field)).and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok())).filter(|value| value.is_finite());
        if value.is_none() { self.unreadable += 1; }
        // Preserve source discovery independently of bounded observation retention.
        self.seen |= value.is_some();
        let kept = kept_fields(labels, excludes, only, fields);
        let eligible = holds(only, &kept) && !excludes.iter().any(|exclude| kept.get(&exclude.field).is_some_and(|value| exclude.values.contains(value)));
        let observation = Observation { id: claim.id.clone(), value, fields: kept, eligible };
        let points = self.numeric.sources.entry(claim.subject.clone()).or_default();
        match points.get_mut(&at) {
            Some(existing) if claim.id < existing.id => *existing = observation,
            Some(_) => {}
            None => { points.insert(at, observation); }
        }
    }

    pub(super) fn budget_rows(&self, subject: &str, gauge: Option<&Gauge>, now: u64, response: &str) -> Vec<Value> {
        let Ok(objective) = &self.objective else { return Vec::new() };
        let Some(policy) = &objective.budget else { return vec![self.unreadable_row(subject, &objective.name)] };
        let (threshold, labels) = match &objective.indicator {
            Indicator::Ratio(ratio) => (Threshold::Above(ratio.bad_above), &ratio.labels),
            Indicator::Gauge(gauge) => (gauge.threshold, &gauge.labels),
            _ => return Vec::new(),
        };
        let mut groups = BTreeSet::new();
        for points in self.numeric.sources.values() {
            for point in points.values() {
                for label in labels {
                    if let Some(value) = point.fields.get(label) { groups.insert((label.as_str(), value.as_str())); }
                }
            }
        }
        let mut rows = vec![self.budget_row(subject, policy, threshold, gauge, now, None, response)];
        rows.extend(groups.into_iter().map(|group| self.budget_row(subject, policy, threshold, gauge, now, Some(group), response)));
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn budget_row(&self, subject: &str, policy: &BudgetPolicy, threshold: Threshold, gauge: Option<&Gauge>, now: u64, group: Option<(&str, &str)>, response: &str) -> Value {
        let objective = self.objective.as_ref().expect("decoded objective");
        let measure = |window: u64| self.numeric.measure(now.saturating_sub(window), now, threshold, gauge, group);
        let horizon = measure(policy.horizon_ms);
        let horizon_eligible = horizon.eligible(policy, gauge.is_some());
        let evidence = |measurement: &Measurement| BudgetEvidence { bad: measurement.bad, total: measurement.total, eligible: measurement.eligible(policy, gauge.is_some()) };
        let pair = |name: &str, window: &BurnWindow| {
            let short_ms = gauge.map_or(window.short_ms, |gauge| window.short_ms.max(gauge.cadence_ms));
            let long_ms = gauge.map_or(window.long_ms, |gauge| window.long_ms.max(gauge.cadence_ms.saturating_mul(2)));
            let long = measure(long_ms);
            let short = measure(short_ms);
            let eligible = long.eligible(policy, gauge.is_some()) && short.eligible(policy, gauge.is_some());
            let long_burn = long.fraction().map(|fraction| fraction / policy.allowance);
            let short_burn = short.fraction().map(|fraction| fraction / policy.allowance);
            let pair_evidence = (evidence(&long), evidence(&short));
            let render = |burn: Option<f64>| burn.map_or_else(|| "unknown".into(), |burn| format!("{burn:.2}x"));
            (pair_evidence, format!("{name} paired {}+{} at {:.1}x: long {}, short {}, eligible={eligible}, coverage {:.1}%/{:.1}%", span(long_ms), span(short_ms), window.burn_rate, render(long_burn), render(short_burn), long.coverage() * 100.0, short.coverage() * 100.0))
        };
        let fast = pair("fast", &policy.fast);
        let slow = pair("slow", &policy.slow);
        let empty_status = if gauge.is_some() || horizon.unknown > 0 || (!self.seen && horizon.excluded == 0) { "no-data" } else { "no-qualifying-events" };
        let (status, remaining) = classify_budget(policy, evidence(&horizon), fast.0, slow.0, empty_status);
        let id = group.map_or_else(|| objective.name.clone(), |(label, value)| format!("{}{{{label}={value}}}", objective.name));
        let current = if gauge.is_some() {
            format!("{:.0} bad ms / {:.0} covered ms; {:.1}% coverage", horizon.bad, horizon.total, horizon.coverage() * 100.0)
        } else {
            format!("{:.0} bad / {:.0} qualifying events", horizon.bad, horizon.total)
        };
        json!({
            "id": id, "kind": "objective", "about": objective.about,
            "target": format!("≤ {:.2}% bad {}", policy.allowance * 100.0, if gauge.is_some() { "covered time" } else { "events" }),
            "current": current, "window": span(policy.horizon_ms), "remaining_percent": remaining, "status": status,
            "explain": format!("Declared as {subject} at revision {}. {current}; {} contributing samples, {} excluded events, {} unknown observations, {} unreadable claims. Unknown time is never healthy time; a gauge is held only until max-age or its next sample. Horizon eligible={horizon_eligible}; minimum coverage {:.1}%, minimum samples {}. {}; {}. Gauge pair short windows are at least one source cadence and long windows at least two; a short-only spike never changes status. {response}", self.short_revision(), horizon.samples, horizon.excluded, horizon.unknown, self.unreadable, policy.min_coverage * 100.0, policy.min_samples, fast.1, slow.1),
        })
    }
}

#[cfg(test)]
#[path = "declared_objectives_budget_tests.rs"]
mod tests;
