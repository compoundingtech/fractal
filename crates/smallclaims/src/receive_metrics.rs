//! Request-local receive accounting. Context is explicitly captured at writer boundaries.
//! Phase wall times contain nested costs; they must not be added to writer or shared time.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const ENVELOPE_SIZE_SAMPLE_LIMIT: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct Counters {
    pub envelopes_admitted: usize,
    /// Preparation attempts, including persisted backlog and retries, not unique envelopes.
    pub envelopes_prepared: usize,
    pub envelope_claims: usize,
    pub envelope_payload_bytes: usize,
    /// First bounded sample of decoded claims and wire bytes, in preparation order.
    pub envelope_sizes: Vec<(usize, usize)>,
    pub envelope_sizes_omitted: usize,
    pub finalizer_time: [Duration; 5],
    pub admit_time: Duration,
    pub project_time: Duration,
    pub projection_chunks: u64,
    pub gate_wait: Duration,
    pub writer_wait: Duration,
    pub final_drain_count: u64,
    pub final_drain_time: Duration,
    pub rebuild_part_count: u64,
    pub rebuild_part_time: Duration,
    /// Successfully committed managed writer transactions, excluding savepoints.
    pub commits: u64,
    pub contended: bool,
    pub projection_error: Option<&'static str>,
    pub projection_stage: Option<String>,
    pub deferred: bool,
    pub phases: [PhaseCounters; 6],
    /// Reverse-link shared events through their participant correlation IDs, without retaining
    /// an unbounded list of batch IDs on a long-running request.
    pub shared_finalizations: u64,
    pub first_shared_batch: Option<u64>,
    pub last_shared_batch: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestMetrics {
    counters: Arc<Mutex<Counters>>,
    correlation_id: Arc<str>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseCounters {
    pub elapsed: Duration,
    pub writer_wait: Duration,
    pub writer_hold: Duration,
}

#[derive(Clone, Debug)]
pub struct Context {
    metrics: RequestMetrics,
    stage: Stage,
}

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

impl RequestMetrics {
    pub fn new(correlation_id: String) -> Self {
        Self { correlation_id: correlation_id.into(), ..Self::default() }
    }

    pub fn enter(&self) -> Scope {
        Scope {
            previous: CURRENT.with(|current| current.replace(Some(Context {
                metrics: self.clone(), stage: Stage::ResponsePostcommit,
            }))),
            _not_send: std::marker::PhantomData,
        }
    }

    pub fn snapshot(&self) -> Counters {
        self.counters.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Restores the prior scope even when a receive fails or unwinds on a reused worker thread.
#[must_use]
pub struct Scope {
    previous: Option<Context>,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

fn record(update: impl FnOnce(&mut Counters)) {
    CURRENT.with(|current| {
        if let Some(metrics) = current.borrow().as_ref() {
            update(&mut metrics.metrics.counters.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}

pub fn envelopes_admitted(count: usize) {
    record(|counters| counters.envelopes_admitted += count);
}

pub fn envelope_size(claims: usize, bytes: usize) {
    record(|counters| {
        counters.envelopes_prepared += 1;
        counters.envelope_claims += claims;
        counters.envelope_payload_bytes += bytes;
        if counters.envelope_sizes.len() < ENVELOPE_SIZE_SAMPLE_LIMIT {
            counters.envelope_sizes.push((claims, bytes));
        } else {
            counters.envelope_sizes_omitted += 1;
        }
    });
}

#[derive(Clone, Copy)]
pub enum Finalizer {
    Resources,
    Custom,
    Completion,
    RunEta,
    ClientMessages,
}

/// Attribute runtime finalization without changing its error or commit boundary.
pub fn finalizer<T>(part: Finalizer, work: impl FnOnce() -> T) -> T {
    if capture().is_none() {
        return work();
    }
    let started = std::time::Instant::now();
    let result = work();
    record(|counters| counters.finalizer_time[part as usize] += started.elapsed());
    result
}

pub fn projection_chunk() {
    record(|counters| counters.projection_chunks += 1);
}

pub fn projection_gate_wait(elapsed: Duration) {
    record(|counters| counters.gate_wait += elapsed);
}

pub fn projection_writer_wait(elapsed: Duration) {
    record(|counters| counters.writer_wait += elapsed);
}

pub fn final_drain(elapsed: Duration) {
    record(|counters| {
        counters.final_drain_count += 1;
        counters.final_drain_time += elapsed;
    });
}

/// Count attempted rebuilds, including work later rolled back, since it still costs time.
pub fn rebuild_part(elapsed: Duration) {
    record(|counters| {
        counters.rebuild_part_count += 1;
        counters.rebuild_part_time += elapsed;
    });
}

pub fn commit() {
    record(|counters| counters.commits += 1);
}

pub fn contended() {
    record(|counters| counters.contended = true);
}

pub fn projection_error(code: &'static str, stage: Option<&str>) {
    record(|counters| {
        counters.projection_error = Some(code);
        counters.projection_stage = stage.map(str::to_owned);
    });
}

pub fn deferred() {
    record(|counters| counters.deferred = true);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    ReceiptIngest,
    AdmissionPrelude,
    Admission,
    Repairs,
    Projection,
    ResponsePostcommit,
}

impl Stage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReceiptIngest => "receipt_ingest",
            Self::AdmissionPrelude => "admission_prelude",
            Self::Admission => "admission",
            Self::Repairs => "repairs",
            Self::Projection => "projection",
            Self::ResponsePostcommit => "response_postcommit",
        }
    }
}

pub fn capture() -> Option<Context> {
    CURRENT.with(|current| current.borrow().clone())
}

pub(crate) fn enter_shared() -> Scope {
    Scope {
        previous: CURRENT.with(|current| current.replace(None)),
        _not_send: std::marker::PhantomData,
    }
}

impl Context {
    pub fn correlation_id(&self) -> &str {
        &self.metrics.correlation_id
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn enter(&self) -> Scope {
        Scope {
            previous: CURRENT.with(|current| current.replace(Some(self.clone()))),
            _not_send: std::marker::PhantomData,
        }
    }

    pub fn writer_wait(&self, elapsed: Duration) {
        self.metrics.counters.lock().unwrap_or_else(PoisonError::into_inner)
            .phases[self.stage as usize].writer_wait += elapsed;
    }

    pub fn writer_hold(&self, elapsed: Duration) {
        self.metrics.counters.lock().unwrap_or_else(PoisonError::into_inner)
            .phases[self.stage as usize].writer_hold += elapsed;
    }

    fn shared_finalization(&self, batch_id: u64) {
        let mut counters = self.metrics.counters.lock().unwrap_or_else(PoisonError::into_inner);
        counters.shared_finalizations += 1;
        counters.first_shared_batch.get_or_insert(batch_id);
        counters.last_shared_batch = Some(batch_id);
    }
}

/// Actual exclusive writer execution, captured on the requester and completed on its owner.
pub(crate) struct WriterSample {
    context: Context,
    started: std::time::Instant,
}

impl WriterSample {
    pub(crate) fn start(context: &Context, waited: Duration) -> Self {
        context.writer_wait(waited);
        Self { context: context.clone(), started: std::time::Instant::now() }
    }
}

impl Drop for WriterSample {
    fn drop(&mut self) {
        self.context.writer_hold(self.started.elapsed());
    }
}

/// One bounded group-commit event. Participant IDs are the authoritative reverse links from
/// request summaries; first/last IDs in summaries are only a bounded index, not an exhaustive list.
pub(crate) struct SharedFinalization {
    batch_id: u64,
    participants: Vec<Context>,
    prepare: Duration,
    finalize_started: Option<std::time::Instant>,
}

#[derive(Debug)]
pub(crate) struct SharedEvent {
    pub(crate) batch_id: u64,
    pub(crate) participants: Vec<(Arc<str>, &'static str)>,
}

impl SharedFinalization {
    pub(crate) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self { batch_id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            participants: Vec::new(), prepare: Duration::ZERO, finalize_started: None }
    }

    pub(crate) fn batch_id(&self) -> Option<u64> {
        (!self.participants.is_empty()).then_some(self.batch_id)
    }

    pub(crate) fn prepared(&mut self, elapsed: Duration) {
        self.prepare = elapsed;
    }

    pub(crate) fn finalizing(&mut self) {
        self.finalize_started = self.batch_id().map(|_| std::time::Instant::now());
    }

    pub(crate) fn participant(&mut self, context: Option<&Context>) {
        if let Some(context) = context
            && !self.participants.iter().any(|other|
                Arc::ptr_eq(&other.metrics.counters, &context.metrics.counters)
                    && other.stage == context.stage)
        {
            // run_write_batch accepts no more than WRITE_BATCH_LIMIT callbacks.
            assert!(self.participants.len() < crate::sqlite::WRITE_BATCH_LIMIT);
            self.participants.push(context.clone());
        }
    }

    pub(crate) fn finish(mut self) -> Option<SharedEvent> {
        let event = self.event();
        self.participants.clear();
        event
    }

    fn event(&self) -> Option<SharedEvent> {
        if self.participants.is_empty() {
            return None;
        }
        let finalize = self.finalize_started.map_or(Duration::ZERO, |started| started.elapsed());
        for (index, context) in self.participants.iter().enumerate() {
            // A request may have several participating phases, but links one event only once.
            if !self.participants[..index].iter().any(|other|
                Arc::ptr_eq(&other.metrics.counters, &context.metrics.counters))
            {
                context.shared_finalization(self.batch_id);
            }
        }
        let event = SharedEvent { batch_id: self.batch_id,
            participants: self.participants.iter().map(|context|
                (context.metrics.correlation_id.clone(), context.stage.name())).collect() };
        tracing::info!(
            batch_id = event.batch_id,
            participants = ?event.participants,
            shared = true,
            additive = false,
            prepare_ms = self.prepare.as_secs_f64() * 1000.0,
            finalization_ms = finalize.as_secs_f64() * 1000.0,
            "replication receive shared finalization",
        );
        Some(event)
    }
}

impl Drop for SharedFinalization {
    fn drop(&mut self) {
        // Also publish links on a finalizer/observer unwind, before ACK senders are dropped.
        let _ = self.event();
    }
}

#[must_use]
pub struct Timer {
    context: Option<Context>,
    previous: Option<Context>,
    started: Option<std::time::Instant>,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Timer {
    pub fn start(stage: Stage) -> Self {
        let context = capture().map(|mut context| { context.stage = stage; context });
        let started = context.as_ref().map(|_| std::time::Instant::now());
        let previous = CURRENT.with(|current| current.replace(context.clone()));
        Self { context, previous, started, _not_send: std::marker::PhantomData }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let (Some(context), Some(started)) = (&self.context, self.started) {
            let elapsed = started.elapsed();
            let mut counters = context.metrics.counters.lock().unwrap_or_else(PoisonError::into_inner);
            counters.phases[context.stage as usize].elapsed += elapsed;
            match context.stage {
                Stage::Admission => counters.admit_time += elapsed,
                Stage::Projection => counters.project_time += elapsed,
                _ => {}
            }
        }
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_sizes_and_failed_finalizers_remain_request_local() {
        let metrics = RequestMetrics::new("sizes".into());
        {
            let _scope = metrics.enter();
            envelope_size(3, 1024);
            envelope_size(7, 2048);
            let failed: Result<(), &str> = finalizer(Finalizer::Resources, || Err("fixture"));
            assert_eq!(failed, Err("fixture"));
        }
        envelope_size(99, 99);
        let counters = metrics.snapshot();
        assert_eq!(counters.envelope_sizes, [(3, 1024), (7, 2048)]);
        assert!(counters.finalizer_time[Finalizer::Resources as usize] > Duration::ZERO);
        assert_eq!(counters.finalizer_time[Finalizer::Custom as usize], Duration::ZERO);
    }

    #[test]
    fn envelope_size_samples_are_bounded_without_losing_request_totals() {
        let first = RequestMetrics::new("first".into());
        let second = RequestMetrics::new("second".into());
        let count = ENVELOPE_SIZE_SAMPLE_LIMIT * 3 + 5;
        for claims in 1..=count {
            // Re-entering the request models preparation across backlog batches and retries.
            let _scope = first.enter();
            envelope_size(claims, claims * 10);
        }
        {
            let _first = first.enter();
            let _second = second.enter();
            envelope_size(7, 70);
        }
        envelope_size(99, 990);

        let counters = first.snapshot();
        let total_claims = count * (count + 1) / 2;
        assert_eq!(counters.envelopes_prepared, count);
        assert_eq!(counters.envelope_claims, total_claims);
        assert_eq!(counters.envelope_payload_bytes, total_claims * 10);
        assert_eq!(counters.envelope_sizes_omitted, count - ENVELOPE_SIZE_SAMPLE_LIMIT);
        assert_eq!(
            counters.envelope_sizes,
            (1..=ENVELOPE_SIZE_SAMPLE_LIMIT).map(|claims| (claims, claims * 10)).collect::<Vec<_>>(),
        );
        let counters = second.snapshot();
        assert_eq!(counters.envelopes_prepared, 1);
        assert_eq!(counters.envelope_claims, 7);
        assert_eq!(counters.envelope_payload_bytes, 70);
        assert_eq!(counters.envelope_sizes, [(7, 70)]);
        assert_eq!(counters.envelope_sizes_omitted, 0);
    }

    #[test]
    fn captured_phase_crosses_threads_and_restores_after_unwind() {
        let metrics = RequestMetrics::new("request-a".into());
        let context = {
            let _request = metrics.enter();
            let _phase = Timer::start(Stage::Repairs);
            capture().unwrap()
        };
        std::thread::spawn(move || {
            assert!(capture().is_none());
            let result = std::panic::catch_unwind(|| {
                let _scope = context.enter();
                assert_eq!(capture().unwrap().correlation_id(), "request-a");
                assert_eq!(capture().unwrap().stage(), Stage::Repairs);
                context.writer_wait(Duration::from_millis(3));
                context.writer_hold(Duration::from_millis(7));
                commit();
                panic!("test restoration");
            });
            assert!(result.is_err());
            assert!(capture().is_none());
        }).join().unwrap();
        let counters = metrics.snapshot();
        assert_eq!(counters.commits, 1);
        assert_eq!(counters.phases[Stage::Repairs as usize].writer_wait, Duration::from_millis(3));
        assert_eq!(counters.phases[Stage::Repairs as usize].writer_hold, Duration::from_millis(7));
        assert_eq!(counters.phases[Stage::Projection as usize].writer_hold, Duration::ZERO);
    }

    #[test]
    fn two_requests_link_the_same_shared_event_without_exclusive_time() {
        use std::collections::BTreeMap;

        #[derive(Clone, Default)]
        struct Events(Arc<Mutex<Vec<BTreeMap<String, String>>>>);
        impl tracing::Subscriber for Events {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool { true }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                struct Fields(BTreeMap<String, String>);
                impl tracing::field::Visit for Fields {
                    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                        self.0.insert(field.name().into(), format!("{value:?}"));
                    }
                }
                let mut fields = Fields(BTreeMap::new());
                event.record(&mut fields);
                self.0.lock().unwrap().push(fields.0);
            }
        }
        let logged = Events::default();
        let first = RequestMetrics::new("request-a".into());
        let second = RequestMetrics::new("request-b".into());
        let mut shared = SharedFinalization::new();
        {
            let _request = first.enter();
            let _phase = Timer::start(Stage::ReceiptIngest);
            shared.participant(capture().as_ref());
            shared.participant(capture().as_ref());
            {
                let _phase = Timer::start(Stage::Repairs);
                shared.participant(capture().as_ref());
            }
        }
        {
            let _request = second.enter();
            let _phase = Timer::start(Stage::AdmissionPrelude);
            shared.participant(capture().as_ref());
        }
        let id = shared.batch_id().unwrap();
        // Assert the emitted event as well as the exact payload used by the emitter.
        shared.finalizing();
        let event = tracing::subscriber::with_default(logged.clone(), || shared.finish().unwrap());
        let events = logged.0.lock().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["batch_id"], id.to_string());
        assert_eq!(events[0]["shared"], "true");
        assert_eq!(events[0]["additive"], "false");
        assert_eq!(events[0]["participants"], format!("{:?}", event.participants));
        assert!(!events[0].contains_key("correlation_id"), "no single shared owner");
        assert_eq!(event.batch_id, id);
        assert_eq!(event.participants, vec![
            (Arc::from("request-a"), "receipt_ingest"),
            (Arc::from("request-a"), "repairs"),
            (Arc::from("request-b"), "admission_prelude"),
        ]);
        for metrics in [&first, &second] {
            let counters = metrics.snapshot();
            assert_eq!(counters.shared_finalizations, 1);
            assert_eq!(counters.first_shared_batch, Some(event.batch_id));
            assert_eq!(counters.last_shared_batch, Some(event.batch_id));
            assert!(event.participants.iter().any(|(id, _)| id.as_ref() == metrics.correlation_id.as_ref()));
            assert!(counters.phases.iter().all(|phase| phase.writer_hold == Duration::ZERO));
        }
        assert!(capture().is_none(), "shared finalization must not enter one request");
    }

    #[test]
    fn receive_metrics_are_thread_local_and_restore_nested_scopes() {
        let outer = RequestMetrics::default();
        let inner = RequestMetrics::default();
        {
            let _outer = outer.enter();
            commit();
            projection_gate_wait(Duration::from_millis(2));
            projection_writer_wait(Duration::from_millis(3));
            final_drain(Duration::from_millis(5));
            std::thread::spawn(|| {
                commit();
                rebuild_part(Duration::from_secs(1));
            }).join().unwrap();
            {
                let _inner = inner.enter();
                projection_chunk();
                envelopes_admitted(2);
                contended();
            }
            rebuild_part(Duration::from_millis(3));
        }
        commit();
        assert_eq!(outer.snapshot().commits, 1);
        assert_eq!(outer.snapshot().rebuild_part_count, 1);
        assert_eq!(outer.snapshot().rebuild_part_time, Duration::from_millis(3));
        assert_eq!(outer.snapshot().projection_chunks, 0);
        assert_eq!(outer.snapshot().gate_wait, Duration::from_millis(2));
        assert_eq!(outer.snapshot().writer_wait, Duration::from_millis(3));
        assert_eq!(outer.snapshot().final_drain_count, 1);
        assert_eq!(outer.snapshot().final_drain_time, Duration::from_millis(5));
        assert!(!outer.snapshot().contended);
        assert_eq!(inner.snapshot().envelopes_admitted, 2);
        assert_eq!(inner.snapshot().projection_chunks, 1);
        assert!(inner.snapshot().contended);
        assert_eq!(inner.snapshot().commits, 0);
        assert_eq!(inner.snapshot().final_drain_count, 0);
    }
}
