//! Bounded attempt roots and durable W3C correlation, independent of trace export.
use std::collections::{BTreeMap, HashMap};

use opentelemetry::trace::{Span as _, TraceContextExt as _, Tracer as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry::propagation::TextMapPropagator as _;
use serde::{Deserialize, Serialize};

/// Store this with the invocation receipt, not in the sampled telemetry backend.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptTraceContext {
    pub traceparent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

impl AttemptTraceContext {
    pub fn context(&self) -> Context {
        TraceContextPropagator::new().extract_with_context(&Context::new(), &HashMap::from([
            ("traceparent".to_owned(), self.traceparent.clone()),
            ("tracestate".to_owned(), self.tracestate.clone().unwrap_or_default()),
        ]))
    }

    pub fn environment(&self, env: &mut BTreeMap<String, String>) {
        env.insert("TRACEPARENT".to_owned(), self.traceparent.clone());
        // Never inherit another invocation's tracestate.
        env.insert("TRACESTATE".to_owned(), self.tracestate.clone().unwrap_or_default());
    }

    pub fn trace_id(&self) -> String {
        self.context().span().span_context().trace_id().to_string()
    }
}

/// Each launch/resume has a root, never a child of the previous attempt. Dropping the
/// owner closes the span on success, failure, cancellation, or host replacement.
pub struct AttemptTrace {
    context: Context,
    pub correlation: AttemptTraceContext,
}

impl AttemptTrace {
    pub fn start(name: &'static str, attributes: Vec<KeyValue>, links: &[AttemptTraceContext]) -> Self {
        let links = links.iter().map(AttemptTraceContext::context)
            .filter_map(|context| {
                let span = context.span().span_context().clone();
                span.is_valid().then(|| opentelemetry::trace::Link::new(span, Vec::new(), 0))
            }).collect();
        let span = opentelemetry::global::tracer("fractal.attempt")
            .span_builder(name).with_attributes(attributes).with_links(links)
            .start_with_context(&opentelemetry::global::tracer("fractal.attempt"), &Context::new());
        let context = if span.span_context().is_valid() {
            Context::new().with_span(span)
        } else {
            // No SDK/exporter is a supported mode. IDs remain real durable correlation,
            // but the flags honestly say that no recording span exists.
            use opentelemetry_sdk::trace::IdGenerator as _;
            let generator = opentelemetry_sdk::trace::RandomIdGenerator::default();
            Context::new().with_remote_span_context(opentelemetry::trace::SpanContext::new(
                generator.new_trace_id(), generator.new_span_id(),
                opentelemetry::trace::TraceFlags::NOT_SAMPLED, false,
                opentelemetry::trace::TraceState::default(),
            ))
        };
        let mut carrier = HashMap::new();
        TraceContextPropagator::new().inject_context(&context, &mut carrier);
        let correlation = AttemptTraceContext {
            traceparent: carrier.remove("traceparent").expect("valid attempt context"),
            // These local roots follow the existing collector-owned sampling contract.
            tracestate: Some("st=c".to_owned()),
        };
        Self { context, correlation }
    }

    pub fn failed(&self) {
        self.context.span().set_status(opentelemetry::trace::Status::error("attempt failed"));
    }
}

impl Drop for AttemptTrace {
    fn drop(&mut self) { self.context.span().end(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_survives_without_exporter_and_overwrites_stale_environment() {
        let attempt = AttemptTrace::start("st.program.attempt", Vec::new(), &[]);
        let context = attempt.correlation.context();
        assert!(context.span().span_context().is_valid());
        let mut env = BTreeMap::from([("TRACESTATE".into(), "stale=parent".into())]);
        attempt.correlation.environment(&mut env);
        assert_eq!(env["TRACEPARENT"], attempt.correlation.traceparent);
        assert_eq!(env["TRACESTATE"], "st=c");
        let without_state = AttemptTraceContext { tracestate: None, ..attempt.correlation.clone() };
        without_state.environment(&mut env);
        assert_eq!(env["TRACESTATE"], "");
        let retry = AttemptTrace::start("st.program.attempt", Vec::new(), std::slice::from_ref(&attempt.correlation));
        assert_ne!(attempt.correlation.trace_id(), retry.correlation.trace_id());
    }

    #[test]
    fn retry_roots_export_links_without_a_parent_chain() {
        const CHILD: &str = "FRACTAL_ATTEMPT_TRACE_TEST";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "attempt_trace::tests::retry_roots_export_links_without_a_parent_chain"])
                .env(CHILD, "1").status().unwrap();
            assert!(status.success());
            return;
        }
        let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone()).build();
        opentelemetry::global::set_tracer_provider(provider.clone());
        let run = AttemptTrace::start("st.mission.run", Vec::new(), &[]);
        let first = AttemptTrace::start("st.program.attempt", Vec::new(), std::slice::from_ref(&run.correlation));
        let retry = AttemptTrace::start("st.program.attempt", Vec::new(), &[
            run.correlation.clone(), first.correlation.clone(),
        ]);
        let first_context = first.correlation.context().span().span_context().clone();
        retry.failed();
        drop(retry);
        drop(first);
        drop(run);
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 3);
        assert!(spans.iter().all(|span| span.parent_span_id == opentelemetry::trace::SpanId::INVALID));
        let retry = &spans[0];
        assert_eq!(retry.links.links.len(), 2);
        assert!(matches!(retry.status, opentelemetry::trace::Status::Error { .. }));
        assert_eq!(retry.links.links[1].span_context, first_context);
        assert_ne!(retry.span_context.trace_id(), first_context.trace_id());
    }
}
