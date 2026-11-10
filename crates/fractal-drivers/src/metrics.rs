//! Bounded native hook invocation capture for Fractal's observation exporter.
//!
//! Capture records the shared application point on this thread without installing an SDK
//! provider. Hook events use the bounded vocabulary shared with the observation exporter.

use std::cell::RefCell;

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
}

/// The bounded event vocabulary shared by native hook capture and the observation exporter.
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
    fn unknown_hook_events_collapse_to_other() {
        assert_eq!(normalize_hook_event("SessionStart"), "SessionStart");
        assert_eq!(normalize_hook_event("TotallyNewEvent"), "other");
    }
}
