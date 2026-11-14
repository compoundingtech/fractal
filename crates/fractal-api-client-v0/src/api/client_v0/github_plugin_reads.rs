//! `GET /v1/client/plugins/github/*`: bounded, cached, read-only GitHub reads for the web client's
//! GitHub plugin (scope `read.projections`). The browser names only typed `{owner, repo, number | id}`
//! path values, never a URL; the daemon reads `https://api.github.com` alone, through
//! `fractal-github-base`'s credentialed, admitted send boundary, one hop at a time, never following
//! a redirect. This is not an image or URL proxy: no route fetches a client-chosen location.
//!
//! The wire shapes are `apps/fractal-web/src/plugins/github/schema.ts`; the normative examples are
//! `docs/st3/client-v0/fixtures/github-plugin-*.json`. Each answer is a `GithubRead<T>`, which the
//! client-v0 middleware wraps in the usual `Envelope` as its `value`.
use super::*;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

mod map;
pub(crate) mod wire;
#[cfg(test)]
mod tests;

use wire::*;

/// The one upstream origin. Not configurable and never derived from a request.
const API_BASE: &str = "https://api.github.com";
/// Who the requests are spent as in the daemon's GitHub admission accounting.
const SPENDER: &str = "plugin/github";
/// A cached upstream answer is served without asking GitHub for this long, then revalidated.
const FRESH_FOR: Duration = Duration::from_secs(30);
/// The cache's own bounds, apart from every other GitHub cache of the daemon.
const CACHE_ENTRIES: usize = 512;
const CACHE_BYTES: usize = 64 * 1024 * 1024;
/// The largest upstream body one request may return.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// One route's whole budget; the client-v0 ordinary read deadline is 15 seconds.
const ROUTE_BUDGET: Duration = Duration::from_secs(12);
/// Conversation and checks list at most this many of each kind.
pub(crate) const LIST_BOUND: usize = 100;
/// GitHub lists at most 3,000 files for a pull request, 100 per page.
const MAX_FILES: usize = 3_000;
const FILES_PAGE: usize = 100;
/// GitHub's own words in an `unavailable` reason are cut to this many characters.
const MAX_REASON_CHARS: usize = 200;

/// The typed subject of a read. Owners and repositories follow GitHub's naming rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Repo {
    pub owner: String,
    pub repo: String,
}

pub(crate) fn valid_owner(owner: &str) -> bool {
    let bytes = owner.as_bytes();
    (1..=39).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

pub(crate) fn valid_repo(repo: &str) -> bool {
    (1..=100).contains(&repo.len())
        && !matches!(repo, "." | "..")
        && repo
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A positive decimal with no sign, no leading zero and no more than `maximum`.
pub(crate) fn parse_id(value: &str, maximum: u64) -> Option<u64> {
    if value.is_empty() || value.len() > 20 || value.starts_with('0') || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|id| (1..=maximum).contains(id))
}

fn subject(owner: &str, repo: &str) -> Result<Repo, ApiError> {
    if !valid_owner(owner) {
        return Err(validation("owner must match ^[A-Za-z0-9][A-Za-z0-9-]{0,38}$"));
    }
    if !valid_repo(repo) {
        return Err(validation("repo must match ^[A-Za-z0-9._-]{1,100}$ and not be . or .."));
    }
    Ok(Repo { owner: owner.to_owned(), repo: repo.to_owned() })
}

/// Pull request and issue numbers are GraphQL `Int`s.
fn number(value: &str) -> Result<u64, ApiError> {
    parse_id(value, i32::MAX as u64).ok_or_else(|| validation("number must be a positive integer"))
}

fn id(value: &str) -> Result<u64, ApiError> {
    parse_id(value, i64::MAX as u64).ok_or_else(|| validation("id must be a positive integer"))
}

fn answer<T: serde::Serialize>(read: GithubRead<T>) -> Result<Json<Value>, ApiError> {
    serde_json::to_value(read).map(Json).map_err(ApiError::internal)
}

async fn serve<T, F>(session: &ClientSession, work: F) -> Result<Json<Value>, ApiError>
where
    T: serde::Serialize,
    F: Future<Output = GithubRead<T>>,
{
    require_scope(session, "read.projections")?;
    answer(crate::github_http::spend_as(SPENDER.to_owned(), work).await)
}

pub(super) async fn pull(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, n)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, n) = (subject(&owner, &repo)?, number(&n)?);
    serve(&session, async move { Reads::new().pull(&repo, n).await }).await
}

pub(super) async fn issue(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, n)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, n) = (subject(&owner, &repo)?, number(&n)?);
    serve(&session, async move { Reads::new().issue(&repo, n).await }).await
}

pub(super) async fn conversation(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, n)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, n) = (subject(&owner, &repo)?, number(&n)?);
    serve(&session, async move { Reads::new().conversation(&repo, n).await }).await
}

pub(super) async fn checks(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, n)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, n) = (subject(&owner, &repo)?, number(&n)?);
    serve(&session, async move { Reads::new().checks(&repo, n).await }).await
}

pub(super) async fn files(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, n)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, n) = (subject(&owner, &repo)?, number(&n)?);
    serve(&session, async move { Reads::new().files(&repo, n).await }).await
}

pub(super) async fn comment(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, comment)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, comment) = (subject(&owner, &repo)?, id(&comment)?);
    serve(&session, async move { Reads::new().comment(&repo, comment).await }).await
}

pub(super) async fn run(
    Extension(session): Extension<ClientSession>,
    AxumPath((owner, repo, run)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let (repo, run) = (subject(&owner, &repo)?, id(&run)?);
    serve(&session, async move { Reads::new().run(&repo, run).await }).await
}

/// Why an upstream request produced no item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Moved(GithubMovedTo),
    Gone,
    NotFound,
    Unavailable { reason: String, retry_after_s: Option<u64> },
    /// A 301 to `/repositories/{id}/...`, GitHub's form for a renamed or transferred repository.
    /// Resolved to [`Failure::Moved`] by one more read of the repository; never answered as is.
    Relocated { repository_id: u64, number: Option<u64>, cacheable: bool },
}

impl Failure {
    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable { reason: reason.into(), retry_after_s: None }
    }

    fn into_read<T>(self) -> GithubRead<T> {
        match self {
            Self::Moved(to) => GithubRead::Moved { to },
            Self::Gone => GithubRead::Gone,
            Self::NotFound => GithubRead::NotFound,
            Self::Unavailable { reason, retry_after_s } => GithubRead::Unavailable { reason, retry_after_s },
            Self::Relocated { .. } => GithubRead::Unavailable { reason: UNRESOLVED_MOVE.into(), retry_after_s: None },
        }
    }

    /// Only an upstream failure may be answered from the cache; GitHub's 404, 410 and 301 are answers.
    fn may_serve_stale(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

const UNRESOLVED_MOVE: &str = "GitHub moved this item somewhere Fractal does not read";

/// One upstream document as the cache holds it.
#[derive(Clone, Debug)]
pub(crate) struct Fetched {
    pub value: Arc<Value>,
    pub fetched_at: String,
    pub stale: bool,
    pub cacheable: bool,
    /// The REST `Link` header named a next page.
    pub has_next: bool,
}

/// What one HTTP exchange returned, before classification.
#[derive(Clone, Debug, Default)]
pub(crate) struct Raw {
    pub status: u16,
    pub etag: Option<String>,
    pub location: Option<String>,
    pub retry_after_s: Option<u64>,
    pub rate_limited: bool,
    pub has_next: bool,
    pub cacheable: bool,
    pub body: Vec<u8>,
}

/// The parts of one route's answer, joined: the oldest part dates the whole, any stale part makes it stale.
#[derive(Default)]
struct Stamp {
    fetched_at: Option<String>,
    stale: bool,
}

impl Stamp {
    fn add(&mut self, part: &Fetched) {
        self.stale |= part.stale;
        if self.fetched_at.as_ref().is_none_or(|at| part.fetched_at < *at) {
            self.fetched_at = Some(part.fetched_at.clone());
        }
    }

    fn found<T>(self, item: T) -> GithubRead<T> {
        GithubRead::Found { fetched_at: self.fetched_at.unwrap_or_else(now), stale: self.stale, item }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

struct CacheEntry {
    etag: Option<String>,
    value: Arc<Value>,
    bytes: usize,
    fetched_at: String,
    has_next: bool,
    fresh_until: Instant,
    used: u64,
    /// A remembered `moved` answer, kept like a document but never revalidated by ETag.
    moved: Option<GithubMovedTo>,
}

/// Upstream documents by request key, with its own entry and byte caps.
#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<String, CacheEntry>,
    bytes: usize,
    tick: u64,
}

impl Cache {
    fn touch(&mut self, key: &str) -> Option<&mut CacheEntry> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        entry.used = tick;
        Some(entry)
    }

    fn insert(&mut self, key: String, entry: CacheEntry) {
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes;
        }
        if entry.bytes > CACHE_BYTES {
            return;
        }
        while !self.entries.is_empty() && (self.entries.len() >= CACHE_ENTRIES || self.bytes + entry.bytes > CACHE_BYTES) {
            let oldest = self.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone());
            if let Some(oldest) = oldest.and_then(|key| self.entries.remove(&key)) {
                self.bytes -= oldest.bytes;
            }
        }
        self.bytes += entry.bytes;
        self.entries.insert(key, entry);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

static CACHE: LazyLock<std::sync::Mutex<Cache>> = LazyLock::new(Default::default);

/// Read `key` through `cache`: a fresh entry answers at once; otherwise `send` asks GitHub, with the
/// entry's ETag when it has one, and a 304 refreshes it. An upstream failure falls back to a cached
/// entry, marked stale.
pub(crate) async fn cached<F, Fut>(cache: &std::sync::Mutex<Cache>, key: &str, revalidate: bool, send: F) -> Result<Fetched, Failure>
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: Future<Output = Result<Raw, Failure>>,
{
    let etag = {
        let mut cache = cache.lock().map_err(|_| Failure::unavailable("GitHub read cache is unavailable"))?;
        match cache.touch(key) {
            Some(entry) if entry.fresh_until > Instant::now() => {
                if let Some(to) = &entry.moved {
                    return Err(Failure::Moved(to.clone()));
                }
                return Ok(Fetched { value: entry.value.clone(), fetched_at: entry.fetched_at.clone(), stale: false, cacheable: true, has_next: entry.has_next });
            }
            Some(entry) if revalidate && entry.moved.is_none() => entry.etag.clone(),
            _ => None,
        }
    };
    let response = send(etag.clone()).await;
    let cacheable = response.as_ref().is_ok_and(|raw| raw.cacheable);
    let result = response.and_then(|raw| classify(raw, etag.is_some()));
    let mut cache = cache.lock().map_err(|_| Failure::unavailable("GitHub read cache is unavailable"))?;
    match result {
        Ok(Classified::NotModified) => match cache.touch(key) {
            Some(entry) => {
                entry.fresh_until = Instant::now() + FRESH_FOR;
                entry.fetched_at = now();
                Ok(Fetched { value: entry.value.clone(), fetched_at: entry.fetched_at.clone(), stale: false, cacheable, has_next: entry.has_next })
            }
            None => Err(Failure::unavailable("GitHub answered 304 for a read Fractal no longer holds")),
        },
        Ok(Classified::Document { value, etag, has_next, cacheable, bytes }) => {
            let value = Arc::new(value);
            let fetched_at = now();
            if cacheable {
                let used = cache.tick;
                cache.insert(key.to_owned(), CacheEntry {
                    etag,
                    value: value.clone(),
                    bytes,
                    fetched_at: fetched_at.clone(),
                    has_next,
                    fresh_until: Instant::now() + FRESH_FOR,
                    used,
                    moved: None,
                });
            } else if let Some(old) = cache.entries.remove(key) {
                cache.bytes -= old.bytes;
            }
            Ok(Fetched { value, fetched_at, stale: false, cacheable, has_next })
        }
        Err(Failure::Moved(to)) => {
            if cacheable {
                remember_moved(&mut cache, key, &to);
            } else if let Some(old) = cache.entries.remove(key) {
                cache.bytes -= old.bytes;
            }
            Err(Failure::Moved(to))
        }
        Err(failure) if failure.may_serve_stale() => match cache.touch(key) {
            Some(entry) if entry.moved.is_none() => Ok(Fetched { value: entry.value.clone(), fetched_at: entry.fetched_at.clone(), stale: true, cacheable: true, has_next: entry.has_next }),
            _ => Err(failure),
        },
        // Resolved by the caller, which remembers the move it finds.
        Err(failure @ Failure::Relocated { .. }) => Err(failure),
        Err(failure) => {
            if let Some(old) = cache.entries.remove(key) {
                cache.bytes -= old.bytes;
            }
            Err(failure)
        }
    }
}

pub(crate) enum Classified {
    NotModified,
    Document { value: Value, etag: Option<String>, has_next: bool, cacheable: bool, bytes: usize },
}

/// GitHub's status, headers and body as a document or a [`Failure`].
pub(crate) fn classify(raw: Raw, conditional: bool) -> Result<Classified, Failure> {
    let words = || {
        serde_json::from_slice::<Value>(&raw.body)
            .ok()
            .and_then(|body| body.get("message").and_then(Value::as_str).map(bounded_reason))
    };
    match raw.status {
        200 => {
            let value = serde_json::from_slice::<Value>(&raw.body)
                .map_err(|_| Failure::unavailable("GitHub's answer was not JSON"))?;
            if let Some(failure) = graphql_failure(&value) {
                return Err(failure);
            }
            Ok(Classified::Document { value, etag: raw.etag, has_next: raw.has_next, cacheable: raw.cacheable, bytes: raw.body.len() })
        }
        304 if conditional && raw.cacheable => Ok(Classified::NotModified),
        301 => Err(raw
            .location
            .as_deref()
            .and_then(moved_to)
            .map(Failure::Moved)
            .or_else(|| raw.location.as_deref().and_then(relocated_to).map(|failure| match failure {
                Failure::Relocated { repository_id, number, .. } => Failure::Relocated { repository_id, number, cacheable: raw.cacheable },
                other => other,
            }))
            .unwrap_or_else(|| Failure::unavailable(UNRESOLVED_MOVE))),
        404 => Err(Failure::NotFound),
        410 => Err(Failure::Gone),
        401 => Err(Failure::unavailable("GitHub rejected this daemon's credential")),
        403 | 429 if raw.rate_limited || raw.retry_after_s.is_some() || raw.status == 429 => Err(Failure::Unavailable {
            reason: words().unwrap_or_else(|| "GitHub's rate limit is spent".into()),
            retry_after_s: raw.retry_after_s,
        }),
        status => Err(Failure::unavailable(match words() {
            Some(words) => format!("GitHub answered HTTP {status}: {words}"),
            None => format!("GitHub answered HTTP {status}"),
        })),
    }
}

fn graphql_failure(value: &Value) -> Option<Failure> {
    let errors = value.get("errors")?.as_array().filter(|errors| !errors.is_empty())?;
    let kind = |name: &str| errors.iter().any(|error| error.get("type").and_then(Value::as_str) == Some(name));
    if kind("NOT_FOUND") {
        return Some(Failure::NotFound);
    }
    let message = errors
        .iter()
        .find_map(|error| error.get("message").and_then(Value::as_str))
        .map(bounded_reason)
        .unwrap_or_else(|| "GitHub's GraphQL answer carried errors".into());
    Some(Failure::Unavailable { reason: message, retry_after_s: None })
}

pub(crate) fn bounded_reason(words: &str) -> String {
    words.chars().filter(|c| !c.is_control()).take(MAX_REASON_CHARS).collect()
}

/// A 301's `Location`, accepted only on `api.github.com` and only when it parses back to a typed
/// repository (and item number, when it names one).
pub(crate) fn moved_to(location: &str) -> Option<GithubMovedTo> {
    let url = reqwest::Url::parse(location).ok()?;
    // Only an already-normal absolute URL: dot segments or escapes the parser rewrote are refused.
    if url.as_str() != location || url.fragment().is_some() || url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let ["repos", owner, repo, rest @ ..] = segments.as_slice() else {
        return None;
    };
    if !valid_owner(owner) || !valid_repo(repo) {
        return None;
    }
    let number = match rest {
        [] => None,
        ["pulls" | "issues", n, ..] => Some(parse_id(n, i32::MAX as u64)?),
        _ => None,
    };
    Some(GithubMovedTo { owner: (*owner).to_owned(), repo: (*repo).to_owned(), number })
}

fn remember_moved(cache: &mut Cache, key: &str, to: &GithubMovedTo) {
    let used = cache.tick;
    cache.insert(key.to_owned(), CacheEntry {
        etag: None,
        value: Arc::new(Value::Null),
        bytes: 0,
        fetched_at: now(),
        has_next: false,
        fresh_until: Instant::now() + FRESH_FOR,
        used,
        moved: Some(to.clone()),
    });
}

/// A 301's `Location` naming a repository by numeric id, `https://api.github.com/repositories/{id}`,
/// with the item number when its tail is `pulls/{n}` or `issues/{n}`.
pub(crate) fn relocated_to(location: &str) -> Option<Failure> {
    let url = reqwest::Url::parse(location).ok()?;
    if url.as_str() != location || url.fragment().is_some() || url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let ["repositories", id, rest @ ..] = segments.as_slice() else {
        return None;
    };
    let repository_id = parse_id(id, i64::MAX as u64)?;
    let number = match rest {
        ["pulls" | "issues", n, ..] => parse_id(n, i32::MAX as u64),
        _ => None,
    };
    Some(Failure::Relocated { repository_id, number, cacheable: true })
}

/// The pull request or issue number a `/repos/{owner}/{repo}/...` request path names.
fn requested_number(path: &str) -> Option<u64> {
    let path = path.split('?').next()?;
    match path.split('/').collect::<Vec<_>>().as_slice() {
        ["", "repos", _, _, "pulls" | "issues", n, ..] => parse_id(n, i32::MAX as u64),
        _ => None,
    }
}

/// Resolve a relocation with ONE more no-follow read of `/repositories/{id}` through `fetch`: its
/// `full_name` is where the item moved, and the answer is remembered under `key`. A second redirect,
/// any failure or a name that is not a typed owner and repository is `unavailable`.
pub(crate) async fn resolve_relocation<F, Fut>(
    cache: &std::sync::Mutex<Cache>,
    key: &str,
    auth: &crate::github_http::GithubAuth,
    repository_id: u64,
    number: Option<u64>,
    cacheable: bool,
    fetch: F,
) -> Failure
where
    F: FnOnce(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<Raw, Failure>>,
{
    let path = format!("/repositories/{repository_id}");
    let lookup = cached(cache, &format!("GET {} {path}", auth.cache_principal()), true, |etag| fetch(path, etag)).await;
    let Ok(repository) = lookup else {
        return Failure::unavailable(UNRESOLVED_MOVE);
    };
    let Some((owner, repo)) = repository.value.get("full_name").and_then(Value::as_str).and_then(|name| name.split_once('/')) else {
        return Failure::unavailable(UNRESOLVED_MOVE);
    };
    if !valid_owner(owner) || !valid_repo(repo) {
        return Failure::unavailable(UNRESOLVED_MOVE);
    }
    let to = GithubMovedTo {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number,
    };
    if cacheable && repository.cacheable {
        match cache.lock() {
            Ok(mut cache) => remember_moved(&mut cache, key, &to),
            Err(_) => return Failure::unavailable("GitHub read cache is unavailable"),
        }
    }
    Failure::Moved(to)
}

/// `GET {path}` through `cache` with `fetch`, resolving a relocation to `moved` with one more read.
pub(crate) async fn rest_read<F, Fut>(cache: &std::sync::Mutex<Cache>, auth: &crate::github_http::GithubAuth, path: &str, fetch: F) -> Result<Fetched, Failure>
where
    F: Fn(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<Raw, Failure>>,
{
    let key = format!("GET {} {path}", auth.cache_principal());
    match cached(cache, &key, true, |etag| fetch(path.to_owned(), etag)).await {
        Err(Failure::Relocated { repository_id, number, cacheable }) => {
            Err(resolve_relocation(cache, &key, auth, repository_id, requested_number(path).or(number), cacheable, &fetch).await)
        }
        other => other,
    }
}

/// One route's reads, sharing one deadline.
struct Reads {
    deadline: Instant,
}

async fn credential() -> Result<crate::github_http::GithubAuth, Failure> {
    crate::github_http::github_auth().await.map_err(|_| {
        Failure::unavailable("this daemon has no GitHub credential; run `gh auth login` as the daemon account")
    })
}

impl Reads {
    fn new() -> Self {
        Self { deadline: Instant::now() + ROUTE_BUDGET }
    }

    fn remaining(&self) -> Result<Duration, Failure> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| Failure::unavailable("GitHub did not answer in time"))
    }

    /// `GET {API_BASE}{path}`, cached with ETag revalidation.
    async fn rest(&mut self, path: &str) -> Result<Fetched, Failure> {
        let auth = crate::github_http::read_auth_for_url(&format!("{API_BASE}{path}"), &credential().await?);
        rest_read(&CACHE, &auth, path, |path, etag| self.get(auth.clone(), path, etag)).await
    }

    /// One no-follow `GET {API_BASE}{path}` within the route's budget.
    fn get(&self, auth: crate::github_http::GithubAuth, path: String, etag: Option<String>) -> impl Future<Output = Result<Raw, Failure>> + use<> {
        let budget = self.remaining();
        async move {
            let budget = budget?;
            let mut request = crate::resource::github_api_client()
                .get(format!("{API_BASE}{path}"))
                .timeout(budget)
                .header("accept", "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28");
            if let Some(etag) = etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            exchange(request, &auth, budget).await
        }
    }

    /// One fixed GraphQL query, cached by TTL only (GraphQL has no ETag).
    async fn graphql(&mut self, owner: &str, query: &'static str, variables: Value) -> Result<Fetched, Failure> {
        graphql_read(&CACHE, &credential().await?, API_BASE, owner, query, variables, self.remaining()?).await
    }
}

/// Select the owner's installation before consulting the principal-partitioned TTL cache.
async fn graphql_read(
    cache: &std::sync::Mutex<Cache>,
    credential: &crate::github_http::GithubAuth,
    api_base: &str,
    owner: &str,
    query: &str,
    variables: Value,
    budget: Duration,
) -> Result<Fetched, Failure> {
    let auth = crate::github_http::read_auth(owner, "gh-ci", credential);
    let key = format!("POST {} /graphql {query} {variables}", auth.cache_principal());
    cached(cache, &key, false, |_| async move {
        let body = serde_json::to_vec(&json!({ "query": query, "variables": variables }))
            .map_err(|_| Failure::unavailable("GraphQL request could not be encoded"))?;
        let request = crate::resource::github_api_client()
            .post(format!("{api_base}/graphql"))
            .timeout(budget)
            .header("accept", "application/vnd.github+json")
            .header("content-type", "application/json")
            .header("x-github-api-version", "2022-11-28")
            .body(body);
        exchange(request, &auth, budget).await
    }).await
}

async fn exchange(request: reqwest::RequestBuilder, auth: &crate::github_http::GithubAuth, budget: Duration) -> Result<Raw, Failure> {
    let send = crate::github_http::send_read_once(request, auth);
    let mut response = match tokio::time::timeout(budget, send).await {
        Err(_) => return Err(Failure::unavailable("GitHub did not answer in time")),
        Ok(Err(error)) => return Err(Failure::unavailable(bounded_reason(&format!("GitHub could not be read: {error}")))),
        Ok(Ok(response)) => response,
    };
    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned);
    let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let rate_limited = header("x-ratelimit-remaining").as_deref() == Some("0");
    let retry_after_s = header("retry-after").and_then(|value| value.trim().parse::<u64>().ok()).or_else(|| {
        rate_limited
            .then(|| header("x-ratelimit-reset").and_then(|reset| reset.parse::<u64>().ok()))
            .flatten()
            .map(|reset| reset.saturating_sub(now_s))
    });
    let mut raw = Raw {
        status: response.status().as_u16(),
        etag: header("etag"),
        location: header("location"),
        retry_after_s,
        rate_limited,
        has_next: header("link").is_some_and(|link| link.split(',').any(|part| part.contains("rel=\"next\""))),
        cacheable: crate::github_http::read_response_cacheable(&response),
        body: Vec::new(),
    };
    loop {
        let chunk = match tokio::time::timeout(budget, response.chunk()).await {
            Err(_) => return Err(Failure::unavailable("GitHub did not finish its answer in time")),
            Ok(Err(_)) => return Err(Failure::unavailable("GitHub's answer was cut off")),
            Ok(Ok(chunk)) => chunk,
        };
        let Some(chunk) = chunk else { break };
        if raw.body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Failure::unavailable("GitHub's answer is larger than Fractal reads"));
        }
        raw.body.extend_from_slice(&chunk);
    }
    Ok(raw)
}

fn repo_path(repo: &Repo) -> String {
    format!("/repos/{}/{}", repo.owner, repo.repo)
}

fn graphql_variables(repo: &Repo, number: u64) -> Value {
    json!({ "owner": repo.owner, "name": repo.repo, "number": number })
}

macro_rules! part {
    ($stamp:expr, $result:expr) => {
        match $result {
            Ok(part) => {
                $stamp.add(&part);
                part
            }
            Err(failure) => return failure.into_read(),
        }
    };
}

impl Reads {
    async fn pull(&mut self, repo: &Repo, number: u64) -> GithubRead<GithubPullRequest> {
        let mut stamp = Stamp::default();
        let rest = part!(stamp, self.rest(&format!("{}/pulls/{number}", repo_path(repo))).await);
        let graph = part!(stamp, self.graphql(&repo.owner, map::PULL_QUERY, graphql_variables(repo, number)).await);
        match map::pull(repo, number, &rest.value, &graph.value) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn issue(&mut self, repo: &Repo, number: u64) -> GithubRead<GithubIssue> {
        let mut stamp = Stamp::default();
        let rest = part!(stamp, self.rest(&format!("{}/issues/{number}", repo_path(repo))).await);
        match map::issue(repo, number, &rest.value) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn comment(&mut self, repo: &Repo, id: u64) -> GithubRead<GithubComment> {
        let mut stamp = Stamp::default();
        let rest = part!(stamp, self.rest(&format!("{}/issues/comments/{id}", repo_path(repo))).await);
        match map::comment(repo, &rest.value) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn conversation(&mut self, repo: &Repo, number: u64) -> GithubRead<GithubConversation> {
        let mut stamp = Stamp::default();
        let graph = part!(stamp, self.graphql(&repo.owner, map::CONVERSATION_QUERY, graphql_variables(repo, number)).await);
        let comments = part!(stamp, self.rest(&format!("{}/issues/{number}/comments?per_page={LIST_BOUND}", repo_path(repo))).await);
        match map::conversation(repo, &graph.value, &comments.value, comments.has_next) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn checks(&mut self, repo: &Repo, number: u64) -> GithubRead<GithubChecks> {
        let mut stamp = Stamp::default();
        let graph = part!(stamp, self.graphql(&repo.owner, map::CHECKS_QUERY, graphql_variables(repo, number)).await);
        match map::checks(&graph.value) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn files(&mut self, repo: &Repo, number: u64) -> GithubRead<GithubFiles> {
        let mut stamp = Stamp::default();
        let pull = part!(stamp, self.rest(&format!("{}/pulls/{number}", repo_path(repo))).await);
        let total = pull.value.get("changed_files").and_then(Value::as_u64).unwrap_or(0);
        let mut pages = Vec::new();
        let mut cut_short = false;
        for page in 1..=MAX_FILES / FILES_PAGE {
            let path = format!("{}/pulls/{number}/files?per_page={FILES_PAGE}&page={page}", repo_path(repo));
            let fetched = match self.rest(&path).await {
                Ok(fetched) => fetched,
                // Pages after the first are best effort: what was read is returned, marked truncated.
                Err(Failure::Unavailable { .. }) if page > 1 => {
                    cut_short = true;
                    break;
                }
                Err(failure) => return failure.into_read(),
            };
            stamp.add(&fetched);
            let more = fetched.has_next;
            pages.push(fetched.value);
            if !more {
                break;
            }
        }
        match map::files(total, &pages, cut_short) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }

    async fn run(&mut self, repo: &Repo, id: u64) -> GithubRead<GithubRun> {
        let mut stamp = Stamp::default();
        let run = part!(stamp, self.rest(&format!("{}/actions/runs/{id}", repo_path(repo))).await);
        let jobs = part!(stamp, self.rest(&format!("{}/actions/runs/{id}/jobs?per_page={LIST_BOUND}", repo_path(repo))).await);
        match map::run(repo, &run.value, &jobs.value) {
            Ok(item) => stamp.found(item),
            Err(failure) => failure.into_read(),
        }
    }
}
