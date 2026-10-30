//! Delivery health for every declared locally owned agent, including never-bound seats.
//! Membership shares mailbox_metrics' declaration-based ownership selection. Collection only
//! reads this cache: no graph, store, process or filesystem work runs on the SDK reader thread.

use super::{Store, client_now_ms};
use crate::EvidenceStore;
use opentelemetry::{KeyValue, metrics::Meter};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const REFRESH_PAUSE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Summary {
    series: BTreeMap<String, Series>,
}

struct Series {
    state: &'static str,
    incarnation: Option<String>,
    clock: StateClock,
    attributes: [KeyValue; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StateClock {
    observed_at: Instant,
    initial_age: Duration,
}

impl StateClock {
    fn age_at(self, now: Instant) -> Duration {
        self.initial_age.saturating_add(now.saturating_duration_since(self.observed_at))
    }
}

/// Fixed metric vocabulary. Arbitrary API codes remain visible in the API/log, but never
/// become unbounded labels. Live and stale always use the empty reason, including unbound.
fn refusal_reason(reason: Option<&str>) -> &'static str {
    match reason.and_then(|reason| reason.strip_prefix("delivery refused: ")) {
        Some("mailbox-runtime-indeterminate") => "mailbox-runtime-indeterminate",
        Some("stale-mailbox-session") => "stale-mailbox-session",
        Some("mailbox-binding-conflict") => "mailbox-binding-conflict",
        Some("mailbox-peer-mismatch") => "mailbox-peer-mismatch",
        Some("mailbox-not-authorized") => "mailbox-not-authorized",
        Some("recipient-retired") => "recipient-retired",
        _ => "other",
    }
}

fn summarize_rows(agents: Vec<String>, rows: &[Value], previous: &Summary, now: u128, observed_at: Instant) -> Summary {
    let rows = rows.iter().filter_map(|row| row["id"].as_str().map(|id| (id, row)))
        .collect::<BTreeMap<_, _>>();
    let series = agents.into_iter().map(|agent| {
        let row = rows.get(agent.as_str());
        let delivery = row.map(|row| &row["delivery"]);
        let state = match delivery.and_then(|delivery| delivery["state"].as_str()) {
            Some("refused") => "refused",
            Some("current" | "outdated" | "legacy") => "live",
            // No confirmed delivery evidence is stale, even during startup or suspension.
            // Never-bound and temporarily unpublished declared agents do not disappear.
            _ => "stale",
        };
        let reason = if state == "refused" {
            refusal_reason(delivery.and_then(|delivery| delivery["reason"].as_str()))
        } else { "" };
        let incarnation = row.and_then(|row| row["incarnation_id"].as_str());
        // Seed once from the API's wall timestamp, then advance exclusively on monotonic time.
        // Preserve the clock across unchanged states even when the wall clock is corrected.
        let clock = previous.series.get(&agent)
            .filter(|previous| previous.state == state && previous.incarnation.as_deref() == incarnation)
            .map_or_else(|| {
                let since = delivery.and_then(|delivery| delivery["since_unix_ms"].as_u64())
                    .map_or(now, u128::from);
                StateClock { observed_at, initial_age: Duration::from_millis(
                    u64::try_from(now.saturating_sub(since)).unwrap_or(u64::MAX)) }
            }, |previous| previous.clock);
        let attributes = [KeyValue::new("st.agent", agent.clone()),
            KeyValue::new("st.delivery.state", state), KeyValue::new("st.delivery.reason", reason),
            KeyValue::new("st.agent.desired", "running")];
        (agent, Series { state, incarnation: incarnation.map(str::to_owned), clock, attributes })
    }).collect();
    Summary { series }
}

fn summarize(store: &Store, previous: &Summary) -> anyhow::Result<Summary> {
    let agents = super::mailbox_metrics::managed_agents_owned_by_local_host(store)?;
    let publication = store.agents_publication();
    let mut summary = summarize_rows(agents, publication.as_ref().map_or(&[], |publication| publication.rows()),
        previous, client_now_ms(), Instant::now());
    for (agent, series) in &mut summary.series {
        // Use the reconciler's authoritative hold, not harness/runtime liveness.
        // Membership already excludes stopped declarations; no unknown state is needed.
        let desired = if crate::suspension::current(store, agent)?
            .is_some_and(|suspension| suspension.holds_seat())
        { "suspended" } else { "running" };
        series.attributes[3] = KeyValue::new("st.agent.desired", desired);
    }
    Ok(summary)
}

pub fn register_metrics(meter: &Meter, summary: Arc<RwLock<Summary>>) {
    meter.f64_observable_gauge("st.delivery.state_age")
        .with_unit("s")
        .with_description("Seconds since this locally owned managed agent's delivery state began; no binding is stale")
        .with_callback(move |observer| {
            let now = Instant::now();
            let summary = summary.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for series in summary.series.values() {
                let age = series.clock.age_at(now).as_secs_f64();
                observer.observe(age, &series.attributes);
            }
        }).build();
}

/// One off-scrape refresh at a time. Failed refreshes retain the complete previous membership
/// and clocks, rather than emitting a partial set or resetting an alert's age.
pub fn start(store: &Arc<Store>) {
    if !crate::otel::metrics_enabled() { return; }
    let _worker = spawn_worker(store, Arc::new(RwLock::new(Summary::default())),
        opentelemetry::global::meter("st3"), REFRESH_PAUSE);
}

pub(super) fn spawn_worker(
    store: &Arc<Store>,
    summary: Arc<RwLock<Summary>>,
    meter: Meter,
    refresh_pause: Duration,
) -> tokio::task::JoinHandle<()> {
    // A cached read alone is not demand: without this receiver the roster owner can stop
    // folding when the last UI disconnects, freezing heartbeat/refusal/recovery evidence.
    let publication_demand = store.subscribe_agents_publications();
    let store = Arc::downgrade(store);
    tokio::spawn(async move {
        let _publication_demand = publication_demand;
        let mut registered = false;
        loop {
            let Some(store) = store.upgrade() else { return; };
            let started = Instant::now();
            let previous = Arc::clone(&summary);
            let result = tokio::task::spawn_blocking(move || {
                crate::task_cpu::profiled_blocking("delivery_metrics/refresh", || {
                    let previous = previous.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                    summarize(&store, &previous)
                })
            }).await;
            match result {
                Ok(Ok(next)) => {
                    *summary.write().unwrap_or_else(std::sync::PoisonError::into_inner) = next;
                    if !registered {
                        register_metrics(&meter, Arc::clone(&summary));
                        registered = true;
                    }
                }
                Ok(Err(error)) => eprintln!("st3: WARN delivery metrics refresh failed: {error:#}"),
                Err(error) => eprintln!("st3: WARN delivery metrics refresh task failed: {error}"),
            }
            tokio::time::sleep(refresh_pause.max(started.elapsed())).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desired_lifecycle_uses_authoritative_suspension_and_omits_stopped_agents() {
        use crate::SeatsStoreCore;
        const AGENT: &str = "agent/example/desired-metric";
        let store = Store::open_memory("node").unwrap();
        let source = "version 2\nagent \"example/desired-metric\" { host \"node\"; workspace \"/tmp\"; harness \"omp\" {} }\n";
        let declare = |source: &str, key: &str| {
            store.apply_internal(&crate::graph::parse_intent(source, "node").unwrap(), key).unwrap();
        };
        let append = |kind: &str, fields: Value, evidence: Vec<String>, key: Option<String>| {
            store.append_claim(&crate::model::ClaimInput {
                subject: AGENT.into(), kind: kind.into(), actor: Some("person/operator".into()),
                fields: serde_json::from_value(fields).unwrap(), evidence,
                expected_subject: None, idempotency_key: key,
            }).unwrap()
        };
        let expect = |desired: &str| {
            let summary = summarize(&store, &Summary::default()).unwrap();
            let series = &summary.series[AGENT];
            assert_eq!(series.attributes[3], KeyValue::new("st.agent.desired", desired.to_owned()));
            // Neither a publication nor a runtime heartbeat exists: liveness cannot select intent.
            assert_eq!(series.state, "stale");
        };
        declare(source, "desired-initial");
        expect("running");
        let token = store.selected_desired_token(AGENT).unwrap().unwrap();
        let suspend = append("runtime.action.requested",
            json!({"action":"suspend","incarnation_id":"one"}), vec![token.clone()], None);
        expect("suspended"); // quiescing is already an authoritative hold
        append("runtime.action.succeeded", json!({"action":"suspend","operation_status":"suspended"}),
            vec![suspend.id.clone()], Some(crate::suspension::suspend_completed_key(&suspend.id)));
        expect("suspended");
        let resume = append("runtime.action.requested", json!({"action":"resume","host":"node"}),
            vec![token, suspend.id.clone()], None);
        expect("suspended"); // transfer/restore/verify holds remain until resume completes
        append("runtime.action.succeeded", json!({"action":"resume","operation_status":"resumed"}),
            vec![resume.id.clone()], Some(crate::suspension::resume_completed_key(&resume.id)));
        expect("running");
        declare("version 2\nstop \"agent/example/desired-metric\"\n", "desired-stop");
        assert!(summarize(&store, &Summary::default()).unwrap().series.is_empty(),
            "stopped declarations are outside the unchanged shared metric membership");
        declare(source, "desired-start");
        expect("running");
    }

    #[test]
    fn every_owned_agent_has_a_series_and_only_refused_has_a_bounded_reason() {
        let agents = ["live", "stale", "refused", "stopped", "never-bound"].map(|name| format!("agent/example/{name}")).to_vec();
        let rows = vec![
            json!({"id":agents[0], "delivery":{"state":"current", "reason":"not a metric reason", "since_unix_ms":1000}}),
            json!({"id":agents[1], "delivery":{"state":"stale", "reason":"not a metric reason", "since_unix_ms":2000}}),
            json!({"id":agents[2], "delivery":{"state":"refused", "reason":"delivery refused: mailbox-runtime-indeterminate", "since_unix_ms":3000}}),
            json!({"id":agents[3], "state":"stopped"}),
            json!({"id":"agent/remote/excluded", "delivery":{"state":"current"}}),
        ];
        let summary = summarize_rows(agents.clone(), &rows, &Summary::default(), 10_000, Instant::now());
        assert_eq!(summary.series.len(), 5);
        assert_eq!(summary.series[&agents[0]].state, "live");
        for index in [1, 3, 4] { assert_eq!(summary.series[&agents[index]].state, "stale"); }
        for index in [0, 1, 3, 4] {
            assert_eq!(summary.series[&agents[index]].attributes[2], KeyValue::new("st.delivery.reason", ""));
        }
        assert_eq!(summary.series[&agents[2]].attributes[2], KeyValue::new("st.delivery.reason", "mailbox-runtime-indeterminate"));
        let repeated = summarize_rows(agents.clone(), &rows, &summary, 20_000, Instant::now());
        for agent in &agents { assert_eq!(repeated.series[agent].clock, summary.series[agent].clock); }
        assert_eq!(refusal_reason(Some("delivery refused: arbitrary-new-code")), "other");
    }

    #[test]
    fn state_age_ignores_wall_clock_corrections_and_preserves_unchanged_refreshes() {
        let agent = "agent/example/monotonic";
        let rows = vec![json!({"id":agent, "incarnation_id":"one",
            "delivery":{"state":"stale", "since_unix_ms":900_000}})];
        let mono = Instant::now();
        let initial = summarize_rows(vec![agent.into()], &rows, &Summary::default(), 1_000_000, mono);
        assert_eq!(initial.series[agent].clock.age_at(mono).as_secs(), 100);
        let backward = summarize_rows(vec![agent.into()], &rows, &initial,
            1, mono + Duration::from_secs(10));
        assert_eq!(backward.series[agent].clock.age_at(mono + Duration::from_secs(10)).as_secs(), 110);
        let forward = summarize_rows(vec![agent.into()], &rows, &backward,
            99_000_000, mono + Duration::from_secs(20));
        assert_eq!(forward.series[agent].clock.age_at(mono + Duration::from_secs(20)).as_secs(), 120);
        assert_eq!(forward.series[agent].clock, initial.series[agent].clock);
    }

    #[test]
    fn observable_gauge_exports_every_state_and_full_agent_identity() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let agents = ["live", "stale", "refused", "never-bound"].map(|name| format!("agent/example/{name}")).to_vec();
        let rows = vec![json!({"id":agents[0],"delivery":{"state":"current"}}),
            json!({"id":agents[1],"delivery":{"state":"stale"}}),
            json!({"id":agents[2],"delivery":{"state":"refused","reason":"delivery refused: mailbox-runtime-indeterminate"}})];
        let now = client_now_ms();
        let mut rows = rows;
        for row in &mut rows { row["delivery"]["since_unix_ms"] = json!(now.saturating_sub(700_000)); }
        let summary = summarize_rows(agents.clone(), &rows, &Summary::default(), now, Instant::now());
        register_metrics(&provider.meter("st3"), Arc::new(RwLock::new(summary)));
        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();
        let metric = finished.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics()).find(|metric| metric.name() == "st.delivery.state_age").unwrap();
        assert_eq!(metric.unit(), "s");
        let AggregatedMetrics::F64(MetricData::Gauge(gauge)) = metric.data() else { panic!("expected f64 gauge") };
        assert_eq!(gauge.data_points().count(), 4);
        for point in gauge.data_points() {
            let never_bound = point.attributes().any(|attribute|
                *attribute == KeyValue::new("st.agent", agents[3].clone()));
            if never_bound {
                assert!(point.value() < 60., "an unbound seat starts its monotonic clock at first observation");
            } else {
                assert!(point.value() >= 700., "an API timestamp seeds the initial age");
            }
            assert_eq!(point.attributes().count(), 4);
            assert!(point.attributes().any(|attribute| agents.iter().any(|agent| *attribute == KeyValue::new("st.agent", agent.clone()))));
        }
        provider.shutdown().unwrap();
    }
}
