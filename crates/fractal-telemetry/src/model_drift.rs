//! Model drift gauges follow the reconciler's complete locally owned roster cut.
//! Scrapes read precomputed evidence only; freshness is reevaluated without store reads.
use std::sync::{Arc, OnceLock, RwLock};

use fractal_drivers::harness_state::HARNESS_OBSERVATION_FRESHNESS;
use opentelemetry::{KeyValue, metrics::{Meter, ObservableGauge}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { AtDeclared, Drifted, Unknown, Stale, Uncomparable }
impl State {
    const ALL: [Self; 5] = [Self::AtDeclared, Self::Drifted, Self::Unknown, Self::Stale, Self::Uncomparable];
    fn label(self) -> &'static str {
        match self {
            Self::AtDeclared => "at-declared", Self::Drifted => "drifted",
            Self::Unknown => "unknown", Self::Stale => "stale", Self::Uncomparable => "uncomparable",
        }
    }
}

pub struct Reading {
    pub agent: String,
    pub state: State,
    pub observed_at_ms: Option<u128>,
    pub drift_since_ms: Option<u128>,
}
struct Series {
    state: State,
    observed_at_ms: Option<u128>,
    drift_since_ms: Option<u128>,
    agent_attributes: [KeyValue; 1],
    state_attributes: [[KeyValue; 2]; 5],
}
impl Series {
    fn new(reading: Reading) -> Self {
        let agent = KeyValue::new("st.agent", reading.agent);
        Self {
            agent_attributes: [agent.clone()],
            state_attributes: State::ALL.map(|state| [agent.clone(), KeyValue::new("st.model.state", state.label())]),
            state: reading.state,
            observed_at_ms: reading.observed_at_ms,
            drift_since_ms: reading.drift_since_ms,
        }
    }
    fn state(&self, now: u128) -> State {
        if self.state == State::Unknown { return State::Unknown; }
        match self.observed_at_ms {
            Some(at) if at <= now && now - at <= HARNESS_OBSERVATION_FRESHNESS.as_millis() => self.state,
            _ => State::Stale,
        }
    }
    fn age_seconds(&self, now: u128) -> Option<f64> {
        if self.state(now) != State::Drifted { return None; }
        self.drift_since_ms.filter(|since| *since <= now)
            .map(|since| (now - since) as f64 / 1000.)
    }
}

pub struct Instruments {
    series: Arc<RwLock<Vec<Series>>>,
    _state: ObservableGauge<u64>,
    _age: ObservableGauge<f64>,
}
impl Instruments {
    pub fn new(meter: &Meter) -> Self {
        let series = Arc::new(RwLock::new(Vec::<Series>::new()));
        let states = Arc::clone(&series);
        let ages = Arc::clone(&series);
        Self {
            series,
            _state: meter.u64_observable_gauge("st.agent.model.state")
                .with_description("One-hot model evidence state for every locally owned declared agent; unknown is never healthy")
                .with_callback(move |observer| {
                    let now = smallclaims::store::now_ms();
                    let cut = states.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for series in cut.iter() {
                        let current = series.state(now);
                        for (state, attributes) in State::ALL.iter().zip(&series.state_attributes) {
                            observer.observe(u64::from(*state == current), attributes);
                        }
                    }
                }).build(),
            _age: meter.f64_observable_gauge("st.agent.model.drift.age_seconds")
                .with_unit("s")
                .with_description("Age of established fresh model drift; absent for unknown, stale, uncomparable or unestablished drift")
                .with_callback(move |observer| {
                    let now = smallclaims::store::now_ms();
                    let cut = ages.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for series in cut.iter() {
                        if let Some(age) = series.age_seconds(now) {
                            observer.observe(age, &series.agent_attributes);
                        }
                    }
                }).build(),
        }
    }
    pub fn replace(&self, readings: Vec<Reading>) {
        *self.series.write().unwrap_or_else(std::sync::PoisonError::into_inner) =
            readings.into_iter().map(Series::new).collect();
    }
}

#[derive(Default)]
pub struct Metrics(OnceLock<Instruments>);

/// Publish only a complete successful reconciliation cut. Host identity comes from the
/// existing daemon resource, and agent labels are bounded to its declared roster.
pub fn replace(metrics: &Metrics, readings: Vec<Reading>) {
    if crate::otel::metrics_enabled() {
        metrics.0.get_or_init(|| Instruments::new(&opentelemetry::global::meter("st3"))).replace(readings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_states_and_missing_onset_preserve_unknown_coverage() {
        let now = 1_000_000;
        for state in State::ALL {
            let mut series = Series::new(Reading {
                agent: "agent/example.worker".into(), state,
                observed_at_ms: Some(now), drift_since_ms: Some(now - 700_000),
            });
            assert_eq!(series.state(now), state);
            assert_eq!(series.age_seconds(now), (state == State::Drifted).then_some(700.));
            series.drift_since_ms = None;
            assert_eq!(series.age_seconds(now), None);
        }
    }

    #[test]
    fn stale_future_and_missing_timestamps_never_export_health_or_age() {
        let now = 1_000_000;
        for at in [None, Some(now + 1), Some(now - HARNESS_OBSERVATION_FRESHNESS.as_millis() - 1)] {
            for state in [State::AtDeclared, State::Drifted, State::Uncomparable] {
                let series = Series::new(Reading {
                    agent: "agent/example.worker".into(), state,
                    observed_at_ms: at, drift_since_ms: Some(1),
                });
                assert_eq!(series.state(now), State::Stale);
                assert_eq!(series.age_seconds(now), None);
            }
        }
    }

    #[test]
    fn exported_gauges_replace_roster_and_omit_unknown_age() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let metrics = Instruments::new(&provider.meter("model-drift-test"));
        let now = smallclaims::store::now_ms();
        metrics.replace(State::ALL.into_iter().enumerate().map(|(index, state)| Reading {
            agent: format!("agent/example.worker-{index}"), state,
            observed_at_ms: Some(now), drift_since_ms: Some(now - 700_000),
        }).collect());
        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();
        let exported: Vec<_> = finished.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics()).collect();
        let state = exported.iter().find(|metric| metric.name() == "st.agent.model.state").unwrap();
        let AggregatedMetrics::U64(MetricData::Gauge(states)) = state.data() else { panic!("state is a gauge"); };
        assert_eq!(states.data_points().count(), 25);
        assert_eq!(states.data_points().map(|point| point.value()).sum::<u64>(), 5);
        let age = exported.iter().find(|metric| metric.name() == "st.agent.model.drift.age_seconds").unwrap();
        let AggregatedMetrics::F64(MetricData::Gauge(ages)) = age.data() else { panic!("age is a gauge"); };
        assert_eq!(ages.data_points().count(), 1);
        metrics.replace(Vec::new());
        assert!(metrics.series.read().unwrap().is_empty());
        provider.shutdown().unwrap();
    }
}
