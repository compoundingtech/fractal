//! Bounded stderr diagnostics shared by replication workers and daemon receive handlers.
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 4;

pub static FAILURE_DIAGNOSTICS: LazyLock<FailureDiagnostics> =
    LazyLock::new(FailureDiagnostics::default);

// Peer names, shards, correlation IDs and error codes belong to the sample,
// never the key: rotating any of them cannot bypass the emission bound.
#[derive(Clone, Copy)]
pub enum FailureKind {
    ExchangeFailed,
    ExchangeNoAnswer,
    ExchangeOverload,
    DaemonReceive,
}

impl FailureKind {
    fn index(self) -> usize {
        match self {
            Self::ExchangeFailed => 0,
            Self::ExchangeNoAnswer => 1,
            Self::ExchangeOverload => 2,
            Self::DaemonReceive => 3,
        }
    }
}

struct Entry {
    started: Instant,
    suppressed: u64,
    sample: String,
}

#[derive(Default)]
struct Limiter {
    windows: [Option<Entry>; MAX_KEYS],
}

impl Limiter {
    fn decide(&mut self, kind: FailureKind, now: Instant, line: impl FnOnce() -> String) -> Vec<String> {
        let mut lines = Vec::new();
        // Like ancestry diagnostics, flush expired suppression counts on the
        // next event, even if it belongs to another key. No timer or I/O task.
        for slot in &mut self.windows {
            if slot.as_ref().is_some_and(|entry| now.duration_since(entry.started) >= WINDOW) {
                let entry = slot.take().expect("expired diagnostic window exists");
                if entry.suppressed != 0 {
                    lines.push(format!("{} event=suppression-summary suppressed={}", entry.sample, entry.suppressed));
                }
            }
        }
        let slot = &mut self.windows[kind.index()];
        if let Some(entry) = slot {
            entry.suppressed = entry.suppressed.saturating_add(1);
        } else {
            let sample = line();
            lines.push(format!("{sample} suppressed=0"));
            *slot = Some(Entry { started: now, suppressed: 0, sample });
        }
        lines
    }
}

#[derive(Default)]
pub struct FailureDiagnostics(Mutex<Limiter>);

impl FailureDiagnostics {
    pub fn record(&self, kind: FailureKind, line: impl FnOnce() -> String) {
        self.record_to(kind, line, &mut std::io::stderr());
    }

    pub fn record_to(&self, kind: FailureKind, line: impl FnOnce() -> String, output: &mut impl std::io::Write) {
        let lines = match self.0.lock() {
            Ok(mut limiter) => limiter.decide(kind, Instant::now(), line),
            // Diagnostic lock poisoning must not affect a replication answer.
            // Do not fall back to unlimited stderr on this path.
            Err(_) => return,
        };
        // Never hold the limiter guard during a potentially blocking write.
        for line in lines { let _ = writeln!(output, "{line}"); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_lines_are_rate_bounded_with_summaries_and_fixed_key_storage() {
        let mut limiter = Limiter::default();
        let now = Instant::now();
        let mut lines = limiter.decide(FailureKind::ExchangeOverload, now, || "first".into());
        for _ in 1..100 {
            lines.extend(limiter.decide(FailureKind::ExchangeOverload, now, || panic!("suppressed records must not be formatted")));
        }
        assert_eq!(lines, ["first suppressed=0"]);
        let next = limiter.decide(FailureKind::DaemonReceive, now + WINDOW, || "daemon".into());
        assert_eq!(next, ["first event=suppression-summary suppressed=99", "daemon suppressed=0"]);
        for kind in [FailureKind::ExchangeFailed, FailureKind::ExchangeNoAnswer,
            FailureKind::ExchangeOverload, FailureKind::DaemonReceive] {
            for _ in 0..100 {
                limiter.decide(kind, now + WINDOW, || "sample".into());
            }
        }
        assert_eq!(limiter.windows.len(), MAX_KEYS);
        assert_eq!(limiter.windows.iter().flatten().count(), MAX_KEYS);
    }
}
