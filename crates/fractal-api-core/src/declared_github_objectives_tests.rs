use super::*;
use fractal_github::measurement::measure;
use fractal_schema::github_capture::{Coverage, FactFamily, Projection, Roster, RosterMember, Source};

fn declaration(kind: &str, measurement: &str, target: &str) -> Objective {
    crate::objective::parse_document(&format!(r#"version 2
objective "delivery/measurement" {{
  about "GitHub source measurement"
  indicator "{kind}" {{
    source "github-source/test"
    roster "github-scope/test"
    timezone "UTC"
    cut "now"
    measurement "{measurement}"
  }}
  {target}
}}"#)).unwrap().remove(0)
}

fn projection(now: u64) -> Projection {
    Projection {
        source: Source { host: "example.invalid".into(), id: "github-source/test".into(), generation: 2 },
        revision: "projection-revision".into(), frontier_ms: now,
        roster: Roster { revision: "roster-revision".into(), members: vec![RosterMember { repository_id: "R1".into(), effective_from_ms: 0, effective_until_ms: None }] },
        pull_requests: vec![], lifecycle: vec![], merges: vec![],
        ready_cycles: std::collections::BTreeMap::new(),
        coverage: [FactFamily::PullRequest, FactFamily::Merge, FactFamily::Lifecycle].into_iter().map(|family| Coverage {
            repository_id: "R1".into(), family, complete: true, authoritative: true, history_from_ms: 0, through_ms: now, error: None,
        }).collect(),
    }
}

#[test]
fn parsed_measured_only_latency_has_no_success_or_budget_even_when_covered() {
    let now = 1_800_000_000_000;
    let objective = declaration("github-ready-to-merged", "only", "");
    let measured = measure(&projection(now), now, Cut::Now, "UTC", true).unwrap();
    let row = measured_row(&objective, "declaration-revision", &measured);
    assert_eq!(row["status"], "no-data");
    assert_eq!(row["verdict"], "measured-only");
    assert_eq!(row["target"], "measured only");
    assert!(row["remaining_percent"].is_null());
    assert!(row["latency_percentiles_ms"]["p95"].is_null());
    assert_eq!(row["measurement"]["days"].as_array().unwrap().len(), 14);
    assert_eq!(row["measurement"]["projection_revision"], "projection-revision");
    assert_eq!(row["measurement"]["roster_revision"], "roster-revision");
    assert_eq!(row["declaration_revision"], "declaration-revision");
}

#[test]
fn covered_zero_misses_authored_minimum_but_uncovered_zero_never_meets_or_misses() {
    let now = 1_800_000_000_000;
    let objective = declaration("github-merged-count", "targeted", "target \"per-hour\" min=7");
    let mut projection = projection(now);
    let measured = measure(&projection, now, Cut::Now, "UTC", false).unwrap();
    assert_eq!(measured_row(&objective, "rev", &measured)["status"], "missed");
    projection.coverage.clear();
    let measured = measure(&projection, now, Cut::Now, "UTC", false).unwrap();
    let row = measured_row(&objective, "rev", &measured);
    assert_eq!(row["status"], "no-data");
    assert_eq!(row["verdict"], "unknown");
    assert!(row["remaining_percent"].is_null());
}

#[test]
fn descriptive_latency_percentiles_are_not_targets() {
    assert_eq!(descriptive_percentiles(&[1, 2, 3, 4, 5]), json!({ "p50": 3, "p95": 5, "p99": 5 }));
    assert_eq!(descriptive_percentiles(&[]), json!({ "p50": null, "p95": null, "p99": null }));
}

#[test]
fn count_rows_keep_covered_days_when_lifecycle_or_a_new_member_is_uncovered() {
    let now = 1_800_000_000_000;
    let today = now / 86_400_000 * 86_400_000;
    let objective = declaration("github-merged-count", "targeted", "target \"per-hour\" min=7");
    let mut projection = projection(now);
    projection.coverage.retain(|record| record.family != FactFamily::Lifecycle);
    let measured = measure(&projection, now, Cut::Watermark, "UTC", false).unwrap();
    assert_eq!(measured_row(&objective, "revision", &measured)["status"], "missed");
    projection.roster.members.push(RosterMember {
        repository_id: "new-member".into(), effective_from_ms: today, effective_until_ms: None,
    });
    let measured = measure(&projection, now, Cut::Watermark, "UTC", false).unwrap();
    let row = measured_row(&objective, "revision", &measured);
    assert_eq!(row["verdict"], "unknown");
    let yesterday = row["measurement"]["days"].as_array().unwrap().iter()
        .find(|day| day["start_ms"] == today - 86_400_000).unwrap();
    assert_eq!(yesterday["complete"], true);
    assert_eq!(yesterday["merged_count"], 0);
}
