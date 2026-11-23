use super::*;
use fractal_schema::github_capture::{Coverage, PullRequestState, Roster, RosterMember};

fn at(value: &str) -> u64 { DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis().try_into().unwrap() }
fn projection(now: u64) -> Projection {
    Projection {
        source: Source { host: "example.invalid".into(), id: "source/test".into(), generation: 1 },
        revision: "projection-1".into(), frontier_ms: now,
        roster: Roster { revision: "roster-1".into(), members: vec![RosterMember { repository_id: "R1".into(), effective_from_ms: 0, effective_until_ms: None }] },
        pull_requests: vec![], lifecycle: vec![], merges: vec![],
        ready_cycles: BTreeMap::new(),
        coverage: [FactFamily::PullRequest, FactFamily::Merge, FactFamily::Lifecycle].into_iter().map(|family| Coverage {
            repository_id: "R1".into(), family, complete: true, authoritative: true, history_from_ms: 0, through_ms: now, error: None,
        }).collect(),
    }
}
fn merge(p: &mut Projection, id: &str, created: u64, merged: u64, author: Option<&str>) {
    p.pull_requests.push(PullRequestFact {
        id: id.into(), repository_id: "R1".into(), number: 1, author_login: author.map(str::to_owned),
        state: PullRequestState::Merged, is_draft: false, created_as_draft: Some(false), created_at_ms: created,
        closed_at_ms: Some(merged), merged_at_ms: Some(merged),
    });
    p.merges.push(MergeFact { pull_request_id: id.into(), repository_id: "R1".into(), merged_at_ms: merged });
}
fn event(p: &mut Projection, id: &str, kind: LifecycleKind, when: u64) {
    p.lifecycle.push(LifecycleFact { id: format!("event-{}", p.lifecycle.len()), repository_id: "R1".into(), pull_request_id: id.into(), kind, occurred_at_ms: when });
}

#[test]
fn rolling_endpoints_all_authors_and_identity_deduplication() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "start", now - 4_000_000, now - 3_600_000, Some("automation"));
    merge(&mut p, "inside", now - 4_000_000, now - 1, None);
    merge(&mut p, "end", now - 4_000_000, now, Some("human"));
    merge(&mut p, "before", now - 4_000_000, now - 3_600_001, Some("another"));
    p.merges.push(p.merges[0].clone());
    let m = measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(m.rolling_hour.merged_count, Some(2));
}

#[test]
fn corrected_images_and_retractions_replace_prior_measurements() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 5_000, now - 1, Some("human"));
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(1));
    p.revision = "projection-2".into();
    p.merges[0].merged_at_ms = now - 4_000_000;
    p.pull_requests[0].merged_at_ms = Some(now - 4_000_000);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(0));
    p.merges.clear();
    p.pull_requests.clear();
    let m = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
    assert_eq!(m.rolling_hour.merged_count, Some(0));
    assert!(m.rolling_hour.ready_to_merged_ms.is_empty());
}

#[test]
fn last_ready_cycle_and_created_ready_require_complete_history() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 100_000, now - 1_000, None);
    event(&mut p, "P1", LifecycleKind::Draft, now - 80_000);
    event(&mut p, "P1", LifecycleKind::Ready, now - 60_000);
    event(&mut p, "P1", LifecycleKind::Draft, now - 40_000);
    event(&mut p, "P1", LifecycleKind::Ready, now - 20_000);
    event(&mut p, "P1", LifecycleKind::Ready, now - 10_000);
    let m = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
    assert_eq!(m.rolling_hour.ready_to_merged_ms, [19_000]);
    p.lifecycle.clear();
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.ready_to_merged_ms, [99_000]);
    p.pull_requests[0].created_as_draft = None;
    let m = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
    assert_eq!(m.rolling_hour.unknown_cycles, 1);
    assert!(!m.rolling_hour.complete);
    p.pull_requests[0].created_as_draft = Some(false);
    p.coverage.retain(|c| c.family != FactFamily::Lifecycle);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.unknown_cycles, 1);
}

#[test]
fn covered_empty_is_zero_unavailable_member_is_unknown_and_dates_are_effective() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(0));
    p.roster.members.push(RosterMember { repository_id: "R2".into(), effective_from_ms: now - 100, effective_until_ms: None });
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, None);
    p.roster.members[1].effective_from_ms = now;
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(0));
    p.coverage.clear();
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, None);
}

#[test]
fn before_earliest_roster_evidence_is_unknown_not_a_covered_zero() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    p.roster.members[0].effective_from_ms = now - 1_800_000;
    let m = measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(m.rolling_hour.merged_count, None);
    assert!(!m.rolling_hour.complete);
    assert!(m.days.iter().all(|day| day.merged_count.is_none()));
    p.roster.members[0].effective_from_ms = now - 3_600_000;
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(0));
}

#[test]
fn civil_bars_honor_both_dst_transitions_and_today_bounds() {
    for (now, day, hours) in [("2026-03-10T16:00:00Z", "2026-03-08T05:00:00Z", 23), ("2026-11-03T17:00:00Z", "2026-11-01T04:00:00Z", 25)] {
        let now = at(now);
        let m = measure(&projection(now), now, Cut::Now, "America/New_York", false, 3_600_000, &[]).unwrap();
        assert_eq!(m.days.len(), 14);
        let bar = m.days.iter().find(|bar| bar.start_ms == at(day)).unwrap();
        assert_eq!(bar.end_ms - bar.start_ms, hours * 3_600_000);
        assert!(!bar.partial_today);
        let today = m.days.last().unwrap();
        assert!(today.partial_today);
        assert_eq!(today.end_ms, now);
        assert!(today.calendar_end_ms.unwrap() > now);
        assert_eq!(history_start(now, "America/New_York").unwrap(), m.days[0].start_ms);
    }
}

#[test]
fn partial_now_and_explicit_watermark_have_different_cuts_and_evidence() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now - 600_000);
    merge(&mut p, "P1", now - 700_000, now - 650_000, None);
    let partial = measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(partial.rolling_hour.merged_count, None);
    assert_eq!(partial.rolling_hour.observed_merges, 1);
    let watermark = measure(&p, now, Cut::Watermark, "UTC", false, 3_600_000, &[]).unwrap();
    assert_eq!(watermark.rolling_hour.merged_count, Some(1));
    assert_eq!(watermark.cut_ms, p.frontier_ms);
    assert_eq!(watermark.source_lag_ms, 600_000);
    assert_eq!(watermark.projection_revision, p.revision);
    assert_eq!(watermark.roster_revision, p.roster.revision);
}

#[test]
fn lifecycle_corrections_and_retractions_recompute_the_final_cycle() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 100_000, now - 1_000, None);
    p.pull_requests[0].created_as_draft = Some(true);
    event(&mut p, "P1", LifecycleKind::Ready, now - 80_000);
    event(&mut p, "P1", LifecycleKind::Draft, now - 40_000);
    event(&mut p, "P1", LifecycleKind::Ready, now - 20_000);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.ready_to_merged_ms, [19_000]);
    p.lifecycle[2].occurred_at_ms = now - 10_000;
    p.revision = "corrected".into();
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.ready_to_merged_ms, [9_000]);
    p.lifecycle.pop();
    let retracted = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
    assert!(retracted.rolling_hour.ready_to_merged_ms.is_empty());
    assert_eq!(retracted.rolling_hour.unknown_cycles, 1);
}

#[test]
fn missing_pr_and_missing_precreation_coverage_remain_unknown() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 7_200_000, now - 1_000, None);
    p.coverage.iter_mut().find(|c| c.family == FactFamily::Lifecycle).unwrap().history_from_ms = now - 3_600_000;
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.unknown_cycles, 1);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(1));
    p.pull_requests.clear();
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, None);
}

#[test]
fn canonical_indexed_ready_proofs_match_full_history_and_preserve_unknowns() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 100_000, now - 1_000, None);
    event(&mut p, "P1", LifecycleKind::Draft, now - 80_000);
    event(&mut p, "P1", LifecycleKind::Ready, now - 20_000);
    let full = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
    p.ready_cycles.insert(("R1".into(), "P1".into()), ReadyCycle::Ready(now - 20_000));
    p.lifecycle.clear();
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.ready_to_merged_ms, full.rolling_hour.ready_to_merged_ms);
    for proof in [ReadyCycle::Unknown, ReadyCycle::Ready(now - 100_001), ReadyCycle::Ready(now)] {
        p.ready_cycles.insert(("R1".into(), "P1".into()), proof);
        let measured = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap();
        assert!(measured.rolling_hour.ready_to_merged_ms.is_empty());
        assert_eq!(measured.rolling_hour.unknown_cycles, 1);
    }
    p.ready_cycles.insert(("R1".into(), "P1".into()), ReadyCycle::Ready(now - 20_000));
    p.coverage.retain(|coverage| coverage.family != FactFamily::Lifecycle);
    assert_eq!(measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &[]).unwrap().rolling_hour.unknown_cycles, 1);
}

#[test]
fn merged_count_does_not_require_lifecycle_coverage() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", now - 100_000, now - 1_000, None);
    p.coverage.retain(|coverage| coverage.family != FactFamily::Lifecycle);
    assert_eq!(measure(&p, now, Cut::Watermark, "UTC", false, 3_600_000, &[]).unwrap().rolling_hour.merged_count, Some(1));
    let latency = measure(&p, now, Cut::Watermark, "UTC", true, 3_600_000, &[]).unwrap();
    assert!(!latency.rolling_hour.complete);
    assert_eq!(latency.rolling_hour.unknown_cycles, 1);
}

#[test]
fn uncovered_current_members_and_families_preserve_covered_civil_days() {
    let now = at("2026-11-17T12:00:00Z");
    let today = at("2026-11-17T00:00:00Z");
    let yesterday = at("2026-11-16T00:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "P1", yesterday + 1_000, yesterday + 2_000, None);
    p.roster.members.push(RosterMember {
        repository_id: "R2".into(), effective_from_ms: today, effective_until_ms: None,
    });
    p.coverage.iter_mut().find(|coverage| coverage.family == FactFamily::Lifecycle).unwrap().through_ms = today;
    for cut in [Cut::Now, Cut::Watermark] {
        for latency in [false, true] {
            let measured = measure(&p, now, cut, "UTC", latency, 3_600_000, &[]).unwrap();
            assert!(!measured.rolling_hour.complete);
            assert!(!measured.days.last().unwrap().complete);
            let covered = measured.days.iter().find(|day| day.start_ms == yesterday).unwrap();
            assert!(covered.complete);
            assert_eq!(covered.merged_count, Some(1));
            if latency { assert_eq!(covered.ready_to_merged_ms, [1_000]); }
        }
    }
}

#[test]
fn author_cohort_filters_rolling_and_civil_windows_without_masking_unknowns() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "selected", now - 5_000, now - 1_000, Some("Example-Bot"));
    merge(&mut p, "outside", now - 5_000, now - 2_000, Some("other"));
    merge(&mut p, "yesterday", now - 86_405_000, now - 86_400_000, Some("example-bot"));
    let authors = vec!["example-bot".into()];
    let measured = measure(&p, now, Cut::Now, "UTC", true, 3_600_000, &authors).unwrap();
    assert_eq!(measured.rolling_hour.merged_count, Some(1));
    assert_eq!(measured.rolling_hour.ready_to_merged_ms, [4_000]);
    assert_eq!(measured.days.iter().map(|day| day.merged_count.unwrap()).sum::<u64>(), 2);
    p.pull_requests[0].author_login = None;
    assert!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &authors).unwrap().rolling_hour.merged_count.is_none());
    p.pull_requests[0].author_login = Some("outside".into());
    assert_eq!(measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &authors).unwrap().rolling_hour.merged_count, Some(0));
    p.coverage.clear();
    let unknown = measure(&p, now, Cut::Now, "UTC", false, 3_600_000, &authors).unwrap();
    assert!(unknown.rolling_hour.merged_count.is_none());
    assert!(unknown.days.iter().all(|day| day.merged_count.is_none()));
}

#[test]
fn authored_day_window_keeps_hourly_diagnostics_and_requires_the_entire_window() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "older", now - 86_400_000, now - 7_200_000, Some("Example-Bot"));
    merge(&mut p, "recent", now - 10_000, now - 1, Some("outside"));
    let authors = vec!["example-bot".into()];
    let measured = measure(&p, now, Cut::Now, "UTC", false, 86_400_000, &authors).unwrap();
    assert_eq!(measured.rolling_hour.merged_count, Some(0));
    assert_eq!(measured.rolling_window.unwrap().merged_count, Some(1));
    for coverage in &mut p.coverage { coverage.history_from_ms = now - 3_600_000; }
    let measured = measure(&p, now, Cut::Now, "UTC", false, 86_400_000, &authors).unwrap();
    assert!(measured.rolling_hour.complete);
    assert!(!measured.rolling_window.unwrap().complete);
}

#[test]
fn latency_deadline_censors_open_and_late_merges_once_without_moving_the_event() {
    let now = at("2026-11-17T12:00:00Z");
    let deadline = 48 * 3_600_000;
    let mut p = projection(now);
    merge(&mut p, "on-time", now - 20_000, now - 10_000, Some("example-bot"));
    merge(&mut p, "late", now - deadline - 1_000, now - 1, Some("example-bot"));
    merge(&mut p, "open", now - deadline, now - 1, Some("example-bot"));
    p.merges.pop();
    let open = p.pull_requests.last_mut().unwrap();
    open.state = PullRequestState::Open;
    open.merged_at_ms = None;
    open.closed_at_ms = None;
    let measured = latency_budget(&p, now, deadline, &[30 * 86_400_000, 300_000], &[]);
    assert_eq!((measured.windows[0].total, measured.windows[0].bad), (2, 1));
    assert_eq!((measured.windows[1].total, measured.windows[1].bad), (2, 1));
    assert!(measured.windows.iter().all(|window| window.complete));
    p.frontier_ms = now + 1;
    for coverage in &mut p.coverage { coverage.through_ms = now + 1; }
    let after_cut = latency_budget(&p, now + 1, deadline, &[30 * 86_400_000, 300_000], &[]);
    assert_eq!((after_cut.windows[0].total, after_cut.windows[0].bad), (3, 2));
    let later = now + 400_000;
    p.frontier_ms = later;
    for coverage in &mut p.coverage { coverage.through_ms = later; }
    let measured = latency_budget(&p, later, deadline, &[30 * 86_400_000, 300_000], &[]);
    assert_eq!((measured.windows[0].total, measured.windows[0].bad), (3, 2));
    assert_eq!(measured.windows[1].total, 0);
}

#[test]
fn latency_pending_draft_cancellation_final_ready_and_unknown_are_distinct() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    for id in ["pending", "draft", "cancelled", "reset"] {
        merge(&mut p, id, now - 10_000, now - 1, Some("example-bot"));
        p.merges.pop();
        let pr = p.pull_requests.last_mut().unwrap();
        pr.state = PullRequestState::Open;
        pr.closed_at_ms = None;
        pr.merged_at_ms = None;
    }
    p.pull_requests[0].created_at_ms = now - 1_000;
    p.pull_requests[1].created_as_draft = Some(true);
    p.pull_requests[1].is_draft = true;
    p.pull_requests[2].closed_at_ms = Some(now - 9_000);
    event(&mut p, "reset", LifecycleKind::Draft, now - 8_000);
    event(&mut p, "reset", LifecycleKind::Ready, now - 1_000);
    let authors = vec!["example-bot".into()];
    let measured = latency_budget(&p, now, 5_000, &[86_400_000], &authors);
    assert_eq!(measured.windows[0].total, 0);
    assert!(measured.windows[0].complete);
    p.pull_requests[0].author_login = None;
    assert!(!latency_budget(&p, now, 5_000, &[86_400_000], &authors).windows[0].complete);
    p.pull_requests[0].author_login = Some("outside".into());
    p.coverage.retain(|coverage| coverage.family != FactFamily::Lifecycle);
    assert!(!latency_budget(&p, now, 5_000, &[86_400_000], &authors).windows[0].complete);
}

#[test]
fn latency_orphan_merge_cannot_disappear_from_a_known_cohort() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "known", now - 10_000, now - 1, Some("example-bot"));
    merge(&mut p, "orphan", now - 10_000, now, Some("example-bot"));
    p.pull_requests.pop();
    let measured = latency_budget(&p, now, 172_800_000, &[30 * 86_400_000], &["example-bot".into()]);
    assert!(measured.windows[0].complete, "the orphan merge at the exclusive cut is not yet in the window");
    p.frontier_ms = now + 1;
    for coverage in &mut p.coverage { coverage.through_ms = now + 1; }
    let measured = latency_budget(&p, now + 1, 172_800_000, &[30 * 86_400_000], &["example-bot".into()]);
    assert!(!measured.windows[0].complete);
    assert!(measured.unknown.iter().any(|reason| reason.contains("missing pull request")));
}

#[test]
fn latency_membership_and_cohort_exclude_irrelevant_unknown_pr_evidence() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "known", now - 10_000, now - 1, Some("example-bot"));
    let mut irrelevant = p.pull_requests[0].clone();
    irrelevant.created_as_draft = None;
    irrelevant.merged_at_ms = None;
    irrelevant.closed_at_ms = None;
    irrelevant.state = PullRequestState::Open;
    irrelevant.author_login = None;
    irrelevant.repository_id = "outside-roster".into();
    irrelevant.id = "outside-roster-pr".into();
    p.pull_requests.push(irrelevant.clone());
    p.roster.members.push(RosterMember { repository_id: "retired".into(), effective_from_ms: 0, effective_until_ms: Some(now - 20_000) });
    irrelevant.repository_id = "retired".into();
    irrelevant.id = "after-retirement".into();
    p.pull_requests.push(irrelevant.clone());
    p.roster.members.push(RosterMember { repository_id: "future".into(), effective_from_ms: now + 1_000, effective_until_ms: None });
    irrelevant.repository_id = "future".into();
    irrelevant.id = "before-membership".into();
    p.pull_requests.push(irrelevant.clone());
    irrelevant.repository_id = "R1".into();
    irrelevant.id = "outside-cohort".into();
    irrelevant.author_login = Some("another-author".into());
    p.pull_requests.push(irrelevant);
    // The retired membership is outside this window; none of its evidence is required.
    let authors = ["example-bot".into()];
    let measured = latency_budget(&p, now, 172_800_000, &[10_000], &authors);
    assert!(measured.windows[0].complete, "{:?}", measured.unknown);
    assert_eq!((measured.windows[0].total, measured.windows[0].bad), (1, 0));
    p.pull_requests.last_mut().unwrap().author_login = Some("example-bot".into());
    assert!(!latency_budget(&p, now, 172_800_000, &[10_000], &authors).windows[0].complete);
}

#[test]
fn latency_settlement_window_includes_start_and_excludes_cut() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "start", now - 20_000, now - 10_000, None);
    merge(&mut p, "cut", now - 20_000, now, None);
    let measured = latency_budget(&p, now, 172_800_000, &[10_000], &[]);
    assert_eq!((measured.windows[0].total, measured.windows[0].bad), (1, 0));
    assert_eq!(measured.merged_latency_ms, [10_000]);
}

#[test]
fn latency_outside_cohort_conflicts_do_not_poison_selected_evidence_but_ambiguous_authors_do() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    merge(&mut p, "selected", now - 10_000, now - 1, Some("example-bot"));
    merge(&mut p, "outside", now - 10_000, now - 1, Some("another-author"));
    let mut conflicting = p.pull_requests[1].clone();
    conflicting.created_as_draft = None;
    p.pull_requests.push(conflicting);
    let authors = ["example-bot".into()];
    assert!(latency_budget(&p, now, 172_800_000, &[86_400_000], &authors).windows[0].complete);
    p.pull_requests[1].author_login = Some("example-bot".into());
    assert!(!latency_budget(&p, now, 172_800_000, &[86_400_000], &authors).windows[0].complete);
    p.pull_requests.reverse();
    assert!(!latency_budget(&p, now, 172_800_000, &[86_400_000], &authors).windows[0].complete);
}

#[test]
fn missing_pr_for_a_relevant_ready_deadline_is_unknown_only_inside_membership() {
    let now = at("2026-11-17T12:00:00Z");
    let mut p = projection(now);
    event(&mut p, "orphan", LifecycleKind::Ready, now - 7_000);
    assert!(!latency_budget(&p, now, 5_000, &[10_000], &[]).windows[0].complete);
    p.lifecycle[0].repository_id = "outside-roster".into();
    assert!(latency_budget(&p, now, 5_000, &[10_000], &[]).windows[0].complete);
}
