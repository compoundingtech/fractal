use super::*;
use fractal_client::quota::{QuotaFailure, QuotaSnapshot, QuotaSource};
use std::time::{Duration, Instant};

// Hold the lock across the request: every client shares successes AND failures for one window.
#[derive(Default)]
struct Source {
    cached: Option<(String, u64, Instant, QuotaSource)>,
}
impl Source {
    async fn read(&mut self, url: &str, refresh_seconds: u64) -> QuotaSource {
        if let Some((key, interval, until, value)) = &self.cached
            && key == url
            && *interval == refresh_seconds
            && *until > Instant::now()
        {
            return value.clone();
        }
        let value = match fetch(url).await {
            Ok(snapshot) => QuotaSource::Available { snapshot, refresh_seconds },
            Err(error) => QuotaSource::Unavailable { refresh_seconds, reason:
                if error.downcast_ref::<reqwest::Error>().is_some() { QuotaFailure::Transport }
                else { QuotaFailure::InvalidSnapshot } },
        };
        self.cached = Some((url.into(), refresh_seconds, Instant::now() + Duration::from_secs(refresh_seconds), value.clone()));
        value
    }
}

pub(super) async fn read(Extension(session): Extension<ClientSession>) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let config = tokio::task::spawn_blocking(crate::config::reload_daemon_quota)
        .await.map_err(ApiError::internal)?.map_err(ApiError::internal)?;
    let value = if let Some(config) = config {
        if !(1..=3600).contains(&config.refresh_seconds) {
            return Err(validation("quota_source.refresh_seconds must be 1..3600"));
        }
        static SOURCE: std::sync::LazyLock<tokio::sync::Mutex<Source>> = std::sync::LazyLock::new(Default::default);
        SOURCE.lock().await.read(&config.url, config.refresh_seconds).await
    } else { QuotaSource::Disabled };
    serde_json::to_value(value).map(Json).map_err(ApiError::internal)
}

async fn fetch(url: &str) -> anyhow::Result<QuotaSnapshot> {
    let url = reqwest::Url::parse(url)?;
    anyhow::ensure!(matches!(url.scheme(), "http" | "https") && url.username().is_empty() && url.password().is_none(), "invalid quota source URL");
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(5)).build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(bytes.len() + chunk.len() <= 1_048_576, "quota source response too large");
        bytes.extend_from_slice(&chunk);
    }
    let snapshot: QuotaSnapshot = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(snapshot.schema_version == 1 && snapshot.freshness_horizon_seconds > 0, "unsupported quota schema");
    chrono::DateTime::parse_from_rfc3339(&snapshot.generated_at)?;
    let mut accounts = BTreeSet::new();
    for account in &snapshot.accounts {
        anyhow::ensure!(!account.id.is_empty() && accounts.insert(&account.id), "invalid quota account identity");
        chrono::DateTime::parse_from_rfc3339(&account.observed_at)?;
        let mut windows = BTreeSet::new();
        for window in &account.windows {
            anyhow::ensure!(!window.id.is_empty() && windows.insert(&window.id), "invalid quota window identity");
            anyhow::ensure!(window.used_percent.is_none_or(|value| value.is_finite() && value >= 0.0), "invalid used percentage");
            if let Some(at) = &window.resets_at { chrono::DateTime::parse_from_rfc3339(at)?; }
        }
        anyhow::ensure!(account.governing_window_id.as_ref().is_none_or(|id| windows.contains(id)), "unknown governing window");
        anyhow::ensure!(account.history.len() <= 4, "too many history series");
        let mut series = BTreeSet::new();
        for history in &account.history {
            anyhow::ensure!(!history.id.is_empty() && series.insert(&history.id) && !history.unit.is_empty(), "invalid history identity or unit");
            anyhow::ensure!((1..=128).contains(&history.points.len()), "history requires 1..128 points");
            let mut previous = None;
            for point in &history.points {
                let at = chrono::DateTime::parse_from_rfc3339(&point.at)?;
                anyhow::ensure!(previous.is_none_or(|previous| at > previous), "history timestamps must increase");
                anyhow::ensure!(point.value.is_none_or(|value| value.is_finite() && value >= 0.0), "invalid history value");
                previous = Some(at);
            }
        }
    }
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
        let app = Router::new().route("/v1/client/quota", get(read)).layer(Extension(session));
        let request = axum::http::Request::builder().uri("/v1/client/quota?url=http://invalid")
            .body(axum::body::Body::empty()).unwrap();
        assert_eq!(app.oneshot(request).await.unwrap().status(), axum::http::StatusCode::FORBIDDEN);
    }
    async fn source(body: Value) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let app = Router::new().route("/quota", get(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            async move { Json(body) }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/quota", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, count, task)
    }
    fn fixture() -> Value {
        json!({"schema_version":1,"generated_at":"2026-01-01T00:00:00Z","freshness_horizon_seconds":60,
            "accounts":[{"id":"sample","provider":"example","label":"Team","observed_at":"2026-01-01T00:00:00Z",
            "governing_window_id":"weekly","windows":[{"id":"weekly","label":"Week","used_percent":72.0,"resets_at":null}]}]})
    }
    #[test]
    fn fixture_matches_published_schema() {
        let schema: Value = serde_json::from_str(include_str!("../../../../../docs/st3/client-v0/schemas/quota-source-v1.schema.json")).unwrap();
        jsonschema::validator_for(&schema).unwrap().validate(&fixture()).unwrap();
    }
    #[tokio::test]
    async fn history_is_optional_bounded_and_preserved() {
        let schema: Value = serde_json::from_str(include_str!("../../../../../docs/st3/client-v0/schemas/quota-source-v1.schema.json")).unwrap();
        let mut body = fixture();
        let history = json!([{"id":"daily","label":"Daily spend","unit":"USD","points":[
            {"at":"2025-12-29T00:00:00Z","value":0},{"at":"2025-12-30T00:00:00Z","value":null},
            {"at":"2025-12-31T00:00:00Z","value":12.5}]}]);
        body["accounts"][0]["history"] = history.clone();
        jsonschema::validator_for(&schema).unwrap().validate(&body).unwrap();
        let (url, _, task) = source(body.clone()).await;
        let value = Source::default().read(&url, 30).await;
        assert_eq!(serde_json::to_value(value).unwrap()["snapshot"]["accounts"][0]["history"], history);
        task.abort();
        for invalid in 0..4 {
            let mut body = body.clone();
            match invalid {
                0 => body["accounts"][0]["history"][0]["points"] = json!(vec![history[0]["points"][0].clone(); 129]),
                1 => body["accounts"][0]["history"] = json!(vec![history[0].clone(); 5]),
                2 => body["accounts"][0]["history"][0]["points"][1]["at"] = json!("2025-12-28T00:00:00Z"),
                _ => body["accounts"][0]["history"][0]["points"][0]["value"] = json!(-1),
            }
            let (url, _, task) = source(body).await;
            assert!(fetch(&url).await.is_err()); task.abort();
        }
    }
    #[tokio::test]
    async fn coalesces_success_and_failure() {
        for body in [fixture(), json!({"schema_version":99})] {
            let valid = body["schema_version"] == 1;
            let (url, count, task) = source(body).await;
            let shared = Arc::new(tokio::sync::Mutex::new(Source::default()));
            let mut jobs = Vec::new();
            for _ in 0..8 {
                let shared = shared.clone(); let url = url.clone();
                jobs.push(tokio::spawn(async move { shared.lock().await.read(&url, 30).await }));
            }
            for job in jobs { assert_eq!(matches!(job.await.unwrap(), QuotaSource::Available { .. }), valid); }
            assert_eq!(count.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }
    #[tokio::test]
    async fn refreshes_expired_windows() {
        let (url, count, task) = source(fixture()).await;
        let mut source = Source::default();
        source.read(&url, 30).await;
        source.cached.as_mut().unwrap().2 = Instant::now();
        source.read(&url, 30).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        task.abort();
    }
    #[tokio::test]
    async fn rejects_bad_contracts() {
        for field in ["schema_version", "generated_at", "accounts"] {
            let mut body = fixture(); body[field] = json!(false);
            let (url, _, task) = source(body).await;
            assert!(fetch(&url).await.is_err()); task.abort();
        }
        assert!(fetch("file:///quota").await.is_err());
    }
}
