//! Read-only installation routing. The operator, not st3, resolves the host token root.
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
#[cfg(test)]
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use serde::Deserialize;
use crate::github_http::GithubAuth;

#[derive(Clone)]
pub struct ReadSource {
    pub root: PathBuf,
    pub owner: String,
    pub app: String,
    pub principal: String,
    pub token: Arc<String>,
    pub used_fallback: Arc<AtomicBool>,
}

#[derive(Deserialize)]
struct Sidecar {
    installation_id: u64,
    expires_at: chrono::DateTime<chrono::Utc>,
    permissions: BTreeMap<String, String>,
    repository_selection: String,
}

#[cfg(test)]
type Counts = BTreeMap<(String, String, String), u64>;
#[cfg(test)]
fn counters() -> &'static Mutex<Counts> {
    static COUNTERS: LazyLock<Mutex<Counts>> = LazyLock::new(Mutex::default);
    &COUNTERS
}

const FALLBACK_WARNING_INTERVAL: Duration = Duration::from_secs(60 * 60);
const MAX_FALLBACK_WARNING_KEYS: usize = 1024;

#[derive(Default)]
struct FallbackWarnings {
    owners: HashMap<String, HashMap<String, HashMap<String, WarningWindow>>>,
    keys: usize,
}

struct WarningWindow {
    emitted_at: Instant,
    suppressed: u64,
}

impl FallbackWarnings {
    /// `now` is injected so the hourly boundary can be tested without sleeping.
    /// Only new keys allocate; repeated selections borrow the existing strings.
    fn emission(&mut self, owner: &str, app: &str, reason: &str, now: Instant) -> Option<u64> {
        if let Some(window) = self.owners.get_mut(owner)
            .and_then(|apps| apps.get_mut(app))
            .and_then(|reasons| reasons.get_mut(reason))
        {
            if now.duration_since(window.emitted_at) < FALLBACK_WARNING_INTERVAL {
                window.suppressed = window.suppressed.saturating_add(1);
                return None;
            }
            window.emitted_at = now;
            return Some(std::mem::take(&mut window.suppressed));
        }
        // Expired keys lose their pending suppressed count when a new key arrives.
        // Keep active keys, but fail open to logging rather than grow past the cap.
        let mut keys = 0;
        self.owners.retain(|_, apps| {
            apps.retain(|_, reasons| {
                reasons.retain(|_, window| now.duration_since(window.emitted_at) < FALLBACK_WARNING_INTERVAL);
                keys += reasons.len();
                !reasons.is_empty()
            });
            !apps.is_empty()
        });
        self.keys = keys;
        if self.keys == MAX_FALLBACK_WARNING_KEYS {
            return Some(0);
        }
        self.owners.entry(owner.into()).or_default()
            .entry(app.into()).or_default()
            .insert(reason.into(), WarningWindow { emitted_at: now, suppressed: 0 });
        self.keys += 1;
        Some(0)
    }
}

pub fn fallback(owner: &str, app: &str, reason: &str) {
    crate::otel::github_auth_fallback(owner, app, reason);
    #[cfg(test)]
    {
        *counters().lock()
            .entry((owner.into(), app.into(), reason.into())).or_default() += 1;
    }
    static WARNINGS: LazyLock<std::sync::Mutex<FallbackWarnings>> = LazyLock::new(std::sync::Mutex::default);
    let suppressed_count = WARNINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .emission(owner, app, reason, Instant::now());
    if let Some(suppressed_count) = suppressed_count {
        tracing::warn!(owner, app, reason, suppressed_count, "GitHub installation read using transition PAT fallback");
    }
}

fn component(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub fn load(root: &Path, owner: &str, app: &str) -> std::result::Result<ReadSource, &'static str> {
    if !component(owner) || !matches!(app, "gh-ci" | "github-ledger") { return Err("unreadable"); }
    if !root.try_exists().map_err(|_| "unreadable")? { return Err("missing"); }
    let directory = root.join(app);
    let sidecar = directory.join(format!("{owner}.json"));
    let token_path = directory.join(format!("{owner}.token"));
    let has_sidecar = sidecar.try_exists().map_err(|_| "unreadable")?;
    let has_token = token_path.try_exists().map_err(|_| "unreadable")?;
    if !has_sidecar && !has_token { return Err("no-installation"); }
    let metadata = std::fs::read(&sidecar).map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" })?;
    let metadata: Sidecar = serde_json::from_slice(&metadata).map_err(|_| "unreadable")?;
    if metadata.expires_at <= chrono::Utc::now() { return Err("expired"); }
    if metadata.installation_id == 0 || metadata.permissions.is_empty() || !matches!(metadata.repository_selection.as_str(), "all" | "selected") { return Err("unreadable"); }
    if !has_token { return Err("missing"); }
    let mut file = crate::github_http::open_token_file(&token_path).map_err(|_| "unreadable")?;
    use std::os::unix::fs::PermissionsExt as _;
    if file.metadata().map_err(|_| "unreadable")?.permissions().mode() & 0o777 != 0o600 { return Err("unreadable"); }
    use std::io::Read as _;
    let mut token = String::new();
    file.read_to_string(&mut token).map_err(|_| "unreadable")?;
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control()) { return Err("unreadable"); }
    let principal = format!("app:{app}:{owner}:{}:{}:{}", metadata.installation_id, metadata.repository_selection,
        serde_json::to_string(&metadata.permissions).map_err(|_| "unreadable")?);
    Ok(ReadSource { root: root.into(), owner: owner.into(), app: app.into(), principal, token: Arc::new(token), used_fallback: Arc::new(AtomicBool::new(false)) })
}

pub fn select(root: Option<&Path>, owner: &str, app: &str, pat: &GithubAuth) -> GithubAuth {
    let Some(root) = root else { fallback(owner, app, "missing"); return pat.clone(); };
    match load(root, owner, app) {
        Ok(source) => GithubAuth::Read { source, pat: Box::new(pat.clone()) },
        Err(reason) => { fallback(owner, app, reason); pat.clone() }
    }
}

pub fn route(url: &reqwest::Url) -> Option<(&str, &'static str, bool)> {
    let mut segments = url.path().split_once("/repos/")?.1.split('/');
    let owner = segments.next()?;
    if !component(owner) { return None; }
    segments.next()?; // repository
    let resource = segments.next().unwrap_or("");
    let statuses = (resource == "commits" && segments.nth(1) == Some("status")) || resource == "statuses";
    let app = if resource == "issues" { "github-ledger" } else { "gh-ci" };
    Some((owner, app, statuses))
}

pub async fn send(request: reqwest::RequestBuilder, source: &ReadSource, pat: &GithubAuth) -> Result<(reqwest::Response, Option<ReadSource>)> {
    let retry = request.try_clone();
    let auth = GithubAuth::Local { token: source.token.clone(), cache: None };
    let response = crate::github_http::send_once(request, &auth).await?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED { return Ok((response, Some(source.clone()))); }
    let Some(retry) = retry else { return Ok((response, Some(source.clone()))); };
    // Reload once for atomic minter rotation. Never replay any mutation.
    let fallback_request = retry.try_clone();
    if let Ok(reloaded) = load(&source.root, &source.owner, &source.app) {
        let retry = if reloaded.principal != source.principal {
            source.used_fallback.store(true, Ordering::Relaxed);
            let (client, request) = retry.build_split();
            let mut request = request?;
            request.headers_mut().remove(reqwest::header::IF_NONE_MATCH);
            reqwest::RequestBuilder::from_parts(client, request)
        } else { retry };
        let auth = GithubAuth::Local { token: reloaded.token.clone(), cache: None };
        let response = crate::github_http::send_once(retry, &auth).await?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED { return Ok((response, Some(reloaded))); }
    }
    let Some(request) = fallback_request else { return Ok((response, Some(source.clone()))); };
    fallback(&source.owner, &source.app, "401");
    source.used_fallback.store(true, Ordering::Relaxed);
    // The app's ETag must not validate a potentially different PAT representation.
    let (client, request) = request.build_split();
    let mut request = request?;
    request.headers_mut().remove(reqwest::header::IF_NONE_MATCH);
    let builder = reqwest::RequestBuilder::from_parts(client, request);
    Ok((crate::github_http::send_once(builder, pat).await?, None))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{Router, extract::State, response::IntoResponse as _};
    use reqwest::StatusCode;
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt as _;

    pub(crate) fn install(root: &Path, owner: &str, app: &str, token: &str, expires: chrono::DateTime<chrono::Utc>) {
        let directory = root.join(app);
        std::fs::create_dir_all(&directory).unwrap();
        let token_path = directory.join(format!("{owner}.token"));
        std::fs::write(&token_path, token).unwrap();
        std::fs::set_permissions(token_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(directory.join(format!("{owner}.json")), serde_json::to_vec(&json!({
            "installation_id": 42, "expires_at": expires.to_rfc3339(),
            "permissions": {"metadata": "read", "contents": "read"},
            "repository_selection": "selected",
        })).unwrap()).unwrap();
    }

    type Handler = dyn Fn(&str, &str, &Value) -> (StatusCode, Value) + Send + Sync;
    pub(crate) struct Server {
        pub base: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server { fn drop(&mut self) { self.task.abort(); } }
    pub(crate) async fn server(handler: impl Fn(&str, &str, &Value) -> (StatusCode, Value) + Send + Sync + 'static) -> Server {
        async fn handle(State(handler): State<Arc<Handler>>, request: axum::extract::Request) -> axum::response::Response {
            let authorization = request.headers().get("authorization").unwrap().to_str().unwrap().to_owned();
            let path = request.uri().to_string();
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024).await.unwrap();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let (status, body) = handler(&authorization, &path, &body);
            (status, axum::Json(body)).into_response()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handler: Arc<Handler> = Arc::new(handler);
        let router = Router::new().fallback(handle).with_state(handler);
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Server { base, task }
    }

    fn count(owner: &str, app: &str, reason: &str) -> u64 {
        counters().lock().get(&(owner.into(), app.into(), reason.into())).copied().unwrap_or(0)
    }

    #[test]
    fn fallback_warning_limiter_counts_suppressed_per_key_and_hour() {
        let mut warnings = FallbackWarnings::default();
        let start = Instant::now();
        let within = start + FALLBACK_WARNING_INTERVAL - Duration::from_secs(1);
        let boundary = start + FALLBACK_WARNING_INTERVAL;
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", start), Some(0));
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", start), None);
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", within), None);
        // Each dimension has its own window and counter.
        assert_eq!(warnings.emission("owner-b", "gh-ci", "missing", within), Some(0));
        assert_eq!(warnings.emission("owner-a", "github-ledger", "missing", within), Some(0));
        assert_eq!(warnings.emission("owner-a", "gh-ci", "expired", within), Some(0));
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", boundary), Some(2));
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", boundary), None);
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", boundary + FALLBACK_WARNING_INTERVAL), Some(1));
        assert_eq!(warnings.emission("owner-a", "gh-ci", "missing", boundary + FALLBACK_WARNING_INTERVAL * 2), Some(0));
    }

    #[test]
    fn fallback_warning_limiter_caps_keys_and_prunes_expired_windows() {
        let mut warnings = FallbackWarnings::default();
        let start = Instant::now();
        for index in 0..MAX_FALLBACK_WARNING_KEYS {
            assert_eq!(warnings.emission(&format!("owner-{index}"), "gh-ci", "missing", start), Some(0));
        }
        assert_eq!(warnings.keys, MAX_FALLBACK_WARNING_KEYS);
        // Over-cap keys always log, without being retained.
        assert_eq!(warnings.emission("overflow", "gh-ci", "missing", start), Some(0));
        assert_eq!(warnings.emission("overflow", "gh-ci", "missing", start), Some(0));
        assert_eq!(warnings.keys, MAX_FALLBACK_WARNING_KEYS);
        assert!(!warnings.owners.contains_key("overflow"));
        assert_eq!(warnings.emission("owner-0", "gh-ci", "missing", start), None);
        assert_eq!(warnings.emission("owner-1", "gh-ci", "missing", start), None);
        let boundary = start + FALLBACK_WARNING_INTERVAL;
        // Refresh one existing key, then insert: only the other keys are pruned.
        assert_eq!(warnings.emission("owner-0", "gh-ci", "missing", boundary), Some(1));
        assert_eq!(warnings.emission("overflow", "gh-ci", "missing", boundary), Some(0));
        assert_eq!(warnings.keys, 2);
        assert_eq!(warnings.owners.len(), 2);
        assert_eq!(warnings.emission("owner-0", "gh-ci", "missing", boundary), None);
        // A pruned key starts fresh, including its discarded suppression count.
        assert_eq!(warnings.emission("owner-1", "gh-ci", "missing", boundary), Some(0));
        assert_eq!(warnings.keys, 3);
    }

    #[test]
    fn supplied_root_rotation_expiry_permissions_and_reason_counters() {
        let root = tempfile::tempdir().unwrap();
        let owner = "source-fixture";
        let pat = GithubAuth::test("fixture-pat");
        let app = "gh-ci";
        let expires = chrono::Utc::now() + chrono::Duration::minutes(60);
        assert!(matches!(select(None, owner, app, &pat), GithubAuth::Local { .. }));
        assert_eq!(count(owner, app, "missing"), 1);
        select(Some(&root.path().join("absent")), owner, app, &pat);
        assert_eq!(count(owner, app, "missing"), 2);
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "no-installation"), 1);
        install(root.path(), owner, app, "fixture-one", expires);
        let first = select(Some(root.path()), owner, app, &pat);
        let principal = first.cache_principal().to_owned();
        install(root.path(), owner, app, "fixture-two", expires);
        let rotated = select(Some(root.path()), owner, app, &pat);
        assert_eq!(rotated.local_token().as_str(), "fixture-two");
        assert_eq!(principal, rotated.cache_principal(), "rotation is not a principal change");
        install(root.path(), owner, app, "fixture-expired", chrono::Utc::now() - chrono::Duration::seconds(1));
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "expired"), 1);
        install(root.path(), owner, app, "fixture-newline\n", expires);
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "unreadable"), 1);
        install(root.path(), owner, app, "fixture-good", expires);
        let token = root.path().join(app).join(format!("{owner}.token"));
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "unreadable"), 2);
        std::fs::remove_file(token).unwrap();
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "missing"), 3);
        std::fs::write(root.path().join(app).join(format!("{owner}.json")), b"invalid").unwrap();
        select(Some(root.path()), owner, app, &pat);
        assert_eq!(count(owner, app, "unreadable"), 3);
    }

    #[tokio::test]
    async fn read_rotation_on_401_and_pat_mutations_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let owner = "rotation-fixture";
        let expires = chrono::Utc::now() + chrono::Duration::minutes(60);
        install(root.path(), owner, "gh-ci", "fixture-old", expires);
        let auth = select(Some(root.path()), owner, "gh-ci", &GithubAuth::test("fixture-pat"));
        let directory = root.path().to_owned();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let server = server(move |authorization, path, _| {
            recorded.lock().push((authorization.to_owned(), path.to_owned()));
            if authorization == "Bearer fixture-old" {
                install(&directory, owner, "gh-ci", "fixture-rotated", expires);
                (StatusCode::UNAUTHORIZED, json!({}))
            } else { (StatusCode::OK, json!({"ok": true})) }
        }).await;
        let client = reqwest::Client::new();
        let response = crate::github_http::spend_as("fixture/app-rotation-accounting".into(),
            crate::github_http::send_read(client.get(format!("{}/repos/{owner}/repo/contents/a", server.base)), &auth)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let report = crate::resource::github_usage_report();
        let usage = report.spenders.iter().find(|usage| usage.spender == "fixture/app-rotation-accounting").unwrap();
        assert_eq!((usage.sent, usage.refused), (2, 1));
        assert_eq!(calls.lock().iter().map(|(auth, _)| auth.as_str()).collect::<Vec<_>>(),
            ["Bearer fixture-old", "Bearer fixture-rotated"]);
        let response = crate::github_http::send(client.post(format!("{}/repos/{owner}/repo/issues", server.base)).json(&json!({"title":"fixture"})), &auth).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = crate::github_http::send_read(client.post(format!("{}/graphql", server.base)).json(&json!({"query":"mutation { addComment }"})), &auth).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(calls.lock()[2..].iter().all(|(auth, _)| auth == "Bearer fixture-pat"));
    }

    #[tokio::test]
    async fn rejected_reads_fall_back_but_rejected_writes_are_never_replayed() {
        let root = tempfile::tempdir().unwrap();
        let owner = "rejected-fixture";
        install(root.path(), owner, "github-ledger", "fixture-rejected", chrono::Utc::now() + chrono::Duration::minutes(60));
        let auth = select(Some(root.path()), owner, "github-ledger", &GithubAuth::test("fixture-pat"));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let server = server(move |authorization, path, _| {
            recorded.lock().push((authorization.to_owned(), path.to_owned()));
            (StatusCode::UNAUTHORIZED, json!({}))
        }).await;
        let client = reqwest::Client::new();
        let response = crate::github_http::spend_as("fixture/app-rejection-accounting".into(),
            crate::github_http::send_read(client.get(format!("{}/repos/{owner}/repo/issues", server.base)), &auth)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let report = crate::resource::github_usage_report();
        let usage = report.spenders.iter().find(|usage| usage.spender == "fixture/app-rejection-accounting").unwrap();
        assert_eq!((usage.sent, usage.refused), (3, 3), "initial app401, reloaded app401 and PAT401 each count exactly once");
        assert_eq!(count(owner, "github-ledger", "401"), 1);
        assert!(!crate::github_http::read_response_cacheable(&response));
        assert!(!auth.used_fallback(), "request fallback must not mutate the batch");
        assert_eq!(calls.lock().len(), 3);
        assert_eq!(calls.lock()[2].0, "Bearer fixture-pat");
        crate::github_http::send(client.post(format!("{}/repos/{owner}/repo/issues", server.base)), &auth).await.unwrap();
        assert_eq!(calls.lock().len(), 4);
        assert_eq!(calls.lock()[3].0, "Bearer fixture-pat");
    }

    #[tokio::test]
    async fn rest_endpoint_selection_and_status_exception_use_existing_transport() {
        let root = tempfile::tempdir().unwrap();
        let owner = "routing-fixture";
        let expires = chrono::Utc::now() + chrono::Duration::minutes(60);
        install(root.path(), owner, "gh-ci", "fixture-ci", expires);
        install(root.path(), owner, "github-ledger", "fixture-ledger", expires);
        let auth = select(Some(root.path()), owner, "gh-ci", &GithubAuth::test("fixture-pat"));
        let server = server(|authorization, path, _| {
            let expected = if path.contains("/status") { "Bearer fixture-pat" }
                else if path.contains("/issues") { "Bearer fixture-ledger" } else { "Bearer fixture-ci" };
            assert_eq!(authorization, expected, "{path}");
            (StatusCode::OK, json!({}))
        }).await;
        let client = reqwest::Client::new();
        for suffix in ["", "/branches/main", "/contents/a", "/pulls/1", "/pulls/1/reviews",
            "/commits/head/check-runs", "/actions/runs", "/issues", "/issues/1/comments", "/issues/comments/2",
            "/commits/head/status"] {
            crate::github_http::send(client.get(format!("{}/repos/{owner}/repo{suffix}", server.base)), &auth).await.unwrap();
        }
        assert_eq!(count(owner, "gh-ci", "statuses"), 1);
        let pat = select(None, owner, "gh-ci", &GithubAuth::test("fixture-pat"));
        let server = self::server(|authorization, _, _| {
            assert_eq!(authorization, "Bearer fixture-pat");
            (StatusCode::OK, json!({}))
        }).await;
        crate::github_http::send(client.get(format!("{}/repos/unconfigured/repo/issues", server.base)), &pat).await.unwrap();
    }
}

#[cfg(test)]
mod response_accounting_tests {
    use super::*;

    #[tokio::test]
    async fn app_401_reload_401_and_successful_pat_final_each_count_once() {
        let root = tempfile::tempdir().unwrap();
        let owner = "pat-success-accounting-fixture";
        tests::install(root.path(), owner, "gh-ci", "fixture-rejected",
            chrono::Utc::now() + chrono::Duration::minutes(60));
        let auth = select(Some(root.path()), owner, "gh-ci", &GithubAuth::test("fixture-pat"));
        let fixture = tests::server(|authorization, _, _| {
            let status = if authorization == "Bearer fixture-pat" {
                reqwest::StatusCode::OK
            } else { reqwest::StatusCode::UNAUTHORIZED };
            (status, serde_json::json!({}))
        }).await;
        let response = crate::github_http::spend_as("fixture/app-pat-success-accounting".into(),
            crate::github_http::send_read(reqwest::Client::new().get(
                format!("{}/repos/{owner}/repo/pulls", fixture.base)), &auth)).await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let report = crate::resource::github_usage_report();
        let usage = report.spenders.iter().find(|usage| usage.spender == "fixture/app-pat-success-accounting").unwrap();
        assert_eq!((usage.sent, usage.refused), (3, 2));
    }
}
