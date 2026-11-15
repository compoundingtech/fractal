//! Worker authority uses process generations, not transferable same-uid secrets.
//! Same-UID service-manager takeover and ptrace of the trusted worker are not covered.
use super::*;
use axum::{body::Body, extract::Request, middleware::{Next, from_fn}, response::IntoResponse};
use std::sync::Arc;
use crate::config::WorkerIngressGenerationCheck;

type Observe = WorkerAuthority;

use fractal_api_core::api::{WorkerAuthority, authorize_worker as authorize, worker_route};

fn protect(app: Router, observe: Observe, mode: WorkerIngressGenerationCheck) -> Router {
    app.layer(from_fn(move |request: Request<Body>, next: Next| {
        let observe = observe.clone();
        async move {
            if worker_route(request.uri().path()) {
                let peer = request.extensions().get::<LocalPeer>().cloned();
                let admitted = tokio::task::spawn_blocking(move || authorize(peer.as_ref(), &*observe)).await;
                if !matches!(admitted, Ok(Ok(()))) {
                    if mode == WorkerIngressGenerationCheck::Observe {
                        eprintln!("fractal: worker authority observation would refuse {}", request.uri().path());
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
        None => Ok(pin_supervised(crate::service::replication_worker_pid)),
    }
}

/// A supervisor replaces its worker only after that process ends, so one observation answers
/// for as long as the observed generation validates. Requests then cost a process-table read
/// instead of service-manager commands; the lock admits one re-observation at a time.
fn pin_supervised(observe: impl Fn() -> anyhow::Result<u32> + Send + Sync + 'static) -> Observe {
    let pinned = std::sync::Mutex::new(None::<fractal_runtime::PeerProcessIdentity>);
    Arc::new(move || {
        let mut pinned = pinned.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(worker) = pinned.as_ref().filter(|worker| worker.validate().is_ok()) {
            return Ok(worker.pid());
        }
        *pinned = None;
        let worker = fractal_runtime::PeerProcessIdentity::capture_pid(observe()?)?;
        let pid = worker.pid();
        *pinned = Some(worker);
        Ok(pid)
    })
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

    #[test]
    fn supervised_worker_is_observed_once_per_generation() {
        let mut first = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let mut second = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let named = Arc::new(AtomicU32::new(first.id()));
        let observations = Arc::new(AtomicU32::new(0));
        let observe = pin_supervised({
            let (named, observations) = (named.clone(), observations.clone());
            move || {
                observations.fetch_add(1, Ordering::SeqCst);
                match named.load(Ordering::SeqCst) { 0 => anyhow::bail!("supervisor unavailable"), pid => Ok(pid) }
            }
        });
        let worker = process(first.id());
        for _ in 0..3 {
            assert!(authorize(Some(&worker), &*observe).is_ok());
        }
        assert!(authorize(Some(&process(second.id())), &*observe).is_err());
        assert_eq!(observations.load(Ordering::SeqCst), 1);

        // The supervisor names a successor only once its predecessor has ended.
        named.store(second.id(), Ordering::SeqCst);
        first.kill().unwrap();
        first.wait().unwrap();
        assert!(authorize(Some(&process(second.id())), &*observe).is_ok());
        assert_eq!(observations.load(Ordering::SeqCst), 2);

        named.store(0, Ordering::SeqCst);
        second.kill().unwrap();
        second.wait().unwrap();
        assert!(observe().is_err());
        assert!(observe().is_err());
        assert_eq!(observations.load(Ordering::SeqCst), 4, "a failed observation pins nothing");
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
            let response = app.oneshot(Request::post(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), 200, "{path}");
        }
    }
}
