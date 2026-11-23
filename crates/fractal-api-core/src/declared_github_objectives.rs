//! Source-backed declarations share the canonical projection with watches and replay.
use std::path::Path;

use fractal_github::measurement::{self, Cut, Measurement};
use serde_json::{Value, json};
use fractal_schema::github_capture::FactFamily;

use crate::objective::{Bound, Indicator, Objective};
use crate::store::Store;
use super::{BudgetEvidence, classify_budget};

pub(super) fn row(store: &Store, state_dir: &Path, objective: &Objective, declaration_revision: &str, now: u64) -> Value {
    let config = match &objective.indicator {
        Indicator::GithubMergedCount(config) | Indicator::GithubReadyToMerged(config) => config,
        _ => unreachable!("only GitHub declarations reach the domain evaluator"),
    };
    let required = match objective.indicator {
        Indicator::GithubMergedCount(_) => &[FactFamily::PullRequest, FactFamily::Merge][..],
        Indicator::GithubReadyToMerged(_) => &[FactFamily::PullRequest, FactFamily::Merge, FactFamily::Lifecycle][..],
        _ => unreachable!("only GitHub declarations reach the domain evaluator"),
    };
    let measured = (|| -> anyhow::Result<Value> {
        let frontier = crate::reconcile::github_capture::measurement_frontier(store, state_dir, &config.source, &config.roster, required)?
            .ok_or_else(|| anyhow::anyhow!("source or roster has no committed required-family evidence horizon"))?;
        let cut = if config.as_of_watermark { Cut::Watermark } else { Cut::Now };
        let end = if config.as_of_watermark { now.min(frontier) } else { now };
        let rolling_ms = objective.budget.map_or_else(|| objective.window.map_or(3_600_000, |window| window.ms), |policy| policy.horizon_ms);
        let start = measurement::history_start(end, &config.timezone)?.min(end.saturating_sub(rolling_ms));
        let view_id = format!("{}:{}:{rolling_ms}", config.timezone, config.as_of_watermark);
        let snapshot = crate::reconcile::github_capture::measurement_snapshot(store, state_dir, &config.source, &config.roster, start..end,
            crate::reconcile::github_capture::MeasurementView { id: &view_id, required, include_unmerged: objective.budget.is_some() })?
            .ok_or_else(|| anyhow::anyhow!("source or roster projection is unavailable or completing a bounded snapshot page"))?;
        let projection = &snapshot.projection;
        if config.as_of_watermark && projection.frontier_ms < snapshot.end_ms {
            anyhow::bail!("pinned source frontier does not cover the snapshot cut");
        }
        if matches!(objective.indicator, Indicator::GithubReadyToMerged(_)) && objective.budget.is_some() {
            return Ok(latency_row(objective, declaration_revision, projection, snapshot.end_ms, now));
        }
        // A paged read pins its own finite cut while Now and acquisition continue advancing.
        let mut measured = measurement::measure(projection, snapshot.end_ms, cut, &config.timezone, matches!(objective.indicator, Indicator::GithubReadyToMerged(_)), rolling_ms, &config.author_logins)?;
        measured.source_lag_ms = now.saturating_sub(projection.frontier_ms);
        Ok(measured_row(objective, declaration_revision, &measured))
    })();
    match measured {
        Ok(row) => row,
        Err(error) => json!({
            "id": objective.name, "kind": "objective", "about": objective.about,
            "target": if config.measured_only { "measured only".to_owned() } else { target_label(objective) },
            "current": "unknown", "window": window_label(objective), "remaining_percent": null, "status": "no-data",
            "verdict": "unknown", "declaration_revision": declaration_revision,
            "explain": format!("Committed GitHub measurement unavailable: {error}. Unknown is never met."),
        }),
    }
}

fn window_label(objective: &Objective) -> String {
    let ms = objective.budget.map_or_else(|| objective.window.map_or(3_600_000, |window| window.ms), |policy| policy.horizon_ms);
    if objective.budget.is_some() && ms.is_multiple_of(86_400_000) { return format!("rolling {}d", ms / 86_400_000); }
    if ms.is_multiple_of(3_600_000) { format!("rolling {}h", ms / 3_600_000) }
    else { format!("rolling {ms}ms") }
}

fn target_label(objective: &Objective) -> String {
    if let Some(target) = objective.target("p95", false) && let Bound::Within(ms) = target.bound {
        return format!("≥ 95% merged within {ms}ms of final ready");
    }
    match objective.target("per-hour", false).map(|target| target.bound) {
        Some(Bound::Min(min)) => format!("≥ {min} merged PRs per hour"),
        _ => "measured only".into(),
    }
}

/// Keeps the existing SLO row contract: descriptive-only observations are `no-data` for
/// legacy verdict consumers, with an explicit native verdict, never an invented green target.
fn measured_row(objective: &Objective, declaration_revision: &str, measured: &Measurement) -> Value {
    let (config, latency) = match &objective.indicator {
        Indicator::GithubMergedCount(config) => (config, false),
        Indicator::GithubReadyToMerged(config) => (config, true),
        _ => unreachable!("only GitHub declarations reach the domain evaluator"),
    };
    let window = measured.rolling_window.as_ref().unwrap_or(&measured.rolling_hour);
    let (status, verdict, remaining) = if !window.complete {
        ("no-data", "unknown", None)
    } else if config.measured_only {
        ("no-data", "measured-only", None)
    } else {
        match objective.target("per-hour", false).map(|target| target.bound) {
            Some(Bound::Min(min)) => {
                let per_hour = window.observed_merges as f64 * 3_600_000.0 / objective.window.map_or(3_600_000, |window| window.ms) as f64;
                let count = per_hour;
                let met = per_hour >= min;
                let remaining = if count > 0.0 { (100.0 * (1.0 - min / count)).clamp(0.0, 100.0) } else { 0.0 };
                (if met { "met" } else { "missed" }, if met { "met" } else { "missed" }, Some(remaining))
            }
            _ => ("no-data", "unknown", None),
        }
    };
    let statistics = descriptive_percentiles(&window.ready_to_merged_ms);
    let current = if latency {
        format!("{} proven final-ready→merged samples; {} unknown cycles", window.ready_to_merged_ms.len(), window.unknown_cycles)
    } else if let Some(count) = window.merged_count {
        format!("{count} merged PRs in {}", window_label(objective))
    } else {
        format!("unknown ({} observed merged PRs, incomplete coverage)", window.observed_merges)
    };
    json!({
        "id": objective.name, "kind": "objective", "about": objective.about,
        "target": if config.measured_only { "measured only".to_owned() } else { target_label(objective) },
        "current": current, "window": window_label(objective), "status": status, "verdict": verdict,
        "remaining_percent": remaining, "declaration_revision": declaration_revision,
        "measurement": measured, "latency_percentiles_ms": statistics,
        "explain": format!("Distinct PR identities for {} in the authored half-open {} window ending at the cut, under effective-dated roster revision {}. Final-ready latency requires complete lifecycle history and a proven ready state; no queue or push proxy. {} Source watermark lag: {}ms. Corrections and retractions replace prior samples at projection revision {}.",
            if config.author_logins.is_empty() { "all authors".to_owned() } else { format!("author logins [{}]", config.author_logins.join(", ")) },
            window_label(objective),
            measured.roster_revision,
            if config.measured_only { "Measured only: no target or success verdict is assigned." } else { "Only fully covered windows can meet the authored target." },
            measured.source_lag_ms, measured.projection_revision),
    })
}

fn latency_row(objective: &Objective, revision: &str, projection: &fractal_schema::github_capture::Projection, cut_ms: u64, now: u64) -> Value {
    let Indicator::GithubReadyToMerged(config) = &objective.indicator else { unreachable!() };
    let policy = objective.budget.as_ref().expect("validated latency budget");
    let within_ms = objective.target("p95", false).and_then(|target| target.within_ms()).expect("validated latency deadline");
    let durations = [policy.horizon_ms, policy.fast.long_ms, policy.fast.short_ms, policy.slow.long_ms, policy.slow.short_ms];
    let measured = measurement::latency_budget(projection, cut_ms, within_ms, &durations, &config.author_logins);
    let evidence = |index: usize| {
        let window = &measured.windows[index];
        BudgetEvidence { bad: window.bad as f64, total: window.total as f64, eligible: window.complete && window.total >= policy.min_samples }
    };
    let (status, remaining) = classify_budget(policy, evidence(0), (evidence(1), evidence(2)), (evidence(3), evidence(4)), "no-qualifying-events");
    // Empty, uncovered source history must never masquerade as a known empty cohort.
    let status = if !measured.windows[0].complete { "no-data" } else { status };
    let verdict = match status { "met" => "met", "burning" | "exhausted" => "missed", _ => "unknown" };
    let horizon = &measured.windows[0];
    json!({
        "id": objective.name, "kind": "objective", "about": objective.about,
        "target": target_label(objective), "window": window_label(objective),
        "current": format!("{} bad / {} settled final-ready cycles", horizon.bad, horizon.total),
        "status": status, "verdict": verdict, "remaining_percent": remaining,
        "declaration_revision": revision, "measurement": measured,
        "latency_percentiles_ms": descriptive_percentiles(&measured.merged_latency_ms),
        "projection_revision": projection.revision, "roster_revision": projection.roster.revision,
        "source_lag_ms": now.saturating_sub(projection.frontier_ms),
        "explain": format!("Final-ready cycles settle at a timely merge or at their deadline; open and late-merged PRs count bad at the deadline, never at later completion. A close before the deadline cancels an unmerged cycle. Pending and proven draft cycles do not vote. Full source coverage and at least {} settled cycles are required in each eligible window; unknown is never healthy. Horizon allowance {:.2}%; fast paired {:.1}x over {}ms+{}ms; slow paired {:.1}x over {}ms+{}ms. Diagnostic p95 describes completed merges, not censoring. Author logins: {}.",
            policy.min_samples, policy.allowance * 100.0, policy.fast.burn_rate, policy.fast.long_ms, policy.fast.short_ms,
            policy.slow.burn_rate, policy.slow.long_ms, policy.slow.short_ms,
            if config.author_logins.is_empty() { "all".to_owned() } else { config.author_logins.join(", ") }),
    })
}

fn descriptive_percentiles(sorted: &[u64]) -> Value {
    let rank = |p: usize| sorted.get((sorted.len() * p).div_ceil(100).saturating_sub(1)).copied();
    json!({ "p50": rank(50), "p95": rank(95), "p99": rank(99) })
}

#[cfg(test)]
#[path = "declared_github_objectives_tests.rs"]
mod tests;
