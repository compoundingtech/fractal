//! Contract: canned GitHub documents serialize to exactly `docs/st3/client-v0/fixtures/github-plugin-*.json`.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<Value>(/* sqlite-large-value: allow-for-now lv-ha3a3bb1e957c owner=fractal_api_client_v0::api::client_v0::github_plugin_reads::tests TODO=trace-statement-and-value-representation */ include_str!(concat!("../../../../../../docs/st3/client-v0/fixtures/github-plugin-", $name, ".json"))).unwrap()
    };
}

const AT: &str = "2026-11-07T06:40:00Z";
const SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn garden() -> Repo {
    Repo { owner: "acme".into(), repo: "garden".into() }
}

fn found<T: serde::Serialize>(item: T, stale: bool) -> Value {
    serde_json::to_value(GithubRead::Found { fetched_at: AT.into(), stale, item }).unwrap()
}

fn read_of<T: serde::Serialize>(failure: Failure) -> Value {
    serde_json::to_value(failure.into_read::<T>()).unwrap()
}

fn rest_user(login: &str) -> Value {
    json!({ "login": login, "id": 1, "url": format!("https://api.github.com/users/{login}"), "html_url": format!("https://github.com/{login}"), "avatar_url": "https://avatars.githubusercontent.com/u/1" })
}

fn graph_user(login: &str) -> Value {
    json!({ "login": login, "url": format!("https://github.com/{login}") })
}

fn rest_pull(expected: &Value) -> Value {
    json!({
        "url": "https://api.github.com/repos/acme/garden/pulls/7",
        "html_url": "https://github.com/acme/garden/pull/7",
        "number": 7,
        "title": expected["title"],
        "body": expected["body"],
        "state": if expected["state"] == "open" { "open" } else { "closed" },
        "draft": false,
        "user": rest_user("ada"),
        "base": { "ref": "main", "sha": "0000000000000000000000000000000000000000" },
        "head": { "ref": "ada/water-schedule", "sha": SHA },
        "additions": 214, "deletions": 37, "changed_files": 3, "commits": 4, "comments": 2, "review_comments": 3,
        "created_at": "2026-11-05T09:12:00Z", "updated_at": "2026-11-07T05:58:00Z",
        "merged_at": expected["merged_at"], "closed_at": expected["closed_at"],
        "labels": [
            { "id": 1, "name": "enhancement", "color": "a2eeef", "default": true },
            { "id": 2, "name": "needs-review", "color": "fbca04", "default": false }
        ],
        "requested_reviewers": [rest_user("grace")],
        "requested_teams": []
    })
}

fn graph_pull(merge_state: Value) -> Value {
    json!({ "data": { "repository": { "pullRequest": {
        "mergeStateStatus": merge_state,
        "reviewDecision": "CHANGES_REQUESTED",
        "closingIssuesReferences": { "nodes": [
            { "number": 9, "title": "Beds dry out over the weekend", "state": "OPEN", "repository": { "name": "garden", "owner": { "login": "acme" } } }
        ] }
    } } } })
}

#[test]
fn pull_request_serializes_to_the_fixture() {
    let expected = fixture!("pull-request");
    let item = map::pull(&garden(), 7, &rest_pull(&expected["item"]), &graph_pull(json!("BLOCKED"))).unwrap();
    assert_eq!(found(item, false), expected);
}

#[test]
fn stale_merged_pull_request_serializes_to_the_fixture() {
    let expected = fixture!("stale");
    let item = map::pull(&garden(), 7, &rest_pull(&expected["item"]), &graph_pull(Value::Null)).unwrap();
    assert_eq!(found(item, true), expected);
}

#[test]
fn issue_serializes_to_the_fixture() {
    let expected = fixture!("issue");
    let rest = json!({
        "url": "https://api.github.com/repos/acme/garden/issues/9",
        "html_url": "https://github.com/acme/garden/issues/9",
        "number": 9,
        "title": expected["item"]["title"],
        "body": expected["item"]["body"],
        "state": "open", "state_reason": null,
        "user": rest_user("grace"),
        "assignees": [rest_user("ada")],
        "labels": [{ "name": "bug", "color": "d73a4a" }],
        "milestone": { "title": "Spring", "number": 1 },
        "comments": 3,
        "created_at": "2026-11-01T08:00:00Z", "updated_at": "2026-11-06T17:30:00Z", "closed_at": null
    });
    assert_eq!(found(map::issue(&garden(), 9, &rest).unwrap(), false), expected);
    let mut pull = rest;
    pull["pull_request"] = json!({ "url": "https://api.github.com/repos/acme/garden/pulls/9" });
    assert!(map::issue(&garden(), 9, &pull).unwrap().is_pull_request);
}

fn rest_comment(id: u64, login: &str, association: &str, body: &str, at: &str, reactions: Value) -> Value {
    let mut counts = json!({ "url": "https://api.github.com/x", "total_count": 0, "+1": 0, "-1": 0, "laugh": 0, "hooray": 0, "confused": 0, "heart": 0, "rocket": 0, "eyes": 0 });
    for (key, value) in reactions.as_object().unwrap() {
        counts[key] = value.clone();
    }
    json!({
        "id": id,
        "url": format!("https://api.github.com/repos/acme/garden/issues/comments/{id}"),
        "html_url": format!("https://github.com/acme/garden/pull/7#issuecomment-{id}"),
        "issue_url": "https://api.github.com/repos/acme/garden/issues/7",
        "user": rest_user(login),
        "author_association": association,
        "body": body,
        "created_at": at, "updated_at": at,
        "reactions": counts
    })
}

fn grace_comment() -> Value {
    rest_comment(
        1832451234,
        "grace",
        "MEMBER",
        "Looks close. Can the scheduler skip beds after rain? See the forecast in `weather.rs`.",
        "2026-11-06T10:02:00Z",
        json!({ "+1": 2, "eyes": 1, "total_count": 3 }),
    )
}

#[test]
fn comment_serializes_to_the_fixture() {
    assert_eq!(found(map::comment(&garden(), &grace_comment()).unwrap(), false), fixture!("comment"));
}

#[test]
fn conversation_serializes_to_the_fixture() {
    let graph = json!({ "data": { "repository": { "pullRequest": {
        "latestOpinionatedReviews": { "totalCount": 2, "nodes": [
            { "id": "PRR_kwDOA2", "state": "APPROVED", "body": "", "submittedAt": "2026-11-06T12:40:00Z", "url": "https://github.com/acme/garden/pull/7#pullrequestreview-2002", "author": graph_user("linus") },
            { "id": "PRR_kwDOA1", "state": "CHANGES_REQUESTED", "body": "The skip for dormant beds needs a test.", "submittedAt": "2026-11-06T10:05:00Z", "url": "https://github.com/acme/garden/pull/7#pullrequestreview-2001", "author": graph_user("grace") }
        ] },
        "reviewThreads": { "totalCount": 2, "nodes": [
            { "id": "PRRT_kwDOB1", "path": "src/scheduler.rs", "line": 42, "isResolved": false, "isOutdated": false, "comments": { "totalCount": 2, "nodes": [
                { "id": "PRRC_1", "body": "Should this read moisture once per bed, not once per run?", "createdAt": "2026-11-06T10:04:00Z", "url": "https://github.com/acme/garden/pull/7#discussion_r1001", "author": graph_user("grace") },
                { "id": "PRRC_2", "body": "Yes, fixed in the next push.", "createdAt": "2026-11-06T11:00:00Z", "url": "https://github.com/acme/garden/pull/7#discussion_r1002", "author": graph_user("ada") }
            ] } },
            { "id": "PRRT_kwDOB2", "path": "docs/garden-guide.md", "line": null, "isResolved": true, "isOutdated": true, "comments": { "totalCount": 1, "nodes": [
                { "id": "PRRC_3", "body": "Typo: *wtaer*.", "createdAt": "2026-11-06T12:30:00Z", "url": "https://github.com/acme/garden/pull/7#discussion_r1003", "author": graph_user("linus") }
            ] } }
        ] }
    } } } });
    let comments = json!([
        grace_comment(),
        rest_comment(1832459999, "ada", "CONTRIBUTOR", "Added rain skip.", "2026-11-06T15:00:00Z", json!({}))
    ]);
    let item = map::conversation(&garden(), &graph, &comments, false).unwrap();
    assert_eq!(found(item, false), fixture!("conversation"));
    // More than the bound of any kind is marked.
    let mut more = graph.clone();
    more["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"][0]["comments"]["totalCount"] = json!(101);
    assert!(map::conversation(&garden(), &more, &comments, false).unwrap().truncated);
    assert!(map::conversation(&garden(), &graph, &comments, true).unwrap().truncated);
}

#[test]
fn checks_serialize_to_the_fixture() {
    let actions = json!({ "app": { "name": "GitHub Actions" }, "workflowRun": { "workflow": { "name": "CI" } } });
    let run = |name: &str, job: u64, status: &str, conclusion: Value, started: Value, completed: Value, summary: Value, suite: &Value| json!({
        "__typename": "CheckRun", "name": name, "status": status, "conclusion": conclusion,
        "startedAt": started, "completedAt": completed,
        "detailsUrl": format!("https://github.com/acme/garden/actions/runs/9001/job/{job}"),
        "url": format!("https://github.com/acme/garden/runs/{job}"),
        "summary": summary, "checkSuite": suite
    });
    let graph = json!({ "data": { "repository": { "pullRequest": {
        "headRefOid": SHA,
        "commits": { "nodes": [{ "commit": { "oid": SHA, "statusCheckRollup": { "contexts": { "totalCount": 5, "nodes": [
            run("build", 1, "COMPLETED", json!("SUCCESS"), json!("2026-11-07T05:59:10Z"), json!("2026-11-07T06:03:42Z"), json!(""), &actions),
            run("test (ubuntu)", 2, "COMPLETED", json!("FAILURE"), json!("2026-11-07T05:59:12Z"), json!("2026-11-07T06:08:05Z"), json!("2 tests failed: scheduler::skips_dormant, scheduler::rain"), &actions),
            run("lint", 3, "COMPLETED", json!("SKIPPED"), Value::Null, Value::Null, Value::Null, &actions),
            run("e2e", 4, "IN_PROGRESS", Value::Null, json!("2026-11-07T06:01:00Z"), Value::Null, Value::Null, &actions),
            run("deploy-preview", 5, "QUEUED", Value::Null, Value::Null, Value::Null, Value::Null, &json!({ "app": null, "workflowRun": null }))
        ] } } } }] }
    } } } });
    assert_eq!(found(map::checks(&graph).unwrap(), false), fixture!("checks"));
}

#[test]
fn files_serialize_to_the_fixture() {
    let expected = fixture!("files");
    let files = &expected["item"]["files"];
    let page = json!([
        { "sha": "a", "filename": "src/scheduler.rs", "status": "added", "additions": 180, "deletions": 0, "changes": 180, "patch": files[0]["patch"] },
        { "sha": "b", "filename": "src/beds.rs", "previous_filename": "src/bed.rs", "status": "renamed", "additions": 34, "deletions": 37, "changes": 71, "patch": files[1]["patch"] },
        { "sha": "c", "filename": "assets/beds.png", "status": "added", "additions": 0, "deletions": 0, "changes": 0 }
    ]);
    assert_eq!(found(map::files(3, &[Arc::new(page.clone())], false).unwrap(), false), expected);
    // Fewer listed than GitHub counts, or a page that could not be read, is truncated.
    assert!(map::files(4, &[Arc::new(page.clone())], false).unwrap().truncated);
    assert!(map::files(3, &[Arc::new(page)], true).unwrap().truncated);
    // A patch that exceeds the encoded response budget is omitted, without dropping later small patches.
    let big = "x".repeat(CLIENT_MAX_RESPONSE_BYTES);
    let page = json!([
        { "filename": "a", "status": "modified", "additions": 1, "deletions": 0, "patch": big },
        { "filename": "b", "status": "modified", "additions": 1, "deletions": 0, "patch": "@@" }
    ]);
    let bounded = map::files(2, &[Arc::new(page)], false).unwrap();
    assert!(bounded.truncated);
    assert!(bounded.files[0].patch.is_none() && bounded.files[1].patch.is_some());
}

#[test]
fn run_serializes_to_the_fixture() {
    let job = |id: u64, name: &str, conclusion: &str, started: Value, completed: Value| json!({
        "id": id, "name": name, "status": "completed", "conclusion": conclusion,
        "started_at": started, "completed_at": completed,
        "html_url": format!("https://github.com/acme/garden/actions/runs/9001/job/{id}")
    });
    let run = json!({
        "id": 9001, "name": "CI", "display_title": "Water the beds on a schedule", "event": "pull_request",
        "status": "completed", "conclusion": "failure",
        "head_branch": "ada/water-schedule", "head_sha": SHA,
        "url": "https://api.github.com/repos/acme/garden/actions/runs/9001",
        "html_url": "https://github.com/acme/garden/actions/runs/9001",
        "run_attempt": 1,
        "run_started_at": "2026-11-07T05:59:00Z",
        "created_at": "2026-11-07T05:58:50Z", "updated_at": "2026-11-07T06:08:05Z",
        "pull_requests": [{ "id": 77, "number": 7, "url": "https://api.github.com/repos/acme/garden/pulls/7" }]
    });
    let jobs = json!({ "total_count": 3, "jobs": [
        job(1, "build", "success", json!("2026-11-07T05:59:10Z"), json!("2026-11-07T06:03:42Z")),
        job(2, "test (ubuntu)", "failure", json!("2026-11-07T05:59:12Z"), json!("2026-11-07T06:08:05Z")),
        job(3, "lint", "skipped", Value::Null, Value::Null)
    ] });
    assert_eq!(found(map::run(&garden(), &run, &jobs).unwrap(), false), fixture!("run"));
}

#[test]
fn moved_gone_not_found_and_unavailable_serialize_to_the_fixtures() {
    let moved = Raw { status: 301, location: Some("https://api.github.com/repos/acme-garden/garden/pulls/7".into()), ..Raw::default() };
    let Err(failure) = classify(moved, false) else { panic!("a 301 is not a document") };
    assert_eq!(read_of::<GithubPullRequest>(failure), fixture!("moved"));
    let Err(failure) = classify(Raw { status: 410, ..Raw::default() }, false) else { panic!() };
    assert_eq!(read_of::<GithubIssue>(failure), fixture!("gone"));
    let Err(failure) = classify(Raw { status: 404, ..Raw::default() }, false) else { panic!() };
    assert_eq!(read_of::<GithubIssue>(failure), fixture!("not-found"));
    let limited = Raw {
        status: 403,
        rate_limited: true,
        retry_after_s: Some(120),
        body: br#"{"message":"GitHub API rate limit exceeded for this installation","documentation_url":"https://docs.github.com/rest"}"#.to_vec(),
        ..Raw::default()
    };
    let Err(failure) = classify(limited, false) else { panic!() };
    assert_eq!(read_of::<GithubIssue>(failure), fixture!("unavailable"));
}

#[test]
fn a_redirect_is_reported_only_when_it_names_a_typed_github_item() {
    assert_eq!(moved_to("https://api.github.com/repos/o/r"), Some(GithubMovedTo { owner: "o".into(), repo: "r".into(), number: None }));
    assert_eq!(moved_to("https://api.github.com/repos/o/r/issues/3/comments").unwrap().number, Some(3));
    for location in [
        "https://api.github.com/repositories/1234/pulls/7",
        "https://evil.example/repos/o/r/pulls/7",
        "http://api.github.com/repos/o/r/pulls/7",
        "https://api.github.com:8443/repos/o/r/pulls/7",
        "https://user@api.github.com/repos/o/r/pulls/7",
        "https://api.github.com/repos/-o/r/pulls/7",
        "https://api.github.com/repos/o/../pulls/7",
        "https://api.github.com/repos/o/./r/pulls/7",
        "https://API.github.com/repos/o/r/pulls/7",
        "https://api.github.com/repos/o/r/pulls/7#x",
        "https://api.github.com/repos/o/r/pulls/0",
        "/repos/o/r/pulls/7",
    ] {
        assert_eq!(moved_to(location), None, "{location}");
    }
    // GitHub's rename and transfer form is resolved by one more read, never followed.
    assert_eq!(
        classify(Raw { status: 301, location: Some("https://api.github.com/repositories/1/pulls/7".into()), ..Raw::default() }, false).err(),
        Some(Failure::Relocated { repository_id: 1, number: Some(7), cacheable: false })
    );
    let Err(Failure::Unavailable { .. }) = classify(Raw { status: 301, location: Some("https://api.github.com/user/1".into()), ..Raw::default() }, false) else {
        panic!("an untyped redirect is unavailable")
    };
    for status in [302, 307, 308, 500, 502, 401] {
        assert!(matches!(classify(Raw { status, ..Raw::default() }, false), Err(Failure::Unavailable { .. })), "{status}");
    }
    // A 304 without a conditional request is not a refresh.
    assert!(matches!(classify(Raw { status: 304, ..Raw::default() }, false), Err(Failure::Unavailable { .. })));
}

#[test]
fn graphql_errors_are_not_documents() {
    let body = |errors: Value| Raw { status: 200, body: serde_json::to_vec(&json!({ "data": null, "errors": errors })).unwrap(), ..Raw::default() };
    assert!(matches!(classify(body(json!([{ "type": "NOT_FOUND", "message": "Could not resolve" }])), false), Err(Failure::NotFound)));
    assert!(matches!(classify(body(json!([{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }])), false), Err(Failure::Unavailable { .. })));
    assert!(matches!(map::pull(&garden(), 7, &json!({"html_url": "x"}), &json!({ "data": { "repository": { "pullRequest": null } } })), Err(Failure::NotFound)));
}

#[test]
fn path_values_are_typed() {
    for owner in ["a", "acme", "acme-garden", "A1", &"a".repeat(39)] {
        assert!(valid_owner(owner), "{owner}");
    }
    for owner in ["", "-acme", "acme_garden", "acme.garden", "a/b", &"a".repeat(40), "ácme"] {
        assert!(!valid_owner(owner), "{owner}");
    }
    for repo in ["garden", "garden.rs", ".garden", "a_b-c", &"r".repeat(100)] {
        assert!(valid_repo(repo), "{repo}");
    }
    for repo in ["", ".", "..", "a/b", "a b", "a%2f", &"r".repeat(101)] {
        assert!(!valid_repo(repo), "{repo}");
    }
    assert_eq!(parse_id("7", 10), Some(7));
    for id in ["0", "07", "-1", "+1", "1e3", "11", "", "https://github.com/acme/garden/pull/7"] {
        assert_eq!(parse_id(id, 10), None, "{id}");
    }
}

fn ok(body: Value, etag: Option<&str>) -> Result<Raw, Failure> {
    Ok(Raw { status: 200, etag: etag.map(str::to_owned), cacheable: true, body: serde_json::to_vec(&body).unwrap(), ..Raw::default() })
}

#[tokio::test]
async fn the_cache_revalidates_with_etags_and_serves_stale_only_for_upstream_failures() {
    let cache = std::sync::Mutex::new(Cache::default());
    let sends = AtomicUsize::new(0);
    let first = cached(&cache, "k", true, |etag| {
        sends.fetch_add(1, Ordering::SeqCst);
        assert_eq!(etag, None);
        async { ok(json!({ "n": 1 }), Some("\"v1\"")) }
    })
    .await
    .unwrap();
    assert_eq!(*first.value, json!({ "n": 1 }));
    // Fresh: no request.
    let again = cached(&cache, "k", true, |_| async { panic!("a fresh entry is not fetched") }).await.unwrap();
    assert!(!again.stale);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    cache.lock().unwrap().entries.get_mut("k").unwrap().fresh_until = Instant::now();
    // Expired: revalidated with the ETag; 304 refreshes.
    let refreshed = cached(&cache, "k", true, |etag| async move {
        assert_eq!(etag.as_deref(), Some("\"v1\""));
        Ok(Raw { status: 304, cacheable: true, ..Raw::default() })
    })
    .await
    .unwrap();
    assert_eq!(*refreshed.value, json!({ "n": 1 }));
    assert!(!refreshed.stale);
    cache.lock().unwrap().entries.get_mut("k").unwrap().fresh_until = Instant::now();
    // An upstream failure answers from the cache, stale.
    let stale = cached(&cache, "k", true, |_| async { Ok(Raw { status: 502, ..Raw::default() }) }).await.unwrap();
    assert!(stale.stale);
    assert_eq!(*stale.value, json!({ "n": 1 }));
    // GitHub's 404 is an answer: it is not hidden by the cache, and it evicts.
    assert!(matches!(cached(&cache, "k", true, |_| async { Ok(Raw { status: 404, ..Raw::default() }) }).await, Err(Failure::NotFound)));
    assert_eq!(cache.lock().unwrap().len(), 0);
    // Without a cached value an upstream failure is reported as such.
    assert!(matches!(cached(&cache, "k", true, |_| async { Ok(Raw { status: 502, ..Raw::default() }) }).await, Err(Failure::Unavailable { .. })));
    // TTL-only keys (GraphQL) never send an ETag.
    cached(&cache, "g", false, |_| async { ok(json!({ "data": {} }), Some("\"x\"")) }).await.unwrap();
    cache.lock().unwrap().entries.get_mut("g").unwrap().fresh_until = Instant::now();
    cached(&cache, "g", false, |etag| async move {
        assert_eq!(etag, None);
        ok(json!({ "data": {} }), None)
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_cache_keeps_its_own_entry_cap_and_skips_ineligible_answers() {
    let cache = std::sync::Mutex::new(Cache::default());
    for index in 0..CACHE_ENTRIES + 10 {
        cached(&cache, &format!("k{index}"), true, |_| async { ok(json!(index), None) }).await.unwrap();
    }
    assert_eq!(cache.lock().unwrap().len(), CACHE_ENTRIES);
    let ineligible = cached(&cache, "fallback", true, |_| async {
        Ok(Raw { status: 200, cacheable: false, body: b"{}".to_vec(), ..Raw::default() })
    })
    .await
    .unwrap();
    assert!(!ineligible.stale);
    assert!(!cache.lock().unwrap().entries.contains_key("fallback"));
}

#[tokio::test]
async fn routes_require_the_projection_scope_and_typed_paths() {
    use tower::ServiceExt as _;
    let app = |session: ClientSession| {
        Router::new()
            .route("/v1/client/plugins/github/{owner}/{repo}/pulls/{number}", get(pull))
            .route("/v1/client/plugins/github/{owner}/{repo}/comments/{comment_id}", get(comment))
            .layer(Extension(session))
    };
    let request = |uri: &str| axum::http::Request::builder().uri(uri).body(axum::body::Body::empty()).unwrap();
    let paired = ClientSession::for_tests("device/example", "person/operator", "paired");
    for uri in [
        "/v1/client/plugins/github/-acme/garden/pulls/7",
        "/v1/client/plugins/github/acme/../pulls/7",
        "/v1/client/plugins/github/acme/garden/pulls/0",
        "/v1/client/plugins/github/acme/garden/pulls/https%3A%2F%2Fexample.com",
        "/v1/client/plugins/github/acme/garden/comments/x",
    ] {
        let status = app(paired.clone()).oneshot(request(uri)).await.unwrap().status();
        assert!(status.is_client_error(), "{uri}: {status}");
    }
}

/// Canned GitHub: each request path answers the next response queued for it; any other is a test failure.
struct Canned(std::sync::Mutex<Vec<(&'static str, Raw)>>, AtomicUsize);

impl Canned {
    fn new(responses: Vec<(&'static str, Raw)>) -> Self {
        Self(std::sync::Mutex::new(responses), AtomicUsize::new(0))
    }

    fn fetch(&self, path: String) -> Result<Raw, Failure> {
        self.1.fetch_add(1, Ordering::SeqCst);
        let mut responses = self.0.lock().unwrap();
        let index = responses.iter().position(|(expected, _)| *expected == path).unwrap_or_else(|| panic!("unexpected GitHub request {path}"));
        Ok(responses.remove(index).1)
    }
}

fn redirect(location: &str) -> Raw {
    Raw { status: 301, location: Some(location.into()), cacheable: true, ..Raw::default() }
}

fn repository(full_name: &str) -> Raw {
    Raw { status: 200, cacheable: true, body: serde_json::to_vec(&json!({ "id": 42, "full_name": full_name })).unwrap(), ..Raw::default() }
}

async fn read(cache: &std::sync::Mutex<Cache>, canned: &Canned, path: &str) -> Result<Fetched, Failure> {
    rest_read(cache, &pat_auth(), path, |path, _| std::future::ready(canned.fetch(path))).await
}

#[tokio::test]
async fn a_renamed_repository_is_moved_with_the_requested_number() {
    let cache = std::sync::Mutex::new(Cache::default());
    let canned = Canned::new(vec![
        ("/repos/acme/garden/pulls/7", redirect("https://api.github.com/repositories/42/pulls/7")),
        ("/repositories/42", repository("acme/orchard")),
    ]);
    let moved = GithubMovedTo { owner: "acme".into(), repo: "orchard".into(), number: Some(7) };
    assert_eq!(read(&cache, &canned, "/repos/acme/garden/pulls/7").await.unwrap_err(), Failure::Moved(moved.clone()));
    // The move is remembered like any answer: the next read asks GitHub nothing.
    assert_eq!(read(&cache, &canned, "/repos/acme/garden/pulls/7").await.unwrap_err(), Failure::Moved(moved));
    assert_eq!(canned.1.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_transferred_repository_is_moved_to_its_new_owner() {
    let cache = std::sync::Mutex::new(Cache::default());
    let canned = Canned::new(vec![
        ("/repos/acme/garden/issues/9", redirect("https://api.github.com/repositories/42/issues/9")),
        ("/repositories/42", repository("acme-garden/garden")),
        ("/repos/acme/garden/issues/comments/1832451234", redirect("https://api.github.com/repositories/42/issues/comments/1832451234")),
        ("/repos/acme/garden/actions/runs/9001", redirect("https://api.github.com/repositories/42/actions/runs/9001")),
    ]);
    let to = |number| Failure::Moved(GithubMovedTo { owner: "acme-garden".into(), repo: "garden".into(), number });
    assert_eq!(read(&cache, &canned, "/repos/acme/garden/issues/9").await.unwrap_err(), to(Some(9)));
    // A comment or a run names no item number; the repository lookup is shared through the cache.
    assert_eq!(read(&cache, &canned, "/repos/acme/garden/issues/comments/1832451234").await.unwrap_err(), to(None));
    assert_eq!(read(&cache, &canned, "/repos/acme/garden/actions/runs/9001").await.unwrap_err(), to(None));
    assert_eq!(
        serde_json::to_value(to(Some(7)).into_read::<GithubPullRequest>()).unwrap(),
        fixture!("moved"),
    );
}

#[tokio::test]
async fn an_unresolvable_relocation_is_unavailable() {
    for (location, lookup) in [
        // A second redirect is never followed.
        ("https://api.github.com/repositories/42/pulls/7", Some(redirect("https://api.github.com/repositories/43"))),
        // GitHub cannot be read, or names something that is not a typed repository.
        ("https://api.github.com/repositories/42/pulls/7", Some(Raw { status: 502, ..Raw::default() })),
        ("https://api.github.com/repositories/42/pulls/7", Some(repository("acme/../etc"))),
        ("https://api.github.com/repositories/42/pulls/7", Some(repository("no-slash"))),
        // A non-numeric id or another host is never looked up.
        ("https://api.github.com/repositories/abc/pulls/7", None),
        ("https://api.github.com/repositories/0/pulls/7", None),
        ("https://example.com/repositories/42/pulls/7", None),
    ] {
        let cache = std::sync::Mutex::new(Cache::default());
        let mut responses = vec![("/repos/acme/garden/pulls/7", redirect(location))];
        responses.extend(lookup.map(|raw| ("/repositories/42", raw)));
        let canned = Canned::new(responses);
        let failure = read(&cache, &canned, "/repos/acme/garden/pulls/7").await.unwrap_err();
        assert!(matches!(failure, Failure::Unavailable { .. }), "{location}: {failure:?}");
        assert!(canned.0.lock().unwrap().is_empty(), "{location}: an expected request was not sent");
    }
}

fn pat_auth() -> crate::github_http::GithubAuth {
    crate::github_http::GithubAuth::test("fixture-pat")
}

fn installation(root: &std::path::Path, id: u64) -> crate::github_http::GithubAuth {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = root.join("gh-ci");
    std::fs::create_dir_all(&directory).unwrap();
    let token = directory.join("acme.token");
    std::fs::write(&token, "fixture-installation").unwrap();
    std::fs::set_permissions(token, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(directory.join("acme.json"), serde_json::to_vec(&json!({
        "installation_id": id,
        "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        "permissions": {"metadata": "read", "contents": "read", "pull_requests": "read"},
        "repository_selection": "selected"
    })).unwrap()).unwrap();
    crate::github_http::app_read::select(Some(root), "acme", "gh-ci", &pat_auth())
}

#[tokio::test]
async fn cache_principals_isolate_fresh_documents_etags_failures_and_relocations() {
    let cache = std::sync::Mutex::new(Cache::default());
    let root = tempfile::tempdir().unwrap();
    let pat = pat_auth();
    let path = "/repos/acme/garden/pulls/7";
    rest_read(&cache, &pat, path, |_, etag| async move {
        assert!(etag.is_none());
        ok(json!({"principal": "pat"}), Some("\"pat\""))
    }).await.unwrap();
    for id in [42, 43] {
        let auth = installation(root.path(), id);
        // Still inside the PAT/previous installation freshness window: selection must fetch,
        // and neither failure nor revalidation may reuse that principal's document or ETag.
        let calls = AtomicUsize::new(0);
        let result = rest_read(&cache, &auth, path, |_, etag| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert!(etag.is_none());
            std::future::ready(Err(Failure::unavailable("fixture upstream failure")))
        }).await;
        assert!(matches!(result, Err(Failure::Unavailable { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let item = rest_read(&cache, &auth, path, |_, etag| async move {
            assert!(etag.is_none());
            ok(json!({"installation": id}), Some("\"app\""))
        }).await.unwrap();
        assert_eq!(item.value["installation"], id);
    }
    let moved_path = "/repos/acme/garden/issues/9";
    for (auth, destination) in [(pat, "old"), (installation(root.path(), 44), "new"), (installation(root.path(), 45), "newest")] {
        let calls = AtomicUsize::new(0);
        let moved = rest_read(&cache, &auth, moved_path, |path, etag| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert!(etag.is_none());
            std::future::ready(Ok(if path == "/repositories/42" {
                repository(&format!("acme/{destination}"))
            } else {
                redirect("https://api.github.com/repositories/42/issues/9")
            }))
        }).await.unwrap_err();
        assert_eq!(moved, Failure::Moved(GithubMovedTo { owner: "acme".into(), repo: destination.into(), number: Some(9) }));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    let auth = installation(root.path(), 46);
    let failure = rest_read(&cache, &auth, moved_path, |path, etag| async move {
        assert!(etag.is_none());
        if path == "/repositories/42" { Err(Failure::unavailable("fixture lookup failure")) }
        else { Ok(redirect("https://api.github.com/repositories/42/issues/9")) }
    }).await.unwrap_err();
    assert!(matches!(failure, Failure::Unavailable { .. }));
    for (redirect_cacheable, lookup_cacheable) in [(false, true), (true, false), (false, false)] {
        let cache = std::sync::Mutex::new(Cache::default());
        let result = rest_read(&cache, &auth, moved_path, |path, _| async move {
            let mut raw = if path == "/repositories/42" { repository("acme/fallback") }
                else { redirect("https://api.github.com/repositories/42/issues/9") };
            raw.cacheable = if path == "/repositories/42" { lookup_cacheable } else { redirect_cacheable };
            Ok(raw)
        }).await.unwrap_err();
        assert!(matches!(result, Failure::Moved(_)));
        assert!(!cache.lock().unwrap().entries.contains_key(&format!("GET {} {moved_path}", auth.cache_principal())));
        assert_eq!(cache.lock().unwrap().len(), usize::from(lookup_cacheable), "ineligible fallback must not populate relocation document or moved caches");
    }
}

#[tokio::test]
async fn graphql_reads_select_installations_and_bound_uncached_pat_fallback() {
    use axum::http::{HeaderMap, StatusCode};
    let root = tempfile::tempdir().unwrap();
    let auth = installation(root.path(), 42);
    let cache = std::sync::Mutex::new(Cache::default());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route("/graphql", axum::routing::post({
        let requests = requests.clone();
        let mode = mode.clone();
        move |headers: HeaderMap, Json(body): Json<Value>| {
            let requests = requests.clone();
            let mode = mode.clone();
            async move {
                let bearer = headers["authorization"].to_str().unwrap().to_owned();
                requests.lock().unwrap().push((bearer.clone(), body));
                assert!(!headers.contains_key("if-none-match"));
                let accepted = match mode.load(Ordering::SeqCst) {
                    0 => bearer == "Bearer fixture-installation",
                    1 => bearer == "Bearer fixture-pat",
                    _ => return (StatusCode::BAD_GATEWAY, Json(json!({}))),
                };
                if accepted { (StatusCode::OK, Json(json!({"data": {"ok": true}}))) }
                else { (StatusCode::UNAUTHORIZED, Json(json!({}))) }
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for query in [map::PULL_QUERY, map::CONVERSATION_QUERY, map::CHECKS_QUERY] {
        let result = graphql_read(&cache, &auth, &base, "acme", query, graphql_variables(&garden(), 7), ROUTE_BUDGET).await.unwrap();
        assert_eq!(result.value["data"]["ok"], true);
    }
    assert_eq!(requests.lock().unwrap().len(), 3, "all three queries must use the installation, never the rejected PAT");
    // A changed installation must not hit a still-fresh GraphQL document.
    installation(root.path(), 43);
    mode.store(2, Ordering::SeqCst);
    for query in [map::PULL_QUERY, map::CONVERSATION_QUERY, map::CHECKS_QUERY] {
        assert!(matches!(graphql_read(&cache, &auth, &base, "acme", query, graphql_variables(&garden(), 7), ROUTE_BUDGET).await, Err(Failure::Unavailable { .. })));
    }
    assert_eq!(requests.lock().unwrap().len(), 6);
    mode.store(1, Ordering::SeqCst);
    cache.lock().unwrap().entries.clear();
    cache.lock().unwrap().bytes = 0;
    requests.lock().unwrap().clear();
    for query in [map::PULL_QUERY, map::CONVERSATION_QUERY, map::CHECKS_QUERY] {
        for _ in 0..2 {
            let result = graphql_read(&cache, &auth, &base, "acme", query, graphql_variables(&garden(), 8), ROUTE_BUDGET).await.unwrap();
            assert_eq!(result.value["data"]["ok"], true);
            assert_eq!(cache.lock().unwrap().len(), 0, "fallback must not cache PAT data under installation identity");
        }
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 18, "each read attempts the installation, one reload, then the PAT exactly once");
    for attempts in requests.chunks_exact(3) {
        assert_eq!(attempts[0].0, "Bearer fixture-installation");
        assert_eq!(attempts[1].0, "Bearer fixture-installation");
        assert_eq!(attempts[2].0, "Bearer fixture-pat");
        assert_eq!(attempts[0].1, attempts[2].1, "fallback must retain the fixed query and variables");
    }
    server.abort();
}

#[tokio::test]
async fn files_negotiate_the_client_limit_and_bound_multipage_escaped_patches_and_metadata() {
    let patch = "\"\\\0".repeat(2048);
    let pages = (0..2).map(|page| Arc::new(Value::Array((0..FILES_PAGE).map(|index| json!({
        "filename": format!("page-{page}/file-{index}"), "status": "modified",
        "additions": 1, "deletions": 0, "patch": patch
    })).collect()))).chain([Arc::new(json!([
        {"filename": "x".repeat(CLIENT_MAX_RESPONSE_BYTES), "status": "added", "additions": 1, "deletions": 0}
    ]))]).collect::<Vec<_>>();
    let raw_patch_bytes = patch.len() * 2 * FILES_PAGE;
    assert!(raw_patch_bytes > CLIENT_MAX_RESPONSE_BYTES && raw_patch_bytes < 4 * 1024 * 1024);
    let files = map::files(201, &pages, false).unwrap();
    assert!(files.truncated);
    assert!(files.files.iter().any(|file| file.patch.is_none()));
    assert!(files.files.iter().any(|file| file.patch.is_some()));
    assert!(files.files.len() < 201, "metadata must also fit, not only patches");
    let mut capabilities: Value = serde_json::from_str(/* sqlite-large-value: allow-for-now lv-h8fa94b508404 owner=fractal_api_client_v0::api::client_v0::github_plugin_reads::tests TODO=trace-statement-and-value-representation */ include_str!("../../../../../../docs/st3/client-v0/fixtures/capabilities.json")).unwrap();
    capabilities["value"]["limits"]["max_response_bytes"] = json!(CLIENT_MAX_RESPONSE_BYTES);
    let mut envelope = capabilities.clone();
    envelope["value"] = found(files, false);
    let encoded = serde_json::to_vec(&envelope).unwrap();
    assert!(encoded.len() <= CLIENT_MAX_RESPONSE_BYTES);
    let app = Router::new()
        .route("/v1/client/capabilities", get(move || {
            let capabilities = capabilities.clone();
            async move { Json(capabilities) }
        }))
        .route("/v1/client/plugins/github/acme/garden/pulls/7/files", get(move || {
            let envelope = envelope.clone();
            async move { Json(envelope) }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = fractal_client::Client::fabric_loopback(format!("http://{}", listener.local_addr().unwrap()), "fixture-credential");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let negotiated = client.capabilities().await.unwrap();
    assert_eq!(negotiated.value.limits.max_response_bytes, CLIENT_MAX_RESPONSE_BYTES);
    let response = client.github_plugin_files("acme", "garden", 7).await.unwrap();
    assert_eq!(response.value["item"]["truncated"], true);
    assert!(serde_json::to_vec(&response).unwrap().len() <= CLIENT_MAX_RESPONSE_BYTES);
    server.abort();
}
