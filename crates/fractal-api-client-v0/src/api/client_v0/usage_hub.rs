//! `GET /v1/client/usage/quota`: the operator's usage hub quota, relayed to clients.
//! The body stays the hub's own v3 document; this route checks only what makes it that document.
use super::*;
use fractal_client::quota::{QuotaFailure, UsageHubQuota};
use std::time::{Duration, Instant};

const QUOTA_PATH: &str = "/api/v1/quota";
const SCHEMA_VERSION: u64 = 3;
/// The web client refuses more than 2 MiB, and this body still gains the client envelope.
const MAX_BODY_BYTES: usize = 2_000_000;

// Hold the lock across the request: every client shares successes AND failures for one window.
#[derive(Default)]
struct Source {
    cached: Option<(String, u64, Instant, UsageHubQuota)>,
}
impl Source {
    async fn read(&mut self, base: &str, refresh_seconds: u64) -> UsageHubQuota {
        if let Some((key, interval, until, value)) = &self.cached
            && key == base
            && *interval == refresh_seconds
            && *until > Instant::now()
        {
            return value.clone();
        }
        let value = match fetch(base).await {
            Ok(snapshot) => UsageHubQuota::Available { snapshot, refresh_seconds },
            Err(error) => UsageHubQuota::Unavailable { refresh_seconds, reason:
                if error.downcast_ref::<reqwest::Error>().is_some() { QuotaFailure::Transport }
                else { QuotaFailure::InvalidSnapshot } },
        };
        self.cached = Some((base.into(), refresh_seconds, Instant::now() + Duration::from_secs(refresh_seconds), value.clone()));
        value
    }
}

pub(super) async fn read(Extension(session): Extension<ClientSession>) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let config = tokio::task::spawn_blocking(crate::config::reload_daemon_usage_hub)
        .await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
    let value = if let Some(config) = config {
        if !(1..=3600).contains(&config.refresh_seconds) {
            return Err(validation("usage_hub.refresh_seconds must be 1..3600"));
        }
        if quota_url(&config.url).is_err() {
            return Err(validation("usage_hub.url must be an https or loopback http base URL with no credentials, path or query"));
        }
        static SOURCE: std::sync::LazyLock<tokio::sync::Mutex<Source>> = std::sync::LazyLock::new(Default::default);
        SOURCE.lock().await.read(&config.url, config.refresh_seconds).await
    } else { UsageHubQuota::Disabled };
    serde_json::to_value(value).map(Json).map_err(ApiError::internal)
}

/// The one URL this route ever requests, from the operator's base URL alone.
fn quota_url(base: &str) -> anyhow::Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(base)?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    anyhow::ensure!(url.scheme() == "https" || (url.scheme() == "http" && loopback), "usage hub URL must be https or loopback http");
    anyhow::ensure!(url.host_str().is_some() && url.username().is_empty() && url.password().is_none(), "usage hub URL carries credentials or has no host");
    anyhow::ensure!(url.path() == "/" && url.query().is_none() && url.fragment().is_none(), "usage hub URL must be a base URL");
    url.set_path(QUOTA_PATH);
    Ok(url)
}

async fn fetch(base: &str) -> anyhow::Result<Value> {
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy()
        .connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(5)).build()?;
    let mut response = client.get(quota_url(base)?).header(reqwest::header::ACCEPT, "application/json")
        .send().await?.error_for_status()?;
    // Redirects aren't followed, and a redirect's own body is not the hub's answer.
    anyhow::ensure!(response.status().is_success(), "usage hub redirected");
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(bytes.len() + chunk.len() <= MAX_BODY_BYTES, "usage hub response too large");
        bytes.extend_from_slice(&chunk);
    }
    let snapshot: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(snapshot["schemaVersion"].as_u64() == Some(SCHEMA_VERSION), "unsupported usage hub schema");
    chrono::DateTime::parse_from_rfc3339(snapshot["generatedAt"].as_str().unwrap_or_default())?;
    anyhow::ensure!(snapshot["accounts"].is_array(), "usage hub quota has no accounts");
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[tokio::test]
    async fn endpoint_requires_projection_scope() {
        use tower::ServiceExt as _;
        let session = ClientSession::for_tests("device/example", "person/operator", "paired");
        let app = Router::new().route("/v1/client/usage/quota", get(read)).layer(Extension(session));
        let request = axum::http::Request::builder().uri("/v1/client/usage/quota").body(axum::body::Body::empty()).unwrap();
        assert_eq!(app.oneshot(request).await.unwrap().status(), axum::http::StatusCode::FORBIDDEN);
    }

    fn document() -> Value {
        json!({"schemaVersion":3,"generatedAt":"2026-01-01T00:00:00Z","freshnessHorizonSeconds":900,
            "coverage":{"reportedAccounts":0,"unreportedAccountIds":[],"disabledAccountIds":[]},"accounts":[],"producerAddition":true})
    }
    async fn hub(status: axum::http::StatusCode, body: Value) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let app = Router::new().route(QUOTA_PATH, get(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            async move { (status, Json(body)) }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, count, task)
    }

    #[test]
    fn requests_only_the_fixed_path_of_a_plain_base_url() {
        assert_eq!(quota_url("https://hub.example.test").unwrap().as_str(), "https://hub.example.test/api/v1/quota");
        assert_eq!(quota_url("https://hub.example.test/").unwrap().as_str(), "https://hub.example.test/api/v1/quota");
        assert_eq!(quota_url("http://127.0.0.1:41033").unwrap().as_str(), "http://127.0.0.1:41033/api/v1/quota");
        for refused in ["http://hub.example.test", "file:///quota", "https://user:secret@hub.example.test", "https://hub.example.test/other",
            "https://hub.example.test/?url=http://elsewhere.test", "https://hub.example.test/#fragment", "not a url"] {
            assert!(quota_url(refused).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn relays_the_hub_document_with_its_additive_fields() {
        let (base, _, task) = hub(axum::http::StatusCode::OK, document()).await;
        let value = Source::default().read(&base, 30).await;
        assert_eq!(value, UsageHubQuota::Available { snapshot: document(), refresh_seconds: 30 });
        task.abort();
    }

    #[tokio::test]
    async fn coalesces_one_window_and_refreshes_after_it() {
        let (base, count, task) = hub(axum::http::StatusCode::OK, document()).await;
        let mut source = Source::default();
        source.read(&base, 30).await;
        source.read(&base, 30).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        source.cached.as_mut().unwrap().2 = Instant::now();
        source.read(&base, 30).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn reports_failures_without_an_old_document() {
        for (status, body, reason) in [
            (axum::http::StatusCode::OK, json!({"schemaVersion":2,"generatedAt":"2026-01-01T00:00:00Z","accounts":[]}), QuotaFailure::InvalidSnapshot),
            (axum::http::StatusCode::OK, json!({"schemaVersion":3,"generatedAt":"yesterday","accounts":[]}), QuotaFailure::InvalidSnapshot),
            (axum::http::StatusCode::OK, json!({"schemaVersion":3,"generatedAt":"2026-01-01T00:00:00Z"}), QuotaFailure::InvalidSnapshot),
            (axum::http::StatusCode::BAD_GATEWAY, document(), QuotaFailure::Transport),
            (axum::http::StatusCode::FOUND, document(), QuotaFailure::InvalidSnapshot),
        ] {
            let (base, _, task) = hub(status, body).await;
            assert_eq!(Source::default().read(&base, 30).await, UsageHubQuota::Unavailable { refresh_seconds: 30, reason });
            task.abort();
        }
    }

    #[tokio::test]
    async fn refuses_an_oversized_document() {
        let (base, _, task) = hub(axum::http::StatusCode::OK, json!({"padding": "x".repeat(MAX_BODY_BYTES)})).await;
        assert!(fetch(&base).await.is_err());
        task.abort();
    }
}
