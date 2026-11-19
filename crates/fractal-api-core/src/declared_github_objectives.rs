//! Source-backed declarations share the canonical projection with watches and replay.
use std::path::Path;

use fractal_github::measurement::{self, Cut, Measurement};
use serde_json::{Value, json};
use fractal_schema::github_capture::FactFamily;

use crate::objective::{Bound, Indicator, Objective};
use crate::store::Store;

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
    let measured = (|| -> anyhow::Result<Measurement> {
        let frontier = crate::reconcile::github_capture::measurement_frontier(store, state_dir, &config.source, &config.roster, required)?
            .ok_or_else(|| anyhow::anyhow!("source or roster has no committed required-family evidence horizon"))?;
        let cut = if config.as_of_watermark { Cut::Watermark } else { Cut::Now };
        let end = if config.as_of_watermark { now.min(frontier) } else { now };
        let start = measurement::history_start(end, &config.timezone)?;
        let view_id = format!("{}:{}", config.timezone, config.as_of_watermark);
        let snapshot = crate::reconcile::github_capture::measurement_snapshot(store, state_dir, &config.source, &config.roster, start..end, &view_id, required)?
            .ok_or_else(|| anyhow::anyhow!("source or roster projection is unavailable or completing a bounded snapshot page"))?;
        let projection = &snapshot.projection;
        if config.as_of_watermark && projection.frontier_ms < snapshot.end_ms {
            anyhow::bail!("pinned source frontier does not cover the snapshot cut");
        }
        // A paged read pins its own finite cut while Now and acquisition continue advancing.
        let mut measured = measurement::measure(projection, snapshot.end_ms, cut, &config.timezone, matches!(objective.indicator, Indicator::GithubReadyToMerged(_)))?;
        measured.source_lag_ms = now.saturating_sub(projection.frontier_ms);
        Ok(measured)
    })();
    match measured {
        Ok(measured) => measured_row(objective, declaration_revision, &measured),
        Err(error) => json!({
            "id": objective.name, "kind": "objective", "about": objective.about,
            "target": if config.measured_only { "measured only".to_owned() } else { target_label(objective) },
            "current": "unknown", "window": "rolling 1h", "remaining_percent": null, "status": "no-data",
            "verdict": "unknown", "declaration_revision": declaration_revision,
            "explain": format!("Committed GitHub measurement unavailable: {error}. Unknown is never met."),
        }),
    }
}

fn target_label(objective: &Objective) -> String {
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
    let window = &measured.rolling_hour;
    let (status, verdict, remaining) = if !window.complete {
        ("no-data", "unknown", None)
    } else if config.measured_only {
        ("no-data", "measured-only", None)
    } else {
        match objective.target("per-hour", false).map(|target| target.bound) {
            Some(Bound::Min(min)) => {
                let count = window.observed_merges as f64;
                let met = count >= min;
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
        format!("{count} merged PRs in the rolling hour")
    } else {
        format!("unknown ({} observed merged PRs, incomplete coverage)", window.observed_merges)
    };
    json!({
        "id": objective.name, "kind": "objective", "about": objective.about,
        "target": if config.measured_only { "measured only".to_owned() } else { target_label(objective) },
        "current": current, "window": "rolling 1h", "status": status, "verdict": verdict,
        "remaining_percent": remaining, "declaration_revision": declaration_revision,
        "measurement": measured, "latency_percentiles_ms": statistics,
        "explain": format!("Distinct PR identities across all authors in [cut-1h,cut), under effective-dated roster revision {}. Final-ready latency requires complete lifecycle history and a proven ready state; no queue or push proxy. {} Source watermark lag: {}ms. Corrections and retractions replace prior samples at projection revision {}.",
            measured.roster_revision,
            if config.measured_only { "Measured only: no target or success verdict is assigned." } else { "Only fully covered windows can meet the authored target." },
            measured.source_lag_ms, measured.projection_revision),
    })
}

fn descriptive_percentiles(sorted: &[u64]) -> Value {
    let rank = |p: usize| sorted.get((sorted.len() * p).div_ceil(100).saturating_sub(1)).copied();
    json!({ "p50": rank(50), "p95": rank(95), "p99": rank(99) })
}

#[cfg(test)]
#[path = "declared_github_objectives_tests.rs"]
mod tests;
