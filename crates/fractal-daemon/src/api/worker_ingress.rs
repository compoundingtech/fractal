//! Worker authority uses process generations, not transferable same-uid secrets.
//! Same-UID service-manager takeover and ptrace of the trusted worker are not covered.
use super::*;
use axum::{body::Body, extract::Request, middleware::{Next, from_fn}, response::IntoResponse};
use std::sync::Arc;

type Observe = Arc<dyn Fn() -> anyhow::Result<u32> + Send + Sync>;

fn worker_route(path: &str) -> bool {
    path.starts_with("/v1/internal/replication/")
        || matches!(path, "/v1/internal/seat-freshness" | "/v1/internal/seat-snapshot")
        || path == crate::peer::CLIENT_READ_FORWARD_PATH
}

fn authorize(peer: Option<&LocalPeer>, observe: &dyn Fn() -> anyhow::Result<u32>) -> anyhow::Result<()> {
    let peer = peer.ok_or_else(|| anyhow::anyhow!("worker socket credentials missing"))?;
    anyhow::ensure!(peer.ancestor.is_none(), "seat callers cannot act as the worker");
    let generation = peer.peer_process.as_ref()
        .ok_or_else(|| anyhow::anyhow!("worker process generation unavailable"))?;
    generation.validate()?;
    anyhow::ensure!(generation.pid() == peer.pid && observe()? == peer.pid,
        "socket connector is not the supervised replication worker");
    generation.validate()
}

fn protect(app: Router, observe: Observe) -> Router {
    app.layer(from_fn(move |request: Request<Body>, next: Next| {
        let observe = observe.clone();
        async move {
            if worker_route(request.uri().path()) {
                if cfg!(target_os = "macos") {
                    eprintln!("fractal: worker authority unverified-platform {}", request.uri().path());
                    return next.run(request).await;
                }
                let peer = request.extensions().get::<LocalPeer>().cloned();
                let admitted = tokio::task::spawn_blocking(move || authorize(peer.as_ref(), &*observe)).await;
                if !matches!(admitted, Ok(Ok(()))) {
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
            #[cfg(not(target_os = "linux"))]
            return Ok(Arc::new(move || Ok(pid)));
            #[cfg(target_os = "linux")]
            {
            let generation = fractal_runtime::PeerProcessIdentity::capture_pid(pid)?;
            Ok(Arc::new(move || { generation.validate()?; Ok(generation.pid()) }))
            }
        }
        None => Ok(Arc::new(crate::service::replication_worker_pid)),
    }
}

/// On Linux refuse before body decoding; re-observation admits restarts and revokes predecessors.
pub fn worker_authenticated(app: Router, observe: Observe) -> Router {
    protect(app, observe)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tower::ServiceExt as _;

    fn process(pid: u32) -> LocalPeer {
        LocalPeer { pid, ancestor: None,
            peer_process: Some(fractal_runtime::PeerProcessIdentity::capture_pid(pid).unwrap()) }
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
        let (connector, _) = tokio::net::UnixStream::pair().unwrap();
        let peer = LocalPeer { pid: std::process::id(), ancestor: None,
            peer_process: Some(local_peer_process(&connector).unwrap()) };
        let observed = Arc::new(AtomicU32::new(0));
        let current = observed.clone();
        let app = protect(Router::new().route("/v1/internal/replication/receive",
            axum::routing::post(|_: axum::body::Bytes| async { "accepted" })),
            Arc::new(move || Ok(current.load(Ordering::SeqCst))));
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
}
