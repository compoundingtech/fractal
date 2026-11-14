//! Host-authoritative resource lane observations, not a second admission ledger.
//!
//! Construct once after telemetry initialization. Publish reconciled snapshots before admitting
//! work after restart. Phase guards measure monotonic time within this daemon incarnation only;
//! callers retain durable wall-clock events and reconcile restart uncertainty themselves.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use opentelemetry::{KeyValue, metrics::{Histogram, Meter}, trace::{Span as _, TraceContextExt as _, Tracer as _}};


/// Execution identity belongs on traces/logs, never metric dimensions.
#[derive(Clone, Debug)]
pub struct LaneIdentity {
    pub lane: String,
    pub membership: String,
    pub run: String,
    pub step: String,
    pub attempt: u64,
    pub generation: String,
    pub invocation: Option<String>,
}

impl LaneIdentity {
    fn attributes(&self) -> Vec<KeyValue> {
        let mut attributes = vec![
            KeyValue::new("st.resource.lane", self.lane.clone()),
            KeyValue::new("st.resource.membership", self.membership.clone()),
            KeyValue::new("st.run", self.run.clone()),
            KeyValue::new("st.step", self.step.clone()),
            KeyValue::new("st.attempt", i64::try_from(self.attempt).unwrap_or(i64::MAX)),
            KeyValue::new("st.resource.generation", self.generation.clone()),
        ];
        if let Some(invocation) = &self.invocation {
            attributes.push(KeyValue::new("st.invocation.id", invocation.clone()));
        }
        attributes
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LanePhaseKind { Admission, Execution, Release }

impl LanePhaseKind {
    fn name(self) -> &'static str {
        match self { Self::Admission => "st.resource.admission", Self::Execution => "st.resource.execution", Self::Release => "st.resource.release" }
    }
    fn label(self) -> &'static str {
        match self { Self::Admission => "admission", Self::Execution => "execution", Self::Release => "release" }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LaneOutcome { Succeeded, Cancelled, Expired, Refused, Failed, Uncertain }

impl LaneOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded", Self::Cancelled => "cancelled", Self::Expired => "expired",
            Self::Refused => "refused", Self::Failed => "failed", Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Clone, Copy)]
struct Snapshot { occupancy: u64, queued: u64, holds: u64 }

struct Instruments {
    snapshots: Arc<RwLock<BTreeMap<String, Snapshot>>>,
    wait: Histogram<f64>,
    execution: Histogram<f64>,
    release: Histogram<f64>,
    hold: Histogram<f64>,
}

impl Instruments {
    fn new(meter: &Meter) -> Self {
        let snapshots = Arc::new(RwLock::new(BTreeMap::<String, Snapshot>::new()));
        for (name, description, select) in [
            ("st.resource.occupancy", "Admitted memberships, including unretired generations", (|s: &Snapshot| s.occupancy) as fn(&Snapshot) -> u64),
            ("st.resource.queue", "Requests awaiting admission", (|s: &Snapshot| s.queued) as fn(&Snapshot) -> u64),
            ("st.resource.holds", "Durable admission holds affecting this lane", (|s: &Snapshot| s.holds) as fn(&Snapshot) -> u64),
        ] {
            let snapshots = Arc::clone(&snapshots);
            meter.u64_observable_gauge(name).with_unit("{item}").with_description(description)
                .with_callback(move |observer| {
                    let Ok(snapshots) = snapshots.read() else { return; };
                    for (lane, snapshot) in snapshots.iter() {
                        observer.observe(select(snapshot), &[KeyValue::new("st.resource.lane", lane.clone())]);
                    }
                }).build();
        }
        let duration = |name: &'static str, description: &'static str| meter.f64_histogram(name).with_unit("s").with_description(description).build();
        Self {
            snapshots,
            wait: duration("st.resource.wait.duration", "Queue and admission preparation time, not payload time"),
            execution: duration("st.resource.execution.duration", "Payload-release to execution-end time"),
            release: duration("st.resource.release.duration", "Retirement request to confirmed retirement or uncertainty"),
            hold: duration("st.resource.hold.duration", "Durable hold age at authenticated resolution"),
        }
    }
}

/// One observer per execution host authority; the host is an exporter resource attribute.
#[derive(Clone)]
pub struct LaneTelemetry { instruments: Option<Arc<Instruments>> }

impl Default for LaneTelemetry {
    fn default() -> Self { Self::new() }
}

impl LaneTelemetry {
    pub fn new() -> Self {
        Self { instruments: super::metrics_enabled().then(|| Arc::new(Instruments::new(&opentelemetry::global::meter("st3.resource")))) }
    }

    /// Counts come from the authoritative reconciled ledger, not guard creation/destruction.
    /// Include zero counts and retain occupancy until the whole generation is retired.
    /// `lane` must be a configured policy name, never a membership/run/hold identifier.
    pub fn snapshot(&self, lane: &str, occupancy: u64, queued: u64, holds: u64) {
        if let Some(metrics) = &self.instruments
            && let Ok(mut snapshots) = metrics.snapshots.write()
        {
            if let Some(snapshot) = snapshots.get_mut(lane) {
                *snapshot = Snapshot { occupancy, queued, holds };
            } else {
                snapshots.insert(lane.to_owned(), Snapshot { occupancy, queued, holds });
            }
        }
    }

    /// Admission begins at queue entry; execution begins only at confirmed payload release.
    /// Release ends only at confirmed retirement; an uncertain end never frees capacity.
    pub fn phase(&self, phase: LanePhaseKind, identity: &LaneIdentity, links: &[opentelemetry::Context]) -> LanePhase {
        let span = if super::export_enabled() {
            let mut attributes = identity.attributes();
            attributes.push(KeyValue::new("span.label", phase.label()));
            let links = links.iter().filter_map(|context| {
                let span = context.span().span_context().clone();
                span.is_valid().then(|| opentelemetry::trace::Link::new(span, Vec::new(), 0))
            }).collect::<Vec<_>>();
            Some(opentelemetry::global::tracer("st3").build_with_context(
                opentelemetry::global::tracer("st3").span_builder(phase.name())
                    .with_kind(opentelemetry::trace::SpanKind::Internal).with_attributes(attributes).with_links(links),
                &opentelemetry::Context::new(),
            ))
        } else { None };
        let guard = LanePhase { phase, identity: identity.clone(), started: Instant::now(), prior_elapsed: Duration::ZERO, clock_uncertain: false, instruments: self.instruments.clone(), span, finished: false };
        guard.log("started", None);
        guard
    }

    /// The durable hold store supplies its persisted age; no automatic expiry is implied.
    pub fn hold_resolved(&self, lane: &str, elapsed: Duration) {
        if let Some(metrics) = &self.instruments {
            metrics.hold.record(elapsed.as_secs_f64(), &[KeyValue::new("st.resource.lane", lane.to_owned())]);
        }
    }
}

#[must_use = "retain the phase until the authoritative transition and finish with its outcome"]
pub struct LanePhase {
    phase: LanePhaseKind,
    identity: LaneIdentity,
    started: Instant,
    prior_elapsed: Duration,
    clock_uncertain: bool,
    instruments: Option<Arc<Instruments>>,
    span: Option<opentelemetry::global::BoxedSpan>,
    finished: bool,
}

impl LanePhase {
    pub fn finish(mut self, outcome: LaneOutcome) { self.complete(outcome); }

    /// Queue entry precedes generation assignment; bind it when admission commits.
    pub fn set_generation(&mut self, generation: &str) {
        self.identity.generation = generation.to_owned();
        if let Some(span) = self.span.as_mut() {
            span.set_attribute(KeyValue::new("st.resource.generation", generation.to_owned()));
        }
    }

    /// Resume observation after recovery without granting a fresh elapsed-time budget.
    /// The new span is only this observation segment; its elapsed attribute includes prior age.
    pub fn rebase(mut self, prior_elapsed: Duration, clock_uncertain: bool) -> Self {
        self.prior_elapsed = prior_elapsed;
        self.clock_uncertain = clock_uncertain;
        if let Some(span) = self.span.as_mut() {
            span.set_attribute(KeyValue::new("st.resource.clock_uncertain", clock_uncertain));
            span.set_attribute(KeyValue::new("st.resource.prior_elapsed", prior_elapsed.as_secs_f64()));
        }
        if clock_uncertain { self.log("resumed", Some(prior_elapsed.as_secs_f64())); }
        self
    }

    fn log(&self, outcome: &'static str, elapsed: Option<f64>) {
        let trace_id = self.span.as_ref().map(|span| span.span_context().trace_id().to_string());
        let span_id = self.span.as_ref().map(|span| span.span_context().span_id().to_string());
        tracing::info!(target: "fractal.resource", event = "resource.lane.phase",
            phase = self.phase.label(), outcome, elapsed_seconds = elapsed, clock_uncertain = self.clock_uncertain,
            st.resource.lane = %self.identity.lane, st.resource.membership = %self.identity.membership,
            st.run = %self.identity.run, st.step = %self.identity.step, st.attempt = self.identity.attempt,
            st.resource.generation = %self.identity.generation, st.invocation.id = self.identity.invocation.as_deref(),
            trace_id = trace_id.as_deref(), span_id = span_id.as_deref(), "resource lane phase");
    }

    fn complete(&mut self, outcome: LaneOutcome) {
        let elapsed = self.prior_elapsed.saturating_add(self.started.elapsed()).as_secs_f64();
        if let Some(metrics) = &self.instruments {
            let histogram = match self.phase { LanePhaseKind::Admission => &metrics.wait, LanePhaseKind::Execution => &metrics.execution, LanePhaseKind::Release => &metrics.release };
            histogram.record(elapsed, &[
                KeyValue::new("st.resource.lane", self.identity.lane.clone()),
                KeyValue::new("st.resource.outcome", outcome.label()),
            ]);
        }
        self.log(outcome.label(), Some(elapsed));
        if let Some(mut span) = self.span.take() {
            span.set_attribute(KeyValue::new("st.resource.outcome", outcome.label()));
            span.set_attribute(KeyValue::new("st.resource.elapsed", elapsed));
            if matches!(outcome, LaneOutcome::Failed | LaneOutcome::Uncertain) {
                span.set_status(opentelemetry::trace::Status::error(outcome.label()));
            }
            span.end();
        }
        self.finished = true;
    }
}

impl Drop for LanePhase {
    fn drop(&mut self) {
        if !self.finished { self.complete(LaneOutcome::Uncertain); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    fn identity() -> LaneIdentity {
        LaneIdentity {
            lane: "executor".into(), membership: "membership-public-fixture".into(),
            run: "run-public-fixture".into(), step: "build".into(), attempt: 3,
            generation: "generation-public-fixture".into(), invocation: Some("invocation-public-fixture".into()),
        }
    }

    #[test]
    fn exports_authoritative_snapshots_and_all_clocks_without_identity_labels() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let telemetry = LaneTelemetry {
            instruments: Some(Arc::new(Instruments::new(&provider.meter("st3.resource")))),
        };
        telemetry.snapshot("executor", 1, 2, 1);
        // Reconciliation replaces rather than adds counts. Zero queue and holds stay visible.
        telemetry.snapshot("executor", 1, 0, 0);
        let id = identity();
        for phase in [LanePhaseKind::Admission, LanePhaseKind::Execution, LanePhaseKind::Release] {
            telemetry.phase(phase, &id, &[]).rebase(Duration::from_secs(5), true).finish(LaneOutcome::Succeeded);
        }
        // A dropped observer cannot imply physical retirement.
        drop(telemetry.phase(LanePhaseKind::Release, &id, &[]));
        telemetry.hold_resolved("executor", Duration::from_secs(42));
        provider.force_flush().unwrap();
        let batches = exporter.get_finished_metrics().unwrap();
        let mut gauges = BTreeMap::new();
        let mut durations = BTreeMap::new();
        for metric in batches.iter().flat_map(|batch| batch.scope_metrics()).flat_map(|scope| scope.metrics()) {
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
                    for point in gauge.data_points() {
                        assert_eq!(point.attributes().collect::<Vec<_>>(), vec![&KeyValue::new("st.resource.lane", "executor")]);
                        gauges.insert(metric.name().to_owned(), point.value());
                    }
                }
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => {
                    for point in histogram.data_points() {
                        let attributes = point.attributes().map(|attribute|
                            (attribute.key.as_str().to_owned(), attribute.value.to_string())
                        ).collect::<BTreeMap<_, _>>();
                        assert!(attributes.keys().all(|key| key == "st.resource.lane" || key == "st.resource.outcome"));
                        durations.insert((metric.name().to_owned(), attributes.get("st.resource.outcome").cloned()), (point.count(), point.sum()));
                    }
                }
                _ => panic!("unexpected resource instrument"),
            }
        }
        assert_eq!(gauges, BTreeMap::from([
            ("st.resource.occupancy".into(), 1), ("st.resource.queue".into(), 0), ("st.resource.holds".into(), 0),
        ]));
        for name in ["st.resource.wait.duration", "st.resource.execution.duration", "st.resource.release.duration"] {
            assert_eq!(durations[&(name.into(), Some("succeeded".into()))].0, 1);
            assert!(durations[&(name.into(), Some("succeeded".into()))].1 >= 5.0);
        }
        assert_eq!(durations[&("st.resource.release.duration".into(), Some("uncertain".into()))].0, 1);
        assert_eq!(durations[&("st.resource.hold.duration".into(), None)], (1, 42.0));
        provider.shutdown().unwrap();
    }

    #[test]
    fn trace_identity_keeps_integer_attempt_and_optional_invocation() {
        let mut id = identity();
        let attributes = id.attributes();
        assert!(attributes.contains(&KeyValue::new("st.attempt", 3_i64)));
        assert!(attributes.contains(&KeyValue::new("st.resource.generation", "generation-public-fixture")));
        assert!(attributes.contains(&KeyValue::new("st.run", "run-public-fixture")));
        assert!(attributes.contains(&KeyValue::new("st.invocation.id", "invocation-public-fixture")));
        id.invocation = None;
        assert!(!id.attributes().iter().any(|attribute| attribute.key.as_str() == "st.invocation.id"));
    }
}
