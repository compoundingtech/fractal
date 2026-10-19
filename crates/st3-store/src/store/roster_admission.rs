//! Admission telemetry stays on the caller's existing span; only cold builds have roots.

use super::*;
use opentelemetry::trace::TraceContextExt as _;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

#[derive(Default)]
pub struct AdmissionState {
    next_id: u64,
    pub holder: Option<Arc<Holder>>,
}

pub enum Blocked {
    Known {
        id: u64,
        class: &'static str,
        context: opentelemetry::trace::SpanContext,
        age: std::time::Duration,
    },
    // Tokio can reserve a released permit for a queued future not yet polled again.
    // It exposes neither that future nor an acquired holder to identify here.
    Unknown,
}

pub enum Blocker {
    Known { holder: Arc<Holder>, age: std::time::Duration },
    Unknown,
}

pub struct Holder {
    pub id: u64,
    pub class: &'static str,
    pub acquired: std::time::Instant,
    pub waited: std::time::Duration,
    pub blocked: Option<Blocked>,
    pub span: tracing::Span,
    pub context: Mutex<opentelemetry::Context>,
    pub rebuilds: Mutex<Vec<opentelemetry::Context>>,
    pub worker: std::sync::OnceLock<std::thread::ThreadId>,
}

impl AdmissionState {
    pub fn register(
        &mut self,
        class: &'static str,
        acquired: std::time::Instant,
        waited: std::time::Duration,
        blocked: Option<Blocker>,
        span: Option<&tracing::Span>,
    ) -> Option<Arc<Holder>> {
        let holder = span.map(|span| {
            self.next_id += 1;
            Arc::new(Holder {
                id: self.next_id, class, acquired, waited,
                blocked: blocked.map(|blocked| match blocked {
                    Blocker::Known { holder, age } => Blocked::Known {
                        id: holder.id, class: holder.class, age,
                        context: holder.context.lock().expect("roster holder context poisoned")
                            .span().span_context().clone(),
                    },
                    Blocker::Unknown => Blocked::Unknown,
                }),
                context: Mutex::new(span.context()), span: span.clone(),
                rebuilds: Mutex::new(Vec::new()),
                worker: std::sync::OnceLock::new(),
            })
        });
        self.holder = holder.clone();
        holder
    }
}
