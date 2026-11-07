//! Shared bounded driver/hook metrics. st captures hook applications for its daemon exporter
//! (docs/st3/observability.md).
//!
//! Every label value comes from a bounded enum (`result`, `driver`, hook registry name +
//! normalized event); identifiers never become metric labels — those live in span attributes.
//!
//! Recording is allocation-free unless a real meter provider is installed by
//! [`crate::telemetry::Telemetry::init`]: every record function checks [`enabled`] first and
//! returns before touching any instrument or allocating a label string. With no provider
//! installed, `opentelemetry::global` hands out a silent no-op meter anyway — this early-out
//! just keeps the disabled case allocation-free. An explicitly scoped invocation capture
//! records the hook's application point for st's daemon exporter without an SDK provider.

use std::cell::RefCell;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Meter};

use crate::driver_diagnostic::{Driver, Reason, Source, Stage, Support};
static ENABLED: AtomicBool = AtomicBool::new(false);

/// An invocation at the shared hook's application point. A caller can collect these without
/// installing an SDK provider, then hand them to its own asynchronous telemetry pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookInvocation {
    pub hook: &'static str,
    pub event: &'static str,
}

thread_local! {
    static HOOK_INVOCATIONS_CAPTURE: RefCell<Option<Vec<HookInvocation>>> = const { RefCell::new(None) };
}

/// Capture only this thread's invocations, restoring an enclosing capture even during unwind.
pub fn capture_hook_invocations<T>(run: impl FnOnce() -> T) -> (T, Vec<HookInvocation>) {
    struct Restore(Option<Vec<HookInvocation>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            HOOK_INVOCATIONS_CAPTURE.with(|capture| capture.replace(self.0.take()));
        }
    }
    let restore =
        Restore(HOOK_INVOCATIONS_CAPTURE.with(|capture| capture.replace(Some(Vec::new()))));
    let result = run();
    let invocations = HOOK_INVOCATIONS_CAPTURE.with(|capture| capture.take().unwrap_or_default());
    drop(restore);
    (result, invocations)
}

/// Whether a real meter provider is installed. False → recording is a free no-op.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Flip recording on when [`crate::telemetry::Telemetry::init`] installs the provider.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

static METER: LazyLock<Meter> = LazyLock::new(|| global::meter("st"));

static HOOK_INVOCATIONS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    METER
        .u64_counter("hook_invocations_total")
        .with_description("Lifecycle hook invocations applied in-process, by hook and event")
        .with_unit("1")
        .build()
});

static DRIVER_DIAGNOSTICS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    METER
        .u64_counter("driver_diagnostic_transitions_total")
        .with_description("Native driver diagnostic failures and recoveries by bounded boundary")
        .with_unit("1")
        .build()
});

/// One lifecycle-hook invocation reached its single in-process application point.
/// Unknown event names collapse to `other` so the label stays bounded.
pub fn record_hook_invocation(hook: &'static str, event: &str) {
    HOOK_INVOCATIONS_CAPTURE.with(|capture| {
        if let Some(invocations) = capture
            .borrow_mut()
            .as_mut()
            .filter(|invocations| invocations.len() < 16)
        {
            invocations.push(HookInvocation {
                hook,
                event: normalize_hook_event(event),
            });
        }
    });
    if !enabled() {
        return;
    }
    HOOK_INVOCATIONS.add(
        1,
        &[
            opentelemetry::KeyValue::new("hook", hook),
            opentelemetry::KeyValue::new("event", normalize_hook_event(event)),
        ],
    );
}

/// One native-driver boundary entered failure or recovered. Every label is a closed enum; raw
/// versions and agent/session/message identity stay on spans and logs.
pub fn record_driver_diagnostic(
    driver: Driver,
    stage: Stage,
    reason: Reason,
    source: Source,
    support: Support,
    recovered: bool,
) {
    if !enabled() {
        return;
    }
    DRIVER_DIAGNOSTICS.add(
        1,
        &driver_diagnostic_attributes(driver, stage, reason, source, support, recovered),
    );
}

fn driver_diagnostic_attributes(
    driver: Driver,
    stage: Stage,
    reason: Reason,
    source: Source,
    support: Support,
    recovered: bool,
) -> [opentelemetry::KeyValue; 6] {
    [
        opentelemetry::KeyValue::new("driver", driver.as_str()),
        opentelemetry::KeyValue::new("stage", stage.as_str()),
        opentelemetry::KeyValue::new("reason", reason.as_str()),
        opentelemetry::KeyValue::new("source", source.as_str()),
        opentelemetry::KeyValue::new("support", support.as_str()),
        opentelemetry::KeyValue::new("outcome", if recovered { "recovery" } else { "failure" }),
    ]
}

/// The bounded event vocabulary shared by SDK instruments and st's observation exporter.
pub fn normalize_hook_event(event: &str) -> &'static str {
    match event {
        "SessionStart" => "SessionStart",
        "UserPromptSubmit" => "UserPromptSubmit",
        "PreToolUse" => "PreToolUse",
        "PostToolUse" => "PostToolUse",
        "PermissionRequest" => "PermissionRequest",
        "Stop" => "Stop",
        "StopFailure" => "StopFailure",
        "SubagentStart" => "SubagentStart",
        "SubagentStop" => "SubagentStop",
        "SessionEnd" => "SessionEnd",
        "PreCompact" => "PreCompact",
        "PostCompact" => "PostCompact",
        "Notification" => "Notification",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_invocations_preserve_the_application_point_and_bounded_vocabulary() {
        let ((), outer) = capture_hook_invocations(|| {
            record_hook_invocation("claude-observe", "SessionStart");
            let ((), inner) = capture_hook_invocations(|| {
                record_hook_invocation("claude-observe", "new-event");
            });
            assert_eq!(
                inner,
                vec![HookInvocation {
                    hook: "claude-observe",
                    event: "other"
                }]
            );
            let other_thread = std::thread::spawn(|| {
                record_hook_invocation("claude-observe", "Stop");
            });
            other_thread.join().unwrap();
            record_hook_invocation("claude-observe", "PreToolUse");
        });
        assert_eq!(
            outer,
            vec![
                HookInvocation {
                    hook: "claude-observe",
                    event: "SessionStart"
                },
                HookInvocation {
                    hook: "claude-observe",
                    event: "PreToolUse"
                },
            ]
        );
    }

    #[test]
    fn scoped_invocations_restore_their_enclosing_capture_during_unwind() {
        let ((), outer) = capture_hook_invocations(|| {
            let panic = std::panic::catch_unwind(|| {
                capture_hook_invocations(|| {
                    record_hook_invocation("claude-observe", "Stop");
                    panic!("example capture fault");
                });
            });
            assert!(panic.is_err());
            record_hook_invocation("claude-observe", "PostToolUse");
        });
        assert_eq!(
            outer,
            vec![HookInvocation {
                hook: "claude-observe",
                event: "PostToolUse"
            }]
        );
    }

    #[test]
    fn disabled_by_default_and_recording_is_a_no_op() {
        // No meter provider installed in unit tests: enabled() is false and every record call
        // must return without panicking or touching instruments.
        assert!(!enabled());
        set_enabled(false);
        record_hook_invocation("claude-observe", "SomethingUnheardOf");
        assert!(!enabled());

        record_driver_diagnostic(
            Driver::OpenCode,
            Stage::Seed,
            Reason::UnknownStatus,
            Source::StatusSnapshot,
            Support::Supported,
            false,
        );
    }

    #[test]
    fn unknown_hook_events_collapse_to_other() {
        assert_eq!(normalize_hook_event("SessionStart"), "SessionStart");
        assert_eq!(normalize_hook_event("TotallyNewEvent"), "other");
    }

    #[test]
    fn driver_diagnostic_metric_attributes_are_exactly_the_bounded_axes() {
        let attributes = driver_diagnostic_attributes(
            Driver::Claude,
            Stage::ProviderAuth,
            Reason::ProviderAuthRejected,
            Source::TurnResult,
            Support::Unknown,
            true,
        );
        let keys: Vec<&str> = attributes
            .iter()
            .map(|attribute| attribute.key.as_str())
            .collect();
        assert_eq!(
            keys,
            ["driver", "stage", "reason", "source", "support", "outcome"]
        );
        let rendered = format!("{attributes:?}");
        for forbidden in ["1.18.19", "h.worker", "ses_", "msg_"] {
            assert!(!rendered.contains(forbidden), "{rendered}");
        }
    }
}
