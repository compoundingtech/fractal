use super::*;
use crate::objective::Ratio;

const HOUR: u64 = 3_600_000;
const NOW: u64 = 40 * 24 * HOUR;
fn policy() -> BudgetPolicy {
    BudgetPolicy {
        allowance: 0.05, horizon_ms: 30 * 24 * HOUR, min_coverage: 0.9, min_samples: 1,
        fast: BurnWindow { burn_rate: 14.4, long_ms: HOUR, short_ms: 5 * 60_000 },
        slow: BurnWindow { burn_rate: 6.0, long_ms: 6 * HOUR, short_ms: 30 * 60_000 },
    }
}
fn gauge() -> Gauge {
    Gauge { subject_prefix: "custom/example/source/".into(), event: "custom.example.reading".into(), at: "at".into(), field: "value".into(), threshold: Threshold::Above(10.0), labels: Vec::new(), excludes: Vec::new(), only: Vec::new(), max_age_ms: HOUR, cadence_ms: HOUR }
}
fn tracked(gauge: Option<Gauge>) -> Tracked {
    let indicator = gauge.map_or_else(|| Indicator::Ratio(Box::new(Ratio {
        subject_prefix: "custom/example/event/".into(), event: "custom.example.event".into(), at: "at".into(), field: "value".into(), bad_above: 10.0, labels: Vec::new(), excludes: Vec::new(), only: Vec::new(),
    })), |gauge| Indicator::Gauge(Box::new(gauge)));
    Tracked {
        revision: "revision".into(), claim_id: "declaration".into(),
        objective: Ok(Objective { name: "example/quality".into(), about: "Example quality".into(), indicator, window: Some(Window { ms: policy().horizon_ms, min_samples: 1 }), budget: Some(policy()), targets: Vec::new(), on_breach: None }),
        cursor: 0, samples: BTreeMap::new(), events: BTreeMap::new(), numeric: Observations::default(), seen: false, labels: BTreeMap::new(), unreadable: 0, ids: BTreeSet::new(),
    }
}
fn point(tracked: &mut Tracked, source: &str, at: u64, value: Option<f64>) {
    tracked.numeric.sources.entry(source.into()).or_default().insert(at, Observation { id: format!("{source}-{at}"), value, fields: BTreeMap::new(), eligible: true });
}
fn row(tracked: &Tracked, gauge: Option<&Gauge>) -> Value {
    tracked.budget_rows("objective/example/quality", gauge, NOW, "No response.").remove(0)
}

#[test]
fn gauge_holds_only_to_max_age_and_invalid_reading_breaks_coverage() {
    let gauge = gauge();
    let mut tracked = tracked(Some(gauge.clone()));
    point(&mut tracked, "source", NOW - 3 * HOUR, Some(20.0));
    point(&mut tracked, "source", NOW - 3 * HOUR + HOUR / 2, None);
    let measured = tracked.numeric.measure(NOW - 4 * HOUR, NOW, gauge.threshold, Some(&gauge), None);
    assert_eq!(measured.total, (HOUR / 2) as f64);
    assert_eq!(measured.bad, measured.total);
    assert_eq!(measured.coverage(), 0.125);
    assert_eq!(row(&tracked, Some(&gauge))["status"], "no-data");
    assert!(row(&tracked, Some(&gauge))["remaining_percent"].is_null());
}

#[test]
fn hourly_gauge_uses_real_paired_windows_without_inventing_thirty_day_coverage() {
    let gauge = gauge();
    let mut tracked = tracked(Some(gauge.clone()));
    for hour in 1..=6 { point(&mut tracked, "source", NOW - hour * HOUR, Some(20.0)); }
    let row = row(&tracked, Some(&gauge));
    assert_eq!(row["status"], "burning");
    assert!(row["remaining_percent"].is_null());
    assert!(row["explain"].as_str().unwrap().contains("fast paired 2h+1h"));
    assert!(row["explain"].as_str().unwrap().contains("slow paired 6h+1h"));
}

#[test]
fn ratio_short_only_spike_stays_met_with_long_window_diagnostics() {
    let mut tracked = tracked(None);
    for event in 0..1000 {
        point(&mut tracked, &format!("good-{event}"), NOW - 40 * 60_000, Some(1.0));
    }
    point(&mut tracked, "spike", NOW - 60_000, Some(20.0));
    let row = row(&tracked, None);
    assert_eq!(row["status"], "met");
    assert!(row["explain"].as_str().unwrap().contains("short 20.00x"));
    assert!(row["remaining_percent"].as_f64().unwrap() > 98.0);
}

#[test]
fn ratio_pair_burns_before_horizon_budget_is_exhausted() {
    let mut tracked = tracked(None);
    for event in 0..1000 { point(&mut tracked, &format!("historic-{event}"), NOW - 24 * HOUR, Some(1.0)); }
    point(&mut tracked, "recent", NOW - 60_000, Some(20.0));
    assert_eq!(row(&tracked, None)["status"], "burning");
}

#[test]
fn exhausted_budget_and_no_qualifying_events_are_distinct_from_missing_data() {
    let mut tracked = tracked(None);
    assert_eq!(row(&tracked, None)["status"], "no-data");
    point(&mut tracked, "bad", NOW - HOUR, Some(20.0));
    assert_eq!(row(&tracked, None)["status"], "exhausted");
    tracked.numeric.sources.get_mut("bad").unwrap().values_mut().next().unwrap().eligible = false;
    assert_eq!(row(&tracked, None)["status"], "no-qualifying-events");
}

#[test]
fn coverage_counts_silent_sources_as_unknown_not_healthy() {
    let gauge = gauge();
    let mut tracked = tracked(Some(gauge.clone()));
    point(&mut tracked, "live", NOW - HOUR, Some(1.0));
    point(&mut tracked, "silent", NOW - 40 * 24 * HOUR, Some(1.0));
    tracked.numeric.prune(NOW - policy().horizon_ms - gauge.max_age_ms, true);
    let measured = tracked.numeric.measure(NOW - HOUR, NOW, gauge.threshold, Some(&gauge), None);
    assert_eq!(measured.coverage(), 0.5);
}

#[test]
fn lower_bound_gauge_counts_only_time_below_threshold_as_bad() {
    let mut gauge = gauge();
    gauge.threshold = Threshold::Below(10.0);
    let mut tracked = tracked(Some(gauge.clone()));
    point(&mut tracked, "source", NOW - 2 * HOUR, Some(5.0));
    point(&mut tracked, "source", NOW - HOUR, Some(10.0));
    let measured = tracked.numeric.measure(NOW - 2 * HOUR, NOW, gauge.threshold, Some(&gauge), None);
    assert_eq!(measured.bad, HOUR as f64);
    assert_eq!(measured.total, (2 * HOUR) as f64);
}

#[test]
fn unknown_ratio_observation_prevents_healthy_or_exhausted_denominator_claims() {
    let mut tracked = tracked(None);
    point(&mut tracked, "good", NOW - HOUR, Some(1.0));
    point(&mut tracked, "unknown", NOW - HOUR, None);
    let row = row(&tracked, None);
    assert_eq!(row["status"], "no-data");
    assert!(row["remaining_percent"].is_null());
    assert!(row["explain"].as_str().unwrap().contains("1 unknown observations"));
}

#[test]
fn ratio_retention_drops_expired_source_identities() {
    let mut tracked = tracked(None);
    point(&mut tracked, "expired", NOW - 40 * 24 * HOUR, Some(1.0));
    tracked.numeric.prune(NOW - policy().horizon_ms, false);
    assert!(tracked.numeric.sources.is_empty());
}
