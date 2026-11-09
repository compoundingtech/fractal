use super::{AncestryUnavailableCause, AncestryUnavailableReason, LocalPeer};
use axum::{body::Body, http::{Method, Request, StatusCode}};
use std::{collections::HashMap, sync::{Arc, LazyLock, Mutex}, time::{Duration, Instant}};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_KEYS: usize = 1024;
const MAX_LINES: u32 = 100;
static LIMITER: LazyLock<Mutex<Limiter>> = LazyLock::new(|| Mutex::new(Limiter::default()));

// Only the successful server-side worker verification inserts this private fact.
#[derive(Clone)]
pub(super) struct VerifiedWorkerFallback;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Decision {
    Refused(u16),
    VerifiedWorker,
    ReadOnly,
    Admitted,
}

impl Decision {
    fn of(status: Option<StatusCode>, worker: bool, method: &Method) -> Self {
        if let Some(status) = status { return Self::Refused(status.as_u16()); }
        if worker { return Self::VerifiedWorker; }
        if matches!(*method, Method::GET | Method::HEAD) { return Self::ReadOnly; }
        Self::Admitted
    }

    fn level(self) -> &'static str {
        match self { Self::VerifiedWorker | Self::ReadOnly => "debug", _ => "warn" }
    }
}

// PID and arbitrary error text belong to the record, never the aggregation key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CauseKind {
    reason: &'static str,
    os_error: Option<i32>,
    metadata: Option<&'static str>,
}

impl CauseKind {
    fn of(cause: &AncestryUnavailableCause) -> Self {
        Self {
            reason: cause.reason_code(),
            os_error: match cause.reason {
                AncestryUnavailableReason::ProcessReadFailed { os_error, .. } => os_error,
                _ => None,
            },
            metadata: match cause.reason {
                AncestryUnavailableReason::ProcessMetadataInvalid(field) => Some(field),
                _ => None,
            },
        }
    }
}

#[derive(Clone)]
pub(super) struct RequestDiagnostic {
    cause: Arc<AncestryUnavailableCause>,
    connector_pid: Option<u32>,
    connector_generation: Option<u64>,
    method: Method,
    route: String,
}

impl RequestDiagnostic {
    pub(super) fn capture(request: &Request<Body>) -> Option<Self> {
        let cause = request.extensions().get::<Arc<AncestryUnavailableCause>>()?.clone();
        let peer = request.extensions().get::<LocalPeer>();
        Some(Self {
            cause,
            connector_pid: peer.map(|peer| peer.pid),
            connector_generation: peer.and_then(|peer| peer.peer_process.as_ref())
                .and_then(fractal_runtime::PeerProcessIdentity::diagnostic_start_token),
            method: request.method().clone(),
            // Never log the query, headers, body, command arguments or environment.
            route: request.uri().path().to_owned(),
        })
    }

    fn line(&self, decision: Decision, suppressed: u64) -> String {
        let worker = (decision == Decision::VerifiedWorker)
            .then_some((self.connector_pid, self.connector_generation));
        format!("st3: local caller ancestry unavailable level={} connector_pid={:?} connector_generation={:?} method={} route={:?} decision={decision:?} worker={worker:?} pid={:?} reason={} detail={:?} suppressed={suppressed}",
            decision.level(), self.connector_pid, self.connector_generation, self.method,
            self.route, self.cause.pid, self.cause.reason_code(), self.cause.reason)
    }

    pub(super) fn record(self, status: Option<StatusCode>, worker: bool) {
        self.record_to(status, worker, Instant::now(), &LIMITER, &mut std::io::stderr());
    }

    fn record_to(&self, status: Option<StatusCode>, worker: bool, now: Instant,
        limiter: &Mutex<Limiter>, output: &mut impl std::io::Write,
    ) {
        let decision = Decision::of(status, worker, &self.method);
        // A poisoned diagnostic lock cannot change authorization or hide a refusal.
        let lines = match limiter.lock() {
            Ok(mut limiter) => limiter.decide(self, decision, now),
            Err(_) => vec![format!("{} dropped=0", self.line(decision, 0))],
        };
        // The guard is gone before any potentially blocking sink operation.
        write_lines(lines, output);
    }
}

type Key = (CauseKind, Decision);
struct Entry {
    started: Instant,
    suppressed: u64,
    sample: RequestDiagnostic,
}

#[derive(Default)]
struct Limiter {
    windows: HashMap<Key, Entry>,
    global_started: Option<Instant>,
    lines: u32,
    dropped: u64,
}

impl Limiter {
    fn summary(key: &Key, entry: &Entry, lines: &mut Vec<String>) {
        if entry.suppressed != 0 {
            lines.push(format!("{} event=suppression-summary dropped=0",
                entry.sample.line(key.1, entry.suppressed)));
        }
    }

    fn flush_dropped(&mut self, lines: &mut Vec<String>) {
        if self.dropped != 0 {
            lines.push(format!(
                "st3: local caller ancestry unavailable level=warn event=global-cap-summary dropped={}", self.dropped));
            self.dropped = 0;
        }
    }

    fn decide(&mut self, record: &RequestDiagnostic, decision: Decision, now: Instant) -> Vec<String> {
        let mut lines = Vec::new();
        let key = (CauseKind::of(&record.cause), decision);
        // Flush every expired key on the next event, even if that event has a
        // different cause. The current key carries its count on its next record.
        self.windows.retain(|other, entry| {
            if *other != key && now.duration_since(entry.started) >= WINDOW {
                Self::summary(other, entry, &mut lines);
                false
            } else { true }
        });
        if self.global_started.is_none_or(|start| now.duration_since(start) >= WINDOW) {
            self.flush_dropped(&mut lines);
            self.global_started = Some(now);
            self.lines = 0;
        }
        let suppressed = if let Some(entry) = self.windows.get_mut(&key) {
            if now.duration_since(entry.started) < WINDOW {
                entry.suppressed = entry.suppressed.saturating_add(1);
                return lines;
            }
            let count = entry.suppressed;
            entry.started = now;
            entry.suppressed = 0;
            entry.sample = record.clone();
            count
        } else {
            if self.windows.len() >= MAX_KEYS {
                let oldest = self.windows.iter().min_by_key(|(_, entry)| entry.started)
                    .map(|(oldest, _)| *oldest);
                if let Some(oldest) = oldest
                    && let Some(entry) = self.windows.remove(&oldest) {
                    Self::summary(&oldest, &entry, &mut lines);
                }
            }
            self.windows.insert(key, Entry { started: now, suppressed: 0, sample: record.clone() });
            0
        };
        // First refusals and count summaries are never hidden by the global cap.
        // Only non-refusal event records are capped; their loss is counted.
        if self.lines >= MAX_LINES && !matches!(decision, Decision::Refused(_)) {
            if suppressed != 0 {
                lines.push(format!("{} event=suppression-summary dropped=0", record.line(decision, suppressed)));
            }
            self.dropped = self.dropped.saturating_add(1);
            return lines;
        }
        lines.push(format!("{} dropped={}", record.line(decision, suppressed), self.dropped));
        self.dropped = 0;
        self.lines = self.lines.saturating_add(1);
        lines
    }

    fn flush_pending(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for (key, entry) in self.windows.drain() { Self::summary(&key, &entry, &mut lines); }
        self.flush_dropped(&mut lines);
        lines
    }

    #[cfg(test)]
    fn emit(&mut self, record: &RequestDiagnostic, decision: Decision, now: Instant,
        output: &mut impl std::io::Write,
    ) {
        write_lines(self.decide(record, decision, now), output);
    }

    #[cfg(test)]
    fn flush(&mut self, output: &mut impl std::io::Write) {
        write_lines(self.flush_pending(), output);
    }
}

fn write_lines(lines: Vec<String>, output: &mut impl std::io::Write) {
    for line in lines { let _ = writeln!(output, "{line}"); }
}

fn flush_to(limiter: &Mutex<Limiter>, output: &mut impl std::io::Write) {
    let lines = match limiter.lock() {
        Ok(mut limiter) => limiter.flush_pending(),
        Err(_) => Vec::new(),
    };
    write_lines(lines, output);
}

pub(super) fn flush() {
    flush_to(&LIMITER, &mut std::io::stderr());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(reason: AncestryUnavailableReason) -> RequestDiagnostic {
        RequestDiagnostic { cause: Arc::new(AncestryUnavailableCause { pid: Some(42), reason }),
            connector_pid: Some(7), connector_generation: Some(123),
            method: Method::POST, route: "/v1/intent/apply".into() }
    }

    struct LockCheckingWriter<'a> {
        limiter: &'a Mutex<Limiter>,
        output: Vec<u8>,
    }

    impl std::io::Write for LockCheckingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert!(self.limiter.try_lock().is_ok(), "output must never hold the limiter lock");
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    #[test]
    fn request_and_shutdown_output_run_without_the_limiter_lock() {
        let now = Instant::now();
        let limiter = Mutex::new(Limiter::default());
        let record = record(AncestryUnavailableReason::WitnessMissing);
        let mut writer = LockCheckingWriter { limiter: &limiter, output: Vec::new() };
        for _ in 0..3 {
            record.record_to(Some(StatusCode::BAD_REQUEST), false, now, &limiter, &mut writer);
        }
        flush_to(&limiter, &mut writer);
        let text = String::from_utf8(writer.output).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("suppressed=2 event=suppression-summary"));
    }

    fn read_failure(pid: u32, errno: i32, text: &str) -> RequestDiagnostic {
        let mut record = record(AncestryUnavailableReason::ProcessReadFailed {
            operation: "proc-environ-read", os_error: Some(errno), error: text.into(),
        });
        Arc::make_mut(&mut record.cause).pid = Some(pid);
        record
    }

    #[test]
    fn real_emit_path_suppresses_identical_requests_and_reports_count() {
        let now = Instant::now();
        let limiter = Mutex::new(Limiter::default());
        let record = record(AncestryUnavailableReason::WitnessMissing);
        let mut output = Vec::new();
        for _ in 0..100 {
            record.record_to(Some(StatusCode::BAD_REQUEST), false, now, &limiter, &mut output);
        }
        assert_eq!(String::from_utf8(output.clone()).unwrap().lines().count(), 1);
        record.record_to(Some(StatusCode::BAD_REQUEST), false, now + WINDOW, &limiter, &mut output);
        let all = String::from_utf8(output).unwrap();
        assert_eq!(all.lines().count(), 2);
        assert!(all.lines().nth(1).unwrap().contains("suppressed=99"));
    }

    #[test]
    fn key_uses_cause_kind_not_pid_or_error_text_and_decisions_are_independent() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let mut output = Vec::new();
        let first = read_failure(42, libc::EACCES, "first error text");
        let same = read_failure(99, libc::EACCES, "different error text");
        let other = read_failure(99, libc::ENOENT, "different cause kind");
        limiter.emit(&first, Decision::ReadOnly, now, &mut output);
        limiter.emit(&same, Decision::ReadOnly, now, &mut output);
        limiter.emit(&other, Decision::ReadOnly, now, &mut output);
        limiter.emit(&same, Decision::Refused(422), now, &mut output);
        assert_eq!(String::from_utf8(output.clone()).unwrap().lines().count(), 3);
        limiter.emit(&same, Decision::ReadOnly, now + WINDOW, &mut output);
        assert!(String::from_utf8(output).unwrap().lines().last().unwrap().contains("suppressed=1"));
    }

    #[test]
    fn saturation_evicts_oldest_and_suppresses_new_key() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let mut output = Vec::new();
        for errno in 0..1024 {
            limiter.emit(&read_failure(42, errno, "error"), Decision::ReadOnly,
                now + Duration::from_millis(errno as u64), &mut output);
        }
        let newest = read_failure(42, 1024, "new key");
        limiter.emit(&newest, Decision::Refused(422), now + Duration::from_secs(2), &mut output);
        let length = output.len();
        limiter.emit(&newest, Decision::Refused(422), now + Duration::from_secs(2), &mut output);
        assert_eq!(output.len(), length);
        assert_eq!(limiter.windows.len(), MAX_KEYS);
        assert!(!limiter.windows.contains_key(&(CauseKind::of(&read_failure(42, 0, "error").cause), Decision::ReadOnly)));
    }

    #[test]
    fn expiry_and_shutdown_flush_pending_counts() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let mut output = Vec::new();
        let first = record(AncestryUnavailableReason::WitnessMissing);
        let other = record(AncestryUnavailableReason::AncestorGenerationChanged);
        for _ in 0..3 { limiter.emit(&first, Decision::ReadOnly, now, &mut output); }
        limiter.emit(&other, Decision::ReadOnly, now + WINDOW, &mut output);
        assert_eq!(limiter.windows.len(), 1);
        let text = String::from_utf8(output.clone()).unwrap();
        assert!(text.contains("suppressed=2 event=suppression-summary"));
        limiter.emit(&other, Decision::ReadOnly, now + WINDOW, &mut output);
        limiter.flush(&mut output);
        assert!(limiter.windows.is_empty());
        assert!(String::from_utf8(output).unwrap().contains("suppressed=1 event=suppression-summary"));
    }

    #[test]
    fn global_cap_counts_drops_but_never_hides_first_refusal() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let mut output = Vec::new();
        for errno in 0..105 {
            limiter.emit(&read_failure(42, errno, "error"), Decision::ReadOnly, now, &mut output);
        }
        assert_eq!(String::from_utf8(output.clone()).unwrap().lines().count(), MAX_LINES as usize);
        assert_eq!(limiter.dropped, 5);
        limiter.emit(&record(AncestryUnavailableReason::WitnessMissing), Decision::Refused(422), now, &mut output);
        let text = String::from_utf8(output).unwrap();
        let last = text.lines().last().unwrap();
        assert!(last.contains("level=warn"));
        assert!(last.contains("dropped=5"));
    }

    #[test]
    fn shutdown_flushes_global_dropped_count() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let mut output = Vec::new();
        for errno in 0..105 {
            limiter.emit(&read_failure(42, errno, "error"), Decision::ReadOnly, now, &mut output);
        }
        limiter.flush(&mut output);
        assert!(String::from_utf8(output).unwrap().contains("event=global-cap-summary dropped=5"));
    }

    #[test]
    fn record_correlates_connector_cause_route_worker_and_decision() {
        let record = record(AncestryUnavailableReason::WitnessMissing);
        let line = record.line(Decision::VerifiedWorker, 9);
        for field in ["level=debug", "connector_pid=Some(7)", "connector_generation=Some(123)",
            "method=POST", "route=\"/v1/intent/apply\"", "decision=VerifiedWorker",
            "worker=Some((Some(7), Some(123)))", "pid=Some(42)", "reason=witness-missing", "suppressed=9"] {
            assert!(line.contains(field), "missing {field}: {line}");
        }
        assert!(record.line(Decision::Refused(422), 0).contains("level=warn"));
    }

    #[test]
    fn refusals_override_read_and_worker_downgrades() {
        for method in [Method::GET, Method::HEAD, Method::POST] {
            for worker in [false, true] {
                let decision = Decision::of(Some(StatusCode::UNPROCESSABLE_ENTITY), worker, &method);
                assert_eq!(decision, Decision::Refused(422));
                assert_eq!(decision.level(), "warn");
            }
        }
        assert_eq!(Decision::of(None, false, &Method::POST).level(), "warn");
        assert_eq!(Decision::of(None, false, &Method::GET).level(), "debug");
        assert_eq!(Decision::of(None, false, &Method::HEAD).level(), "debug");
        assert_eq!(Decision::of(None, true, &Method::POST), Decision::VerifiedWorker);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn capture_uses_connector_generation_not_failed_ancestor_or_client_fields() {
        let pid = std::process::id();
        let peer = fractal_runtime::PeerProcessIdentity::capture_pid(pid).unwrap();
        let generation = peer.diagnostic_start_token();
        let mut request = Request::builder().method(Method::GET)
            .uri("/v1/health?environment=must-not-log").header("x-connector-pid", "999")
            .body(Body::empty()).unwrap();
        request.extensions_mut().insert(record(AncestryUnavailableReason::WitnessMissing).cause);
        request.extensions_mut().insert(LocalPeer { pid, peer_process: Some(peer), ancestor: None });
        let record = RequestDiagnostic::capture(&request).unwrap();
        assert_eq!(record.connector_pid, Some(pid));
        assert_eq!(record.connector_generation, generation);
        assert_eq!(record.cause.pid, Some(42));
        assert!(!record.line(Decision::ReadOnly, 0).contains("must-not-log"));
    }

    #[test]
    fn client_header_cannot_claim_verified_worker_fallback() {
        let mut request = Request::builder().method(Method::POST)
            .header("x-verified-worker-fallback", "true").body(Body::empty()).unwrap();
        request.extensions_mut().insert(record(AncestryUnavailableReason::WitnessMissing).cause);
        let record = RequestDiagnostic::capture(&request).unwrap();
        let worker = request.extensions().get::<VerifiedWorkerFallback>().is_some();
        assert!(!worker);
        assert_eq!(Decision::of(None, worker, &record.method), Decision::Admitted);
        assert_eq!(Decision::of(Some(StatusCode::UNPROCESSABLE_ENTITY), worker, &record.method).level(), "warn");
    }
}
