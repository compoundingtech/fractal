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
    let measured = measure(&projection(now), now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
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
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(measured_row(&objective, "rev", &measured)["status"], "missed");
    projection.coverage.clear();
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap();
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
    let measured = measure(&projection, now, Cut::Watermark, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(measured_row(&objective, "revision", &measured)["status"], "missed");
    projection.roster.members.push(RosterMember {
        repository_id: "new-member".into(), effective_from_ms: today, effective_until_ms: None,
    });
    let measured = measure(&projection, now, Cut::Watermark, "UTC", false, 3_600_000, &[]).unwrap();
    let row = measured_row(&objective, "revision", &measured);
    assert_eq!(row["verdict"], "unknown");
    let yesterday = row["measurement"]["days"].as_array().unwrap().iter()
        .find(|day| day["start_ms"] == today - 86_400_000).unwrap();
    assert_eq!(yesterday["complete"], true);
    assert_eq!(yesterday["merged_count"], 0);
}

#[test]
fn selected_count_rows_keep_the_native_target_and_unknown_coverage_contract() {
    let now = 1_800_000_000_000;
    let mut objective = declaration("github-merged-count", "targeted", "target \"per-hour\" min=1");
    let Indicator::GithubMergedCount(config) = &mut objective.indicator else { panic!() };
    config.author_logins = vec!["example-bot".into()];
    let mut projection = projection(now);
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 3_600_000, &config.author_logins).unwrap();
    let row = measured_row(&objective, "selected-revision", &measured);
    assert_eq!(row["status"], "missed");
    assert_eq!(row["declaration_revision"], "selected-revision");
    assert!(row["explain"].as_str().unwrap().contains("author logins [example-bot]"));
    projection.coverage.clear();
    let Indicator::GithubMergedCount(config) = &objective.indicator else { panic!() };
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 3_600_000, &config.author_logins).unwrap();
    assert_eq!(measured_row(&objective, "selected-revision", &measured)["verdict"], "unknown");
}

#[test]
fn rolling_day_minimum_is_an_hourly_rate_not_a_total_count() {
    use fractal_schema::github_capture::{MergeFact, PullRequestFact, PullRequestState};
    let now = 1_800_000_000_000;
    let objective = declaration("github-merged-count", "targeted", "window \"24h\"\n target \"per-hour\" min=5");
    let mut projection = projection(now);
    for number in 0..120 {
        let id = format!("pr-{number}");
        let merged = now - 7_200_000 - number;
        projection.pull_requests.push(PullRequestFact {
            id: id.clone(), repository_id: "R1".into(), number,
            author_login: Some("example-bot".into()), state: PullRequestState::Merged, is_draft: false,
            created_as_draft: Some(false), created_at_ms: merged - 100, closed_at_ms: Some(merged), merged_at_ms: Some(merged),
        });
        projection.merges.push(MergeFact { repository_id: "R1".into(), pull_request_id: id, merged_at_ms: merged });
    }
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 86_400_000, &[]).unwrap();
    let row = measured_row(&objective, "revision", &measured);
    assert_eq!(row["status"], "met");
    assert_eq!(row["window"], "rolling 24h");
    projection.pull_requests.pop();
    projection.merges.pop();
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 86_400_000, &[]).unwrap();
    assert_eq!(measured_row(&objective, "revision", &measured)["status"], "missed");
    for coverage in &mut projection.coverage { coverage.history_from_ms = now - 3_600_000; }
    let measured = measure(&projection, now, Cut::Now, "UTC", false, 86_400_000, &[]).unwrap();
    assert_eq!(measured_row(&objective, "revision", &measured)["verdict"], "unknown");
}

fn latency_objective() -> Objective {
    let mut objective = declaration("github-ready-to-merged", "targeted", r#"
 target "p95" within="48h"
 budget allowance=0.05 horizon="30d" min-coverage=1 min-samples=1 {
   fast burn-rate=14.4 long="1h" short="5m"
   slow burn-rate=6 long="6h" short="30m"
 }"#);
    objective.name = "delivery/merge-latency/p95".into();
    objective
}

fn latency_sample(projection: &mut Projection, id: u64, settled: u64, bad: bool) {
    use fractal_schema::github_capture::{MergeFact, PullRequestFact, PullRequestState};
    let deadline = 172_800_000;
    let ready = if bad { settled - deadline } else { settled - 1_000 };
    let merged = if bad { settled + 1 } else { settled };
    let id_string = format!("pr-{id}");
    projection.pull_requests.push(PullRequestFact {
        id: id_string.clone(), repository_id: "R1".into(), number: id, author_login: Some("example-bot".into()),
        state: PullRequestState::Merged, is_draft: false, created_as_draft: Some(false), created_at_ms: ready,
        closed_at_ms: Some(merged), merged_at_ms: Some(merged),
    });
    projection.merges.push(MergeFact { repository_id: "R1".into(), pull_request_id: id_string, merged_at_ms: merged });
}

#[test]
fn latency_ratio_keeps_authored_id_diagnostic_p95_and_inclusive_allowance() {
    let now = 1_800_000_000_000;
    let objective = latency_objective();
    let mut projection = projection(now);
    for id in 0..20 { latency_sample(&mut projection, id, now - 100_000, id == 0); }
    let row = latency_row(&objective, "revision", &projection, now, now);
    assert_eq!(row["id"], "delivery/merge-latency/p95");
    assert_eq!(row["status"], "met");
    assert_eq!(row["latency_percentiles_ms"]["p95"], 1_000);
    latency_sample(&mut projection, 20, now - 100_000, true);
    assert_eq!(latency_row(&objective, "revision", &projection, now, now)["status"], "exhausted");
    projection.coverage.clear();
    assert_eq!(latency_row(&objective, "revision", &projection, now, now)["verdict"], "unknown");
}

#[test]
fn latency_burns_require_both_windows_not_a_short_only_spike() {
    let now = 1_800_000_000_000;
    let objective = latency_objective();
    let mut projection = projection(now);
    for id in 0..400 { latency_sample(&mut projection, id, now - 86_400_000, false); }
    for id in 400..410 { latency_sample(&mut projection, id, now - 100_000, true); }
    assert_eq!(latency_row(&objective, "revision", &projection, now, now)["status"], "burning");
    for id in 410..510 { latency_sample(&mut projection, id, now - 1_800_000, false); }
    assert_eq!(latency_row(&objective, "revision", &projection, now, now)["status"], "met");
}
