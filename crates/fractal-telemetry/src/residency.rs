//! Inert residency instruments. The owner enables these only with the stop/wake cutover.
//! Callbacks read a precomputed summary; they never query a store or the operating system.
use std::sync::{Arc, Mutex};
use opentelemetry::{KeyValue, metrics::{Counter, Meter, ObservableGauge}};

macro_rules! vocabulary {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }
    };
}
vocabulary!(SuspendResult { Suspended => "suspended", Refused => "refused", Cancelled => "cancelled" });
vocabulary!(WakeSource { Person => "person", Agent => "agent", Daemon => "daemon", Github => "github" });
vocabulary!(WakeOutcome { Turn => "turn", NoTurn => "no-turn" });
vocabulary!(UnroutedKind { Attach => "attach", ExplicitResume => "explicit-resume" });

/// Unknown driver/policy codes are one bounded bucket, never dynamic attributes.
pub const REFUSAL_CODES: &[&str] = &[
    "pending-ask", "harness-unobserved", "background-jobs-unreported", "harness-busy",
    "quiescence-unreported", "claimed-work", "subagent-running", "native-session-unbound",
    "workspace-not-git", "workspace-unborn", "workspace-not-root", "workspace-submodule-unsupported",
    "other",
];
pub const RESUME_FAILURE_CODES: &[&str] = &[
    "transcript-missing", "native-session-mismatch", "snapshot-in-place", "restore-failed",
    "verify-timeout", "stop-timeout", "other",
];
fn bounded<'a>(code: &str, vocabulary: &'a [&'a str]) -> &'a str {
    vocabulary.iter().copied().find(|known| *known == code).unwrap_or("other")
}

/// Three results + refusal codes + eight wake pairs + failure codes + one short-sleep
/// + two unrouted kinds + six gauge series. No seat/session/message labels.
pub const SERIES_ALLOCATION: usize = 3 + REFUSAL_CODES.len() + 8 + RESUME_FAILURE_CODES.len() + 1 + 2 + 6;

/// Missing coverage is absent, not a zero observation. Memory is a paired delta,
/// not summed RSS; negative reclamation is a valid measurement.
#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    pub idle_suspended: Option<u64>,
    pub manual_suspended: Option<u64>,
    pub oldest_wake_seconds: Option<f64>,
    pub oldest_silent_seconds: Option<f64>,
    pub host_available_delta_bytes: Option<i64>,
    pub private_reclaimed_bytes: Option<i64>,
}

pub struct Instruments {
    suspends: Counter<u64>,
    refusals: Counter<u64>,
    wakes: Counter<u64>,
    failures: Counter<u64>,
    short_sleeps: Counter<u64>,
    unrouted: Counter<u64>,
    summary: Arc<Mutex<Summary>>,
    _suspended: ObservableGauge<u64>,
    _pending: ObservableGauge<f64>,
    _reclaimed: ObservableGauge<i64>,
}
impl Instruments {
    /// Explicit construction is the activation boundary. No daemon calls this yet.
    pub fn new(meter: &Meter) -> Self {
        let summary = Arc::new(Mutex::new(Summary::default()));
        let suspended = Arc::clone(&summary);
        let pending = Arc::clone(&summary);
        let reclaimed = Arc::clone(&summary);
        Self {
            suspends: meter.u64_counter("st.residency.suspends").build(),
            refusals: meter.u64_counter("st.residency.refusals").build(),
            wakes: meter.u64_counter("st.residency.wakes").build(),
            failures: meter.u64_counter("st.residency.resume.failures").build(),
            short_sleeps: meter.u64_counter("st.residency.short_sleeps").build(),
            unrouted: meter.u64_counter("st.residency.unrouted").build(),
            summary,
            _suspended: meter.u64_observable_gauge("st.residency.suspended")
                .with_callback(move |observer| {
                    let cut = *suspended.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for (cause, value) in [("idle", cut.idle_suspended), ("manual", cut.manual_suspended)] {
                        if let Some(value) = value { observer.observe(value, &[KeyValue::new("cause", cause)]); }
                    }
                }).build(),
            _pending: meter.f64_observable_gauge("st.residency.pending.oldest_age").with_unit("s")
                .with_callback(move |observer| {
                    let cut = *pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for (class, value) in [("wake", cut.oldest_wake_seconds), ("silent", cut.oldest_silent_seconds)] {
                        if let Some(value) = value { observer.observe(value, &[KeyValue::new("class", class)]); }
                    }
                }).build(),
            _reclaimed: meter.i64_observable_gauge("st.residency.reclaimed").with_unit("By")
                .with_callback(move |observer| {
                    let cut = *reclaimed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for (measure, value) in [("host-available", cut.host_available_delta_bytes), ("private", cut.private_reclaimed_bytes)] {
                        if let Some(value) = value { observer.observe(value, &[KeyValue::new("measure", measure)]); }
                    }
                }).build(),
        }
    }
    pub fn publish(&self, summary: Summary) {
        *self.summary.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = summary;
    }
    pub fn suspend(&self, result: SuspendResult) {
        self.suspends.add(1, &[KeyValue::new("result", result.label())]);
    }
    pub fn refusal(&self, reason: &str) {
        self.refusals.add(1, &[KeyValue::new("reason", bounded(reason, REFUSAL_CODES))]);
    }
    /// Called once after the ten-minute outcome is durable, not at demand arrival.
    pub fn wake(&self, source: WakeSource, outcome: WakeOutcome) {
        self.wakes.add(1, &[KeyValue::new("source", source.label()), KeyValue::new("outcome", outcome.label())]);
    }
    pub fn resume_failure(&self, code: &str) {
        self.failures.add(1, &[KeyValue::new("code", bounded(code, RESUME_FAILURE_CODES))]);
    }
    pub fn short_sleep(&self) { self.short_sleeps.add(1, &[]); }
    pub fn unrouted(&self, kind: UnroutedKind) {
        self.unrouted.add(1, &[KeyValue::new("kind", kind.label())]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_are_closed_and_unknown_codes_share_other() {
        assert_eq!(SuspendResult::ALL.len(), 3);
        assert_eq!(WakeSource::ALL.len() * WakeOutcome::ALL.len(), 8);
        assert_eq!(UnroutedKind::ALL.len(), 2);
        for codes in [REFUSAL_CODES, RESUME_FAILURE_CODES] {
            for &code in codes { assert_eq!(bounded(code, codes), code); }
            for unknown in ["", "agent/example", "new-driver-failure"] {
                assert_eq!(bounded(unknown, codes), "other");
            }
            let mut unique = codes.to_vec(); unique.sort_unstable(); unique.dedup();
            assert_eq!(unique.len(), codes.len());
        }
        assert_eq!(SERIES_ALLOCATION, 40);
    }
    #[test]
    fn missing_gauge_coverage_does_not_export_zero() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let instruments = Instruments::new(&provider.meter("residency-test"));
        instruments.publish(Summary { idle_suspended: Some(0), ..Summary::default() });
        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();
        let metrics: Vec<_> = finished.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics()).collect();
        let suspended = metrics.iter().find(|metric| metric.name() == "st.residency.suspended").unwrap();
        let AggregatedMetrics::U64(MetricData::Gauge(gauge)) = suspended.data() else {
            panic!("suspended is an unsigned gauge");
        };
        assert_eq!(gauge.data_points().count(), 1, "manual coverage is absent, idle zero is real");
        for metric in metrics.iter().filter(|metric| matches!(metric.name(),
            "st.residency.pending.oldest_age" | "st.residency.reclaimed")) {
            let points = match metric.data() {
                AggregatedMetrics::F64(MetricData::Gauge(gauge)) => gauge.data_points().count(),
                AggregatedMetrics::I64(MetricData::Gauge(gauge)) => gauge.data_points().count(),
                _ => panic!("age and reclaimed instruments are gauges"),
            };
            assert_eq!(points, 0, "missing coverage must not produce a zero point");
        }
        provider.shutdown().unwrap();
    }
}
