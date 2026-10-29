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
    since_unix_ms: u128,
    attributes: [KeyValue; 3],
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

fn summarize_rows(agents: Vec<String>, rows: &[Value], previous: &Summary, now: u128) -> Summary {
    let rows = rows.iter().filter_map(|row| row["id"].as_str().map(|id| (id, row)))
        .collect::<BTreeMap<_, _>>();
    let series = agents.into_iter().map(|agent| {
        let row = rows.get(agent.as_str());
        let delivery = row.map(|row| &row["delivery"]);
        let state = match delivery.and_then(|delivery| delivery["state"].as_str()) {
            Some("refused") => "refused",
            Some("current" | "outdated" | "legacy") => "live",
            // No confirmed delivery evidence is explicitly stale, even during startup or
            // while stopped. Never-bound and temporarily unpublished seats do not disappear.
            _ => "stale",
        };
        let reason = if state == "refused" {
            refusal_reason(delivery.and_then(|delivery| delivery["reason"].as_str()))
        } else { "" };
        let incarnation = row.and_then(|row| row["incarnation_id"].as_str());
        let since_unix_ms = previous.series.get(&agent)
            .filter(|previous| previous.state == state && previous.incarnation.as_deref() == incarnation)
            .map_or_else(|| delivery.and_then(|delivery| delivery["since_unix_ms"].as_u64())
                .map_or(now, u128::from), |previous| previous.since_unix_ms);
        let attributes = [KeyValue::new("st.agent", agent.clone()),
            KeyValue::new("st.delivery.state", state), KeyValue::new("st.delivery.reason", reason)];
        (agent, Series { state, incarnation: incarnation.map(str::to_owned), since_unix_ms, attributes })
    }).collect();
    Summary { series }
}

fn summarize(store: &Store, previous: &Summary) -> anyhow::Result<Summary> {
    let agents = super::mailbox_metrics::managed_agents_owned_by_local_host(store)?;
    let publication = store.agents_publication();
    Ok(summarize_rows(agents, publication.as_ref().map_or(&[], |publication| publication.rows()),
        previous, client_now_ms()))
}

pub fn register_metrics(meter: &Meter, summary: Arc<RwLock<Summary>>) {
    meter.f64_observable_gauge("st.delivery.state_age")
        .with_unit("s")
        .with_description("Seconds since this locally owned managed agent's delivery state began; no binding is stale")
        .with_callback(move |observer| {
            let now = client_now_ms();
            let summary = summary.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            for series in summary.series.values() {
                let age = Duration::from_millis(u64::try_from(now.saturating_sub(series.since_unix_ms))
                    .unwrap_or(u64::MAX)).as_secs_f64();
                observer.observe(age, &series.attributes);
            }
        }).build();
}

/// One off-scrape refresh at a time. Failed refreshes retain the complete previous membership
/// and clocks, rather than emitting a partial set or resetting an alert's age.
pub fn start(store: &Arc<Store>) {
    if !crate::otel::metrics_enabled() { return; }
    let store = Arc::downgrade(store);
    tokio::spawn(async move {
        let summary = Arc::new(RwLock::new(Summary::default()));
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
                        register_metrics(&opentelemetry::global::meter("st3"), Arc::clone(&summary));
                        registered = true;
                    }
                }
                Ok(Err(error)) => eprintln!("st3: WARN delivery metrics refresh failed: {error:#}"),
                Err(error) => eprintln!("st3: WARN delivery metrics refresh task failed: {error}"),
            }
            tokio::time::sleep(REFRESH_PAUSE.max(started.elapsed())).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let summary = summarize_rows(agents.clone(), &rows, &Summary::default(), 10_000);
        assert_eq!(summary.series.len(), 5);
        assert_eq!(summary.series[&agents[0]].state, "live");
        for index in [1, 3, 4] { assert_eq!(summary.series[&agents[index]].state, "stale"); }
        for index in [0, 1, 3, 4] {
            assert_eq!(summary.series[&agents[index]].attributes[2], KeyValue::new("st.delivery.reason", ""));
        }
        assert_eq!(summary.series[&agents[2]].attributes[2], KeyValue::new("st.delivery.reason", "mailbox-runtime-indeterminate"));
        let repeated = summarize_rows(agents.clone(), &rows, &summary, 20_000);
        for agent in &agents { assert_eq!(repeated.series[agent].since_unix_ms, summary.series[agent].since_unix_ms); }
        assert_eq!(refusal_reason(Some("delivery refused: arbitrary-new-code")), "other");
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
        let summary = summarize_rows(agents.clone(), &rows, &Summary::default(), client_now_ms().saturating_sub(700_000));
        register_metrics(&provider.meter("st3"), Arc::new(RwLock::new(summary)));
        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();
        let metric = finished.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics()).find(|metric| metric.name() == "st.delivery.state_age").unwrap();
        assert_eq!(metric.unit(), "s");
        let AggregatedMetrics::F64(MetricData::Gauge(gauge)) = metric.data() else { panic!("expected f64 gauge") };
        assert_eq!(gauge.data_points().count(), 4);
        for point in gauge.data_points() {
            assert!(point.value() >= 700.);
            assert_eq!(point.attributes().count(), 3);
            assert!(point.attributes().any(|attribute| agents.iter().any(|agent| *attribute == KeyValue::new("st.agent", agent.clone()))));
        }
        provider.shutdown().unwrap();
    }
}
