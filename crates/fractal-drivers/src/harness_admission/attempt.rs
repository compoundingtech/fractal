//! Optional local diagnostic extension. It never supplies admission authority.

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{Driver, ProbeDiagnostic, Scratch};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptDiagnostic {
    pub incarnation: String,
    pub phase: String,
    pub outcome: String,
    pub elapsed_ms: u64,
    pub deadline_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub stderr_tail: String,
    pub error_chain: String,
}

impl AttemptDiagnostic {
    /// Render only the current incarnation. Re-sanitize on read as well as write: this local
    /// file can also be produced by a different driver generation.
    pub fn explanation(&self, incarnation: &str) -> Option<String> {
        if incarnation.is_empty() || self.incarnation != incarnation {
            return None;
        }
        Some(format!(
            "phase={} outcome={} elapsed={}ms deadline={} exit_code={:?} exit_signal={:?}; {}; stderr: {}",
            sanitize(&self.phase, 80),
            sanitize(&self.outcome, 80),
            self.elapsed_ms,
            self.deadline_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "none".into()),
            self.exit_code,
            self.exit_signal,
            sanitize(&self.error_chain, 2048),
            sanitize(&self.stderr_tail, 4096),
        ))
    }
}

pub(super) struct Capture {
    diagnostic: AttemptDiagnostic,
    started: Instant,
    // Preserve a failed contract's own timing while checking identity and writing its cache.
    probe_failure: Option<ProbeDiagnostic>,
}

impl Capture {
    pub(super) fn new(incarnation: &str) -> Self {
        Self {
            diagnostic: AttemptDiagnostic {
                incarnation: incarnation.chars().take(512).collect(),
                phase: "identity".into(),
                outcome: "error".into(),
                elapsed_ms: 0,
                deadline_ms: None,
                exit_code: None,
                exit_signal: None,
                stderr_tail: String::new(),
                error_chain: String::new(),
            },
            started: Instant::now(),
            probe_failure: None,
        }
    }

    pub(super) fn phase(&mut self, phase: &str, deadline: Option<Duration>) {
        self.diagnostic.phase = phase.into();
        self.diagnostic.deadline_ms = deadline.map(|d| d.as_millis() as u64);
        self.diagnostic.exit_code = None;
        self.diagnostic.exit_signal = None;
        self.started = Instant::now();
    }

    pub(super) fn exit(&mut self, exit: ExitStatus) {
        self.diagnostic.exit_code = exit.code();
        self.diagnostic.exit_signal = exit.signal();
    }

    pub(super) fn failure(&mut self, outcome: &str, detail: &str) {
        self.diagnostic.outcome = outcome.into();
        self.diagnostic.error_chain = sanitize(detail, 2048);
    }

    pub(super) fn error(&mut self, error: &anyhow::Error) {
        let timeout = error
            .chain()
            .any(|e| e.to_string() == "admission process timed out");
        self.failure(
            if timeout { "timeout" } else { "error" },
            &format!("{error:#}"),
        );
        // If the final error is cache publication/identity verification, report that error,
        // rather than an earlier missing contract observation.
        self.probe_failure = None;
    }

    pub(super) fn probe(&mut self, diagnostic: &ProbeDiagnostic) {
        self.probe_failure = Some(diagnostic.clone());
    }

    pub(super) fn finish(mut self, scratch: Option<&Scratch>, driver: Driver) -> AttemptDiagnostic {
        self.diagnostic.elapsed_ms = self.started.elapsed().as_millis() as u64;
        if let Some(probe) = self.probe_failure {
            self.diagnostic.phase = probe.phase;
            self.diagnostic.outcome = probe.outcome;
            self.diagnostic.elapsed_ms = probe.elapsed_ms;
            self.diagnostic.deadline_ms = Some(super::PROBE_TIMEOUT.as_millis() as u64);
            self.diagnostic.exit_code = probe.exit_code;
            self.diagnostic.error_chain = sanitize(&probe.detail, 2048);
            self.diagnostic.stderr_tail = sanitize(&probe.stderr_tail, 4096);
        } else if let Some(scratch) = scratch {
            let name = if self.diagnostic.phase == "version" {
                "version"
            } else {
                driver.as_str()
            };
            self.diagnostic.stderr_tail = match scratch.stderr_tail(&format!("{name}.stderr")) {
                Ok(tail) => sanitize(&tail, 4096),
                Err(error) => format!("capture unavailable: {}", sanitize(&error.to_string(), 256)),
            };
        }
        self.diagnostic
    }
}

/// No argv or environment dump is retained. Withhold credential-bearing lines entirely rather
/// than guessing where a quoted, spaced or punctuation-delimited secret ends.
fn sanitize(text: &str, bound: usize) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    let mut output = String::new();
    for line in clean.lines() {
        let lower = line.to_ascii_lowercase();
        let sensitive = [
            "authorization",
            "bearer ",
            "api_key",
            "api-key",
            "apikey",
            "token",
            "password",
            "secret",
            "sk-",
            "ghp_",
            "akia",
            "private key",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
            || (lower.contains("://") && lower.contains('@'));
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(if sensitive {
            "[redacted credential-bearing line]"
        } else {
            line
        });
        // A credential may continue on the next line (a header, YAML value or PEM block).
        // Withhold the remainder rather than leaking a continuation without its label.
        if sensitive {
            break;
        }
        if output.len() >= bound {
            break;
        }
    }
    let mut end = output.len().min(bound);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    output
}
