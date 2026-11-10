//! Worker authority uses process generations, not transferable same-uid secrets.
//! Same-UID service-manager takeover and ptrace of the trusted worker are not covered.
use super::*;
use axum::{body::Body, extract::Request, middleware::{Next, from_fn}, response::IntoResponse};
use std::{sync::{Arc, LazyLock, Mutex}, time::{Duration, Instant}};
use crate::config::WorkerIngressGenerationCheck;

type Observe = WorkerAuthority;

use fractal_api_core::api::{WorkerAuthority, authorize_worker as authorize, worker_route};

const OBSERVATION_WINDOW: Duration = Duration::from_secs(60);
static OBSERVATION_LIMITER: LazyLock<Mutex<ObservationLimiter>> =
    LazyLock::new(|| Mutex::new(ObservationLimiter::default()));

// One fixed key for all routes: arbitrary replication paths must not increase
// either the diagnostic volume or the limiter's memory usage.
#[derive(Default)]
struct ObservationLimiter {
    started: Option<Instant>,
    suppressed: u64,
}

impl ObservationLimiter {
    fn decide(&mut self, now: Instant) -> Option<u64> {
        if self.started.is_some_and(|start| now.duration_since(start) < OBSERVATION_WINDOW) {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.started = Some(now);
        Some(std::mem::take(&mut self.suppressed))
    }
}

fn record_observation(path: &str, now: Instant, limiter: &Mutex<ObservationLimiter>,
    output: &mut impl std::io::Write,
) {
    // Recover diagnostic state after poisoning rather than bypassing the cap.
    // Neither a diagnostic lock nor a failed sink changes admission.
    let suppressed = limiter.lock().unwrap_or_else(std::sync::PoisonError::into_inner).decide(now);
    // As in ancestry diagnostics, release the guard before touching the sink.
    if let Some(suppressed) = suppressed {
        let _ = writeln!(output,
            "fractal: worker authority observation would refuse {path} suppressed={suppressed}");
    }
}

fn protect(app: Router, observe: Observe, mode: WorkerIngressGenerationCheck) -> Router {
    app.layer(from_fn(move |request: Request<Body>, next: Next| {
        let observe = observe.clone();
        async move {
            if worker_route(request.uri().path()) {
                let peer = request.extensions().get::<LocalPeer>().cloned();
                let admitted = tokio::task::spawn_blocking(move || authorize(peer.as_ref(), &*observe)).await;
                if !matches!(admitted, Ok(Ok(()))) {
                    if mode == WorkerIngressGenerationCheck::Observe {
                        record_observation(request.uri().path(), Instant::now(),
                            &OBSERVATION_LIMITER, &mut std::io::stderr());
                        return next.run(request).await;
                    }
                    return (axum::http::StatusCode::FORBIDDEN,
                        axum::Json(serde_json::json!({"code":"worker-authentication-required",
                            "message":"this route requires the authenticated replication worker"}))).into_response();
                }
            }
            next.run(request).await
        }
    }))
}

/// Capture an explicit foreground worker before listening; service daemons cannot enroll one.
/// Same-UID supervisor takeover and ptrace of the trusted worker remain host trust assumptions.
pub fn worker_observer(pid: Option<u32>) -> anyhow::Result<Observe> {
    match pid {
        Some(pid) => {
            anyhow::ensure!(!crate::service::daemon_is_service_managed()?,
                "service-managed daemons cannot enroll a foreground worker");
            let generation = fractal_runtime::PeerProcessIdentity::capture_pid(pid)?;
            Ok(Arc::new(move || { generation.validate()?; Ok(generation.pid()) }))
        }
        None => Ok(Arc::new(crate::service::replication_worker_pid)),
    }
}

/// Observe by default; explicit enforcement refuses before body decoding.
pub fn worker_authenticated(app: Router, observe: Observe, mode: WorkerIngressGenerationCheck) -> Router {
    protect(app, observe, mode)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tower::ServiceExt as _;

    #[test]
    fn worker_observation_volume_is_independent_of_path_cardinality() {
        let now = Instant::now();
        let limiter = Mutex::new(ObservationLimiter::default());
        let mut output = Vec::new();
        for index in 0..10_000 {
            record_observation(&format!("/v1/internal/replication/{index}"), now,
                &limiter, &mut output);
        }
        assert_eq!(String::from_utf8(output.clone()).unwrap(),
            "fractal: worker authority observation would refuse /v1/internal/replication/0 suppressed=0\n");
        record_observation("/v1/internal/seat-snapshot",
            now + OBSERVATION_WINDOW - Duration::from_nanos(1), &limiter, &mut output);
        record_observation("/v1/internal/seat-freshness",
            now + OBSERVATION_WINDOW, &limiter, &mut output);
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.lines().count(), 2);
        assert!(output.ends_with("/v1/internal/seat-freshness suppressed=10000\n"));
        assert_eq!(limiter.lock().unwrap().suppressed, 0);
    }

    #[test]
    fn worker_observation_counts_saturate_and_reset_after_emission() {
        let now = Instant::now();
        let mut limiter = ObservationLimiter { started: Some(now), suppressed: u64::MAX };
        assert_eq!(limiter.decide(now), None);
        assert_eq!(limiter.decide(now + OBSERVATION_WINDOW), Some(u64::MAX));
        assert_eq!(limiter.decide(now + OBSERVATION_WINDOW * 2), Some(0));
    }

    #[test]
    fn worker_observation_output_releases_limiter_lock() {
        struct Writer<'a>(&'a Mutex<ObservationLimiter>);
        impl std::io::Write for Writer<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                assert!(self.0.try_lock().is_ok());
                Err(std::io::Error::other(format!("sink unavailable: {} bytes", bytes.len())))
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let limiter = Mutex::new(ObservationLimiter::default());
        let now = Instant::now();
        record_observation("/v1/internal/seat-snapshot", now, &limiter, &mut Writer(&limiter));
        assert_eq!(limiter.lock().unwrap().decide(now), None);
    }

    fn process(pid: u32) -> LocalPeer {
        LocalPeer { pid, ancestor: None,
            peer_process: Some(fractal_runtime::PeerProcessIdentity::capture_pid(pid).unwrap()) }
    }

    #[test]
    fn worker_authority_requires_generation_even_when_ingress_only_observes() {
        let peer = process(std::process::id());
        let observe = || Ok(peer.pid);
        assert!(authorize(Some(&peer), &observe).is_ok());
        let mut missing = peer.clone();
        missing.peer_process = None;
        assert!(authorize(Some(&missing), &observe).is_err());
        let mut seat = peer.clone();
        seat.ancestor = Some("agent/example".into());
        assert!(authorize(Some(&seat), &observe).is_err());
        let mut wrong_pid = peer.clone();
        wrong_pid.pid += 1;
        assert!(authorize(Some(&wrong_pid), &|| Ok(wrong_pid.pid)).is_err());
        assert!(authorize(Some(&peer), &|| Ok(0)).is_err());
    }

    #[test]
    fn worker_ingress_restart_reobserves_generation_and_refuses_predecessor() {
        let original = process(std::process::id());
        let mut replacement = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let restarted = process(replacement.id());
        let observed = AtomicU32::new(original.pid);
        let observe = || Ok(observed.load(Ordering::SeqCst));
        assert!(authorize(Some(&original), &observe).is_ok());
        assert!(authorize(Some(&restarted), &observe).is_err());
        observed.store(restarted.pid, Ordering::SeqCst);
        assert!(authorize(Some(&restarted), &observe).is_ok());
        assert!(authorize(Some(&original), &observe).is_err());
        replacement.kill().unwrap();
        replacement.wait().unwrap();
        assert!(authorize(Some(&restarted), &observe).is_err());
        assert!(authorize(None, &observe).is_err());
        assert!(authorize(Some(&original), &|| anyhow::bail!("supervisor unavailable")).is_err());
        assert!(!worker_route("/v1/internal/fleet/invites"));
        assert!(worker_route("/v1/internal/seat-snapshot"));
        assert!(worker_route(crate::peer::CLIENT_READ_FORWARD_PATH));
    }

    #[tokio::test]
    async fn worker_ingress_refuses_unbound_before_body_and_accepts_pinned_worker() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("worker.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        // Darwin LOCAL_PEERPID is available on accepted connections, not socketpair.
        let (client, accepted) = tokio::join!(
            tokio::net::UnixStream::connect(&socket), listener.accept(),
        );
        let _client = client.unwrap();
        let (connector, _) = accepted.unwrap();
        let peer = LocalPeer { pid: std::process::id(), ancestor: None,
            peer_process: Some(local_peer_process(&connector).unwrap()) };
        let observed = Arc::new(AtomicU32::new(0));
        let current = observed.clone();
        let app = protect(Router::new().route("/v1/internal/replication/receive",
            axum::routing::post(|_: axum::body::Bytes| async { "accepted" })),
            Arc::new(move || Ok(current.load(Ordering::SeqCst))), WorkerIngressGenerationCheck::Enforce);
        let request = |body| {
            let mut request = Request::post("/v1/internal/replication/receive").body(body).unwrap();
            request.extensions_mut().insert(peer.clone());
            request
        };
        let unread = Body::from_stream(futures_util::stream::poll_fn::<
            Result<axum::body::Bytes, std::io::Error>, _>(|_| panic!("unauthenticated body polled")));
        assert_eq!(app.clone().oneshot(request(unread)).await.unwrap().status(), 403);
        observed.store(peer.pid, Ordering::SeqCst);
        assert_eq!(app.clone().oneshot(request(Body::from("[]"))).await.unwrap().status(), 200);
        observed.store(0, Ordering::SeqCst);
        assert_eq!(app.oneshot(request(Body::empty())).await.unwrap().status(), 403);
    }

    #[tokio::test]
    async fn worker_ingress_observe_admits_unbound_callers_on_every_route_class() {
        for path in ["/v1/internal/replication/receive", "/v1/internal/seat-freshness",
            "/v1/internal/seat-freshness/reads", "/v1/internal/seat-snapshot", crate::peer::CLIENT_READ_FORWARD_PATH] {
            let app = protect(Router::new().route(path, axum::routing::post(|| async { "accepted" })),
                Arc::new(|| anyhow::bail!("supervisor unavailable")), WorkerIngressGenerationCheck::Observe);
            // Admission stays unchanged even after repeated observations exhaust
            // the shared diagnostic window.
            for _ in 0..3 {
                let response = app.clone().oneshot(Request::post(path).body(Body::empty()).unwrap()).await.unwrap();
                assert_eq!(response.status(), 200, "{path}");
            }
        }
    }
}
