//! A GitHub for tests: REST paths answered from a table, with ETag revalidation, and GraphQL
//! answered with a fixed body or queued pages. Other crates' tests drive the real repository
//! provider against it.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{ObservationRequest, ProviderObservation, pull_request_checks};
use crate::github_http::GithubAuth;

/// One poll by the real repository provider against the GitHub at `base`.
pub async fn observe(request: ObservationRequest, base: &str) -> anyhow::Result<ProviderObservation> {
    super::observe_at(request, base, Some(&GithubAuth::test("test"))).await
}

/// Pulls served by the fake listing, with the largest page it accepts.
pub type PullPages = Option<(Vec<Value>, usize)>;

/// A GitHub that answers each path from a table, revalidates with ETags, answers GraphQL
/// with a fixed body or queued pages, and keeps every request it received.
#[derive(Clone, Default)]
pub struct FakeGithub {
    pub routes: Arc<Mutex<HashMap<String, (String, String)>>>,
    pub links: Arc<Mutex<HashMap<String, String>>>,
    pub refused: Arc<Mutex<HashMap<String, String>>>,
    pub graphql: Arc<Mutex<String>>,
    pub graphql_pages: Arc<Mutex<HashMap<String, String>>>,
    pub graphql_queue: Arc<Mutex<std::collections::VecDeque<Value>>>,
    pub graphql_pulls: Arc<Mutex<PullPages>>,
    pub reject_combined_details: Arc<Mutex<bool>>,
    pub graphql_requests: Arc<Mutex<Vec<Value>>>,
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl FakeGithub {
    pub async fn start() -> (Self, String) {
        let github = Self::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        // cargo test shares the production caches between tests. A recycled TCP port
        // must not let a new fixture reuse another fixture's REST or GraphQL state.
        let scope = format!("/fixture/{}", uuid::Uuid::now_v7());
        let base = format!("http://{}{scope}", listener.local_addr().unwrap());
        let server = github.clone();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let server = server.clone();
                let scope = scope.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 4096];
                    let header_end = loop {
                        let size = stream.read(&mut chunk).await.unwrap();
                        request.extend_from_slice(&chunk[..size]);
                        if let Some(end) =
                            request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        {
                            break end + 4;
                        }
                        if size == 0 {
                            return;
                        }
                    };
                    // Route tables and request assertions use paths relative to this
                    // fixture's API base; the client/cache still sees the unique scope.
                    let head = String::from_utf8_lossy(&request[..header_end]).replacen(&scope, "", 1);
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    while request.len() < header_end + length {
                        let size = stream.read(&mut chunk).await.unwrap();
                        request.extend_from_slice(&chunk[..size]);
                    }
                    let target = head.lines().next().unwrap().to_owned();
                    server.requests.lock().push(head.clone());
                    let respond = |status: &str, etag: Option<&str>, body: &str| {
                        let etag =
                            etag.map_or(String::new(), |etag| format!("ETag: {etag}\r\n"));
                        let link = target.split(' ').nth(1)
                            .and_then(|path| server.links.lock().get(path).cloned())
                            .map_or(String::new(), |link| format!("Link: {link}\r\n"));
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{etag}{link}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    let response = if target.starts_with("POST /graphql ") {
                        let request: Value =
                            serde_json::from_slice(&request[header_end..header_end + length])
                                .unwrap();
                        let combined = request["query"].as_str().is_some_and(|query| {
                            query.contains("pullRequests(") && query.contains("nodes(ids:")
                        });
                        // Only the pull request listing pages; identity resolution is its own query.
                        let listing = request["variables"].get("first").is_some();
                        let keyed = request["variables"]["after"].as_str()
                            .and_then(|after| server.graphql_pages.lock().get(after).cloned());
                        let body = if combined && *server.reject_combined_details.lock() {
                            r#"{"errors":[{"message":"Something went wrong while executing your query"}]}"#.to_owned()
                        } else if let Some((pulls, max_size)) = server.graphql_pulls.lock().as_ref().filter(|_| listing) {
                            let first = request["variables"]["first"].as_u64().unwrap() as usize;
                            if first > *max_size {
                                json!({"errors": [{
                                    "message": "Something went wrong while executing your query",
                                }]}).to_string()
                            } else {
                                let after = request["variables"]["after"].as_str()
                                    .map_or(0, |cursor| cursor.parse::<usize>().unwrap());
                                let end = (after + first).min(pulls.len());
                                let cursor = end.to_string();
                                pull_page(
                                    json!(pulls[after..end]),
                                    (end < pulls.len()).then_some(cursor.as_str()), json!([]),
                                ).to_string()
                            }
                        } else if let Some(body) = keyed {
                            body
                        } else if let Some(page) = listing
                            .then(|| server.graphql_queue.lock().pop_front())
                            .flatten()
                        {
                            page.to_string()
                        } else {
                            server.graphql.lock().clone()
                        };
                        server.graphql_requests.lock().push(request);
                        respond("200 OK", None, &body)
                    } else {
                        let path = target.split(' ').nth(1).unwrap().to_owned();
                        let route = server.routes.lock().get(&path).cloned();
                        let refused = server.refused.lock().get(&path).cloned();
                        match route {
                            _ if refused.is_some() => respond(
                                refused.as_deref().unwrap(),
                                None,
                                r#"{"message":"Resource not accessible by integration"}"#,
                            ),
                            None => respond("404 Not Found", None, r#"{"message":"Not Found"}"#),
                            Some((etag, _))
                                if head
                                    .to_ascii_lowercase()
                                    .contains(&format!("if-none-match: {}", etag.to_ascii_lowercase())) =>
                            {
                                "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                            }
                            Some((etag, body)) => respond("200 OK", Some(&etag), &body),
                        }
                    };
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        (github, base)
    }

    pub fn route(&self, path: &str, body: Value) {
        let body = body.to_string();
        let etag = format!(
            "\"{}\"",
            &hex::encode(Sha256::digest(body.as_bytes()))[..12]
        );
        self.routes.lock()
            .insert(path.to_owned(), (etag, body));
    }

    /// Answer `path` with `status`, such as `403 Forbidden`.
    pub fn refuse(&self, path: &str, status: &str) {
        self.refused.lock()
            .insert(path.to_owned(), status.to_owned());
    }

    pub fn open_pull_requests(&self, nodes: Value) {
        *self.graphql.lock() = json!({"data": {"repository": {"pullRequests": {
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": nodes,
        }}}})
        .to_string();
    }

    pub fn resolved_nodes(&self, nodes: Value) {
        let mut response: Value = serde_json::from_str(&self.graphql.lock()).unwrap();
        response["data"]["nodes"] = nodes;
        *self.graphql.lock() = response.to_string();
    }

    pub fn take_requests(&self) -> Vec<String> {
        std::mem::take(&mut *self.requests.lock())
            .into_iter()
            .map(|request| request.lines().next().unwrap_or_default().to_owned())
            .collect()
    }
}

/// Make this host's last GraphQL answer for `acme/garden` at `base` older by `by`.
pub fn age_pull_request_check(base: &str, by: Duration) {
    let mut checks = pull_request_checks()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let key = format!("{base} acme/garden");
    let key = if checks.contains_key(&key) {
        key
    } else {
        format!("{key} issues")
    };
    let check = checks.get_mut(&key).expect("GraphQL was read");
    check.at = check.at.checked_sub(by).expect("the clock reaches back");
}

pub fn pull_page(nodes: Value, after: Option<&str>, resolved: Value) -> Value {
    json!({"data": {
        "repository": {"pullRequests": {
            "nodes": nodes,
            "pageInfo": {"hasNextPage": after.is_some(), "endCursor": after},
        }},
        "nodes": resolved,
    }})
}
