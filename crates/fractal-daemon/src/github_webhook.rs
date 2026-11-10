//! The GitHub webhook receiver: one loopback route, `POST /fractal/github-webhook`, that verifies
//! each delivery's signature and asks the observers of the repository it names to poll now.
//! A public proxy forwards the route; the secret's custody is the operator's.
use std::{io::Read as _, net::SocketAddr, path::{Path, PathBuf}, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use fractal_kernel::github_webhook::{self as webhook, Outcome};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use tokio::{net::TcpListener, sync::{Notify, watch}};

use crate::store::Store;

#[derive(Clone)]
struct Receiver {
    secret_file: Arc<PathBuf>,
    store: Arc<Store>,
    notify: Arc<Notify>,
    event_notify: watch::Sender<u64>,
}

/// A bound receiver, ready to serve.
pub struct Bound {
    listener: TcpListener,
    router: Router,
}

impl Bound {
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        axum::serve(self.listener, self.router).await?;
        Ok(())
    }
}

/// Bind the receiver `[github].webhook_listen` configures, or `None` when it is unset. The
/// secret file is not read here: it is read for every delivery, so it may appear later.
pub async fn bind(
    config: &crate::config::GithubConfig,
    store: Arc<Store>,
    notify: Arc<Notify>,
    event_notify: watch::Sender<u64>,
) -> anyhow::Result<Option<Bound>> {
    let (Some(listen), Some(secret_file)) = (config.webhook_listen, config.webhook_secret_file.clone()) else {
        return Ok(None);
    };
    let listener = TcpListener::bind(listen).await
        .map_err(|error| anyhow::anyhow!("bind the GitHub webhook receiver at {listen}: {error}"))?;
    let receiver = Receiver { secret_file: Arc::new(secret_file), store, notify, event_notify };
    let router = Router::new()
        .route(webhook::ROUTE, post(deliver).fallback(not_found))
        .fallback(not_found)
        .with_state(receiver);
    webhook::listening(&listener.local_addr()?.to_string());
    Ok(Some(Bound { listener, router }))
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn deliver(State(receiver): State<Receiver>, headers: HeaderMap, body: Body) -> Response {
    let event = header(&headers, "x-github-event");
    let (outcome, response) = receive(&receiver, &headers, event, body).await;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    webhook::record(outcome, now);
    crate::otel::github_webhook_delivery(webhook::event_label(event.unwrap_or("")), outcome.as_str());
    response
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok()).filter(|value| !value.is_empty())
}

async fn receive(receiver: &Receiver, headers: &HeaderMap, event: Option<&str>, body: Body) -> (Outcome, Response) {
    let reply = |outcome, status: StatusCode| (outcome, status.into_response());
    let Ok(body) = axum::body::to_bytes(body, webhook::BODY_LIMIT).await else {
        return reply(Outcome::Invalid, StatusCode::PAYLOAD_TOO_LARGE);
    };
    let secret = match read_secret(&receiver.secret_file) {
        Ok(secret) => secret,
        Err(error) => {
            tracing::warn!(error = format!("{error:#}"), "GitHub webhook secret unavailable; refusing the delivery");
            return reply(Outcome::Unavailable, StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    if !signed(&secret, header(headers, "x-hub-signature-256"), &body) {
        return reply(Outcome::Unauthorized, StatusCode::UNAUTHORIZED);
    }
    let (Some(event), Some(delivery)) = (event, header(headers, "x-github-delivery")
        .filter(|delivery| delivery.len() <= 128 && delivery.bytes().all(|byte| byte.is_ascii_graphic())))
    else {
        return reply(Outcome::Invalid, StatusCode::BAD_REQUEST);
    };
    if event == "ping" {
        return reply(Outcome::Ignored, StatusCode::OK);
    }
    if matches!(webhook::event_label(event), "other" | "ping") {
        return reply(Outcome::Ignored, StatusCode::NO_CONTENT);
    }
    let scope = match serde_json::from_slice(&body).map_err(|_| "the payload is not JSON")
        .and_then(|payload| webhook::scope(event, &payload))
    {
        Ok(Some(scope)) => scope,
        Ok(None) => return reply(Outcome::Ignored, StatusCode::NO_CONTENT),
        Err(_) => return reply(Outcome::Invalid, StatusCode::BAD_REQUEST),
    };
    let store = receiver.store.clone();
    let delivery = delivery.to_owned();
    let refreshed = store.connection.write_workers()
        .spawn_blocking({
            let store = store.clone();
            move || store.request_github_webhook_refresh(&delivery, &scope)
        })
        .await;
    match refreshed {
        Ok(Ok(refreshed)) => {
            if refreshed > 0 {
                receiver.notify.notify_one();
                receiver.event_notify.send_modify(|generation| *generation = generation.saturating_add(1));
            }
            (Outcome::Refreshed, (StatusCode::ACCEPTED, Json(serde_json::json!({"refreshed": refreshed}))).into_response())
        }
        Ok(Err(error)) => {
            tracing::warn!(code = error.code, message = error.message, "GitHub webhook refresh failed");
            reply(Outcome::Unavailable, StatusCode::SERVICE_UNAVAILABLE)
        }
        Err(error) => {
            tracing::warn!(%error, "GitHub webhook refresh stopped");
            reply(Outcome::Unavailable, StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// The secret, from an owner-only regular file read again for every delivery.
fn read_secret(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut secret = String::new();
    crate::config::open_token_file(path)?
        .read_to_string(&mut secret)
        .map_err(|error| anyhow::anyhow!("read {}: {error}", path.display()))?;
    let secret = secret.trim_end_matches(['\r', '\n']);
    anyhow::ensure!(!secret.is_empty(), "{} is empty", path.display());
    Ok(secret.as_bytes().to_vec())
}

/// Whether `signature` is `sha256=` and the HMAC-SHA256 of `body` under `secret`, compared in
/// constant time.
fn signed(secret: &[u8], signature: Option<&str>, body: &Bytes) -> bool {
    let Some(expected) = signature
        .and_then(|signature| signature.strip_prefix("sha256="))
        .and_then(|digest| hex::decode(digest).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use reqwest::StatusCode;
    use serde_json::{Value, json};

    use super::*;

    const SECRET: &str = "fixture-webhook-secret";
    const OBSERVERS: &str = r#"version 2
resource "garden" { kind "vcs.repository" }
observer "garden" { resource "resource/garden"; provider "github.repository"; locator "Acme/Garden"; field "pull_requests" }
resource "garden-main" { kind "vcs.ref" }
observer "garden-main" { resource "resource/garden-main"; provider "github.ref"; locator "acme/garden@main"; field "head" }
"#;

    struct Fixture {
        _root: tempfile::TempDir,
        secret_file: PathBuf,
        store: Arc<Store>,
        base: String,
        client: reqwest::Client,
    }

    impl Fixture {
        async fn start() -> Self {
            let root = tempfile::tempdir().unwrap();
            let secret_file = root.path().join("secret");
            let store = Arc::new(Store::open_memory("node").unwrap());
            let intent = crate::graph::parse_internal_intent(OBSERVERS, "node").unwrap();
            let planned = store
                .mission(&intent, crate::model::IntentInput { kdl: OBSERVERS.into(), source_name: None })
                .unwrap();
            store.apply(&intent, &planned.subject_tokens, "webhook-observers").unwrap();
            let config = crate::config::GithubConfig {
                webhook_listen: Some("127.0.0.1:0".parse().unwrap()),
                webhook_secret_file: Some(secret_file.clone()),
                ..Default::default()
            };
            let bound = bind(&config, store.clone(), Arc::new(Notify::new()), watch::channel(0).0)
                .await.unwrap().unwrap();
            let base = format!("http://{}", bound.local_addr().unwrap());
            tokio::spawn(bound.serve());
            Self { _root: root, secret_file, store, base, client: reqwest::Client::new() }
        }

        fn write_secret(&self, mode: u32) {
            std::fs::write(&self.secret_file, format!("{SECRET}\n")).unwrap();
            std::fs::set_permissions(&self.secret_file, std::fs::Permissions::from_mode(mode)).unwrap();
        }

        async fn send(&self, event: &str, delivery: &str, payload: &Value, secret: &str) -> (StatusCode, String) {
            let body = payload.to_string();
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(body.as_bytes());
            let response = self.client.post(format!("{}{}", self.base, webhook::ROUTE))
                .header("x-github-event", event)
                .header("x-github-delivery", delivery)
                .header("x-hub-signature-256", format!("sha256={}", hex::encode(mac.finalize().into_bytes())))
                .header("content-type", "application/json")
                .body(body)
                .send().await.unwrap();
            (response.status(), response.text().await.unwrap())
        }

        fn requests(&self, observer: &str) -> Vec<Value> {
            self.store.graph.claims_for(observer, Some("observer.refresh-requested")).unwrap()
                .into_iter().map(|claim| claim.body["fields"].clone()).collect()
        }
    }

    fn push(branch: &str) -> Value {
        json!({"ref": format!("refs/heads/{branch}"), "repository": {"full_name": "ACME/garden"}})
    }

    #[tokio::test]
    async fn only_a_signed_delivery_with_a_present_secret_refreshes() {
        let fixture = Fixture::start().await;
        let (status, _) = fixture.send("push", "d-1", &push("main"), SECRET).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "an absent secret file never accepts a delivery");
        fixture.write_secret(0o644);
        let (status, _) = fixture.send("push", "d-1", &push("main"), SECRET).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "a secret others can read is refused");
        fixture.write_secret(0o600);
        let (status, _) = fixture.send("push", "d-1", &push("main"), "another-secret").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let unsigned = fixture.client.post(format!("{}{}", fixture.base, webhook::ROUTE))
            .header("x-github-event", "push").header("x-github-delivery", "d-1")
            .body(push("main").to_string()).send().await.unwrap();
        assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
        assert!(fixture.requests("observer/garden").is_empty());

        let (status, body) = fixture.send("push", "d-1", &push("main"), SECRET).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({"refreshed": 2}));
        for observer in ["observer/garden", "observer/garden-main"] {
            let revision = fixture.store.selected_desired_revision(observer).unwrap().unwrap();
            assert_eq!(fixture.requests(observer), [json!({
                "attempt": webhook::attempt("d-1", observer),
                "revision": revision,
            })]);
        }
        // A redelivery asks nothing more; another delivery asks again, as a poll may be running.
        let (status, body) = fixture.send("push", "d-1", &push("main"), SECRET).await;
        assert_eq!((status, body.as_str()), (StatusCode::ACCEPTED, r#"{"refreshed":0}"#));
        let opened = json!({"action": "opened", "repository": {"full_name": "acme/garden"}});
        let (status, body) = fixture.send("pull_request", "d-2", &opened, SECRET).await;
        assert_eq!((status, body.as_str()), (StatusCode::ACCEPTED, r#"{"refreshed":1}"#));
        assert_eq!(fixture.requests("observer/garden").len(), 2);
    }

    #[tokio::test]
    async fn the_receiver_filters_events_and_serves_only_its_route() {
        let fixture = Fixture::start().await;
        fixture.write_secret(0o600);
        let repository = json!({"full_name": "acme/garden"});
        assert_eq!(fixture.send("ping", "d-ping", &json!({"zen": "Keep it logically awesome."}), SECRET).await.0, StatusCode::OK);
        assert_eq!(fixture.send("star", "d-star", &json!({"repository": repository}), SECRET).await.0, StatusCode::NO_CONTENT);
        assert_eq!(fixture.send("check_run", "d-run", &json!({"action": "created", "repository": repository}), SECRET).await.0, StatusCode::NO_CONTENT);
        assert_eq!(fixture.send("issues", "d-bad", &json!({"action": "opened"}), SECRET).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(fixture.send("push", "d-other", &json!({"ref": "refs/heads/main", "repository": {"full_name": "acme/greenhouse"}}), SECRET).await.0, StatusCode::ACCEPTED);
        assert!(fixture.requests("observer/garden").is_empty());
        let (status, body) = fixture.send("check_run", "d-done", &json!({"action": "completed", "repository": repository}), SECRET).await;
        assert_eq!((status, body.as_str()), (StatusCode::ACCEPTED, r#"{"refreshed":1}"#));
        assert!(fixture.requests("observer/garden-main").is_empty(), "a check run moves no branch");

        let oversized = fixture.client.post(format!("{}{}", fixture.base, webhook::ROUTE))
            .body(vec![b' '; webhook::BODY_LIMIT + 1]).send().await.unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        for (method, path) in [(reqwest::Method::GET, webhook::ROUTE), (reqwest::Method::POST, "/"), (reqwest::Method::POST, "/v1/apply")] {
            let response = fixture.client.request(method.clone(), format!("{}{path}", fixture.base)).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
        }
    }

    #[test]
    fn the_signature_is_hmac_sha256_of_the_exact_body() {
        let body = Bytes::from_static(b"{\"zen\":\"x\"}");
        let mut mac = Hmac::<Sha256>::new_from_slice(b"key").unwrap();
        mac.update(&body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(signed(b"key", Some(&signature), &body));
        assert!(!signed(b"other", Some(&signature), &body));
        assert!(!signed(b"key", Some(&signature), &Bytes::from_static(b"{\"zen\":\"y\"}")));
        assert!(!signed(b"key", Some(signature.trim_start_matches("sha256=")), &body));
        assert!(!signed(b"key", Some("sha256=not-hex"), &body));
        assert!(!signed(b"key", None, &body));
    }
}
