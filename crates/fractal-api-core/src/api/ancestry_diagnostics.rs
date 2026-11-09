use super::{AncestryUnavailableCause, LocalPeer};
use axum::{body::Body, http::{Method, Request, StatusCode}};
use std::{collections::HashMap, sync::{Arc, LazyLock, Mutex}, time::{Duration, Instant}};

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
        static LIMITER: LazyLock<Mutex<Limiter>> = LazyLock::new(|| Mutex::new(Limiter::default()));
        self.record_to(status, worker, Instant::now(), &LIMITER, &mut std::io::stderr());
    }

    fn record_to(
        &self,
        status: Option<StatusCode>,
        worker: bool,
        now: Instant,
        limiter: &Mutex<Limiter>,
        output: &mut impl std::io::Write,
    ) {
        let decision = Decision::of(status, worker, &self.method);
        // Poisoning must not hide the first refusal or affect the authorization result.
        let suppressed = limiter.lock().map_or(Some(0), |mut limiter|
            limiter.observe((*self.cause).clone(), decision, now));
        if let Some(suppressed) = suppressed {
            let _ = writeln!(output, "{}", self.line(decision, suppressed));
        }
    }
}

#[derive(Default)]
struct Limiter {
    windows: HashMap<(AncestryUnavailableCause, Decision), (Instant, u64)>,
}

impl Limiter {
    fn observe(&mut self, cause: AncestryUnavailableCause, decision: Decision, now: Instant) -> Option<u64> {
        let key = (cause, decision);
        if let Some((last, suppressed)) = self.windows.get_mut(&key) {
            if now.duration_since(*last) < Duration::from_secs(60) {
                *suppressed = suppressed.saturating_add(1);
                return None;
            }
            let count = *suppressed;
            *last = now;
            *suppressed = 0;
            return Some(count);
        }
        // Reclaim expired windows before admitting a new cause/decision pair.
        // Look up the requested key first so its next report retains its count.
        self.windows.retain(|_, (last, _)| now.duration_since(*last) < Duration::from_secs(60));
        if self.windows.len() >= 1024 {
            let oldest = self.windows.iter().min_by_key(|(_, (last, _))| *last)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest { self.windows.remove(&oldest); }
        }
        self.windows.insert(key, (now, 0));
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AncestryUnavailableReason;

    fn cause(reason: AncestryUnavailableReason) -> AncestryUnavailableCause {
        AncestryUnavailableCause { pid: Some(42), reason }
    }

    #[test]
    fn repeated_cause_and_decision_report_suppressed_count_without_hiding_other_failures() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        let first = cause(AncestryUnavailableReason::WitnessMissing);
        let other = cause(AncestryUnavailableReason::AncestorGenerationChanged);
        assert_eq!(limiter.observe(first.clone(), Decision::ReadOnly, now), Some(0));
        assert_eq!(limiter.observe(first.clone(), Decision::ReadOnly, now), None);
        assert_eq!(limiter.observe(first.clone(), Decision::ReadOnly, now), None);
        assert_eq!(limiter.observe(other, Decision::ReadOnly, now), Some(0));
        assert_eq!(limiter.observe(first.clone(), Decision::Refused(400), now), Some(0));
        assert_eq!(limiter.observe(first, Decision::ReadOnly, now + Duration::from_secs(60)), Some(2));
    }

    #[test]
    fn saturation_evicts_oldest_and_still_suppresses_new_keys() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        for pid in 0..1024 {
            let cause = AncestryUnavailableCause { pid: Some(pid), reason: AncestryUnavailableReason::WitnessMissing };
            assert_eq!(limiter.observe(cause, Decision::Refused(400),
                now + Duration::from_millis(u64::from(pid))), Some(0));
        }
        let newest = AncestryUnavailableCause { pid: Some(1024), reason: AncestryUnavailableReason::WitnessMissing };
        let later = now + Duration::from_secs(2);
        assert_eq!(limiter.observe(newest.clone(), Decision::Refused(400), later), Some(0));
        assert_eq!(limiter.observe(newest.clone(), Decision::Refused(400), later), None);
        assert_eq!(limiter.windows.len(), 1024);
        assert!(!limiter.windows.contains_key(&(AncestryUnavailableCause {
            pid: Some(0), reason: AncestryUnavailableReason::WitnessMissing,
        }, Decision::Refused(400))));
        assert!(limiter.windows.contains_key(&(newest, Decision::Refused(400))));
    }

    #[test]
    fn inserting_new_key_reclaims_expired_windows() {
        let now = Instant::now();
        let mut limiter = Limiter::default();
        for pid in 0..1024 {
            let cause = AncestryUnavailableCause { pid: Some(pid), reason: AncestryUnavailableReason::WitnessMissing };
            assert_eq!(limiter.observe(cause, Decision::ReadOnly, now), Some(0));
        }
        let next = cause(AncestryUnavailableReason::AncestorGenerationChanged);
        let later = now + Duration::from_secs(60);
        assert_eq!(limiter.observe(next.clone(), Decision::ReadOnly, later), Some(0));
        assert_eq!(limiter.windows.len(), 1);
        assert_eq!(limiter.observe(next, Decision::ReadOnly, later), None);
    }

    #[test]
    fn real_emit_path_writes_one_line_for_identical_requests_then_reports_count() {
        let now = Instant::now();
        let limiter = Mutex::new(Limiter::default());
        let record = RequestDiagnostic {
            cause: Arc::new(cause(AncestryUnavailableReason::WitnessMissing)),
            connector_pid: Some(7), connector_generation: Some(123),
            method: Method::POST, route: "/v1/intent/apply".into(),
        };
        let mut output = Vec::new();
        for _ in 0..100 {
            record.record_to(Some(StatusCode::BAD_REQUEST), false, now, &limiter, &mut output);
        }
        let first = String::from_utf8(output.clone()).unwrap();
        assert_eq!(first.lines().count(), 1);
        assert!(first.contains("level=warn"));
        record.record_to(Some(StatusCode::BAD_REQUEST), false,
            now + Duration::from_secs(60), &limiter, &mut output);
        let all = String::from_utf8(output).unwrap();
        assert_eq!(all.lines().count(), 2);
        assert!(all.lines().nth(1).unwrap().contains("suppressed=99"));
    }

    #[test]
    fn record_correlates_connector_cause_route_and_decision() {
        let record = RequestDiagnostic {
            cause: Arc::new(cause(AncestryUnavailableReason::WitnessMissing)),
            connector_pid: Some(7), connector_generation: Some(123),
            method: Method::POST, route: "/v1/intent/apply".into(),
        };
        let line = record.line(Decision::VerifiedWorker, 9);
        for field in ["level=debug", "connector_pid=Some(7)", "connector_generation=Some(123)",
            "method=POST", "route=\"/v1/intent/apply\"", "decision=VerifiedWorker",
            "worker=Some((Some(7), Some(123)))", "pid=Some(42)", "reason=witness-missing", "suppressed=9"] {
            assert!(line.contains(field), "missing {field}: {line}");
        }
        assert!(record.line(Decision::Refused(400), 0).contains("level=warn"));
    }

    #[test]
    fn refusals_override_read_and_worker_downgrades() {
        for method in [Method::GET, Method::HEAD, Method::POST] {
            for worker in [false, true] {
                let decision = Decision::of(Some(StatusCode::BAD_REQUEST), worker, &method);
                assert_eq!(decision, Decision::Refused(400));
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
            .uri("/v1/health?environment=must-not-log")
            .header("x-connector-pid", "999")
            .body(Body::empty()).unwrap();
        request.extensions_mut().insert(Arc::new(cause(AncestryUnavailableReason::WitnessMissing)));
        request.extensions_mut().insert(LocalPeer { pid, peer_process: Some(peer), ancestor: None });
        let record = RequestDiagnostic::capture(&request).unwrap();
        assert_eq!(record.connector_pid, Some(pid));
        assert_eq!(record.connector_generation, generation);
        assert_eq!(record.cause.pid, Some(42));
        let line = record.line(Decision::ReadOnly, 0);
        assert!(!line.contains("must-not-log"));
        assert_eq!(record.route, "/v1/health");
    }

    #[test]
    fn client_header_cannot_claim_verified_worker_fallback() {
        let mut request = Request::builder().method(Method::POST)
            .header("x-verified-worker-fallback", "true").body(Body::empty()).unwrap();
        request.extensions_mut().insert(Arc::new(cause(AncestryUnavailableReason::WitnessMissing)));
        let record = RequestDiagnostic::capture(&request).unwrap();
        let worker = request.extensions().get::<VerifiedWorkerFallback>().is_some();
        assert!(!worker);
        assert_eq!(Decision::of(None, worker, &record.method), Decision::Admitted);
        assert_eq!(Decision::of(Some(StatusCode::BAD_REQUEST), worker, &record.method).level(), "warn");
    }
}
