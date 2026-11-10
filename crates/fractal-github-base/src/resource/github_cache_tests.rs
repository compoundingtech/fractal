use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

async fn server(replies: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") { break; }
            }
            requests.push(String::from_utf8(request).unwrap().to_ascii_lowercase());
            stream.write_all(reply.as_bytes()).await.unwrap();
        }
        requests
    });
    (base, task)
}

fn reply(status: &str, headers: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

#[tokio::test]
async fn durable_validators_revalidate_after_restart_and_isolate_credentials() {
    let (base, requests) = server(vec![
        reply("200 OK", "ETag: \"one\"\r\nLast-Modified: Tue, 10 Nov 2026 17:00:00 GMT\r\n", "{\"head\":\"a\"}"),
        reply("304 Not Modified", "", ""),
        reply("200 OK", "ETag: \"two\"\r\n", "{\"head\":\"b\"}"),
        reply("304 Not Modified", "", ""),
        reply("200 OK", "ETag: \"other\"\r\n", "{\"head\":\"private\"}"),
    ]).await;
    let root = tempfile::tempdir().unwrap();
    let url = format!("{base}/repos/acme/garden");
    let auth = GithubAuth::test("credential-one");
    let client = github_client();
    let first = github_json_read_at(&client, url.clone(), &auth, Duration::ZERO, Some(root.path())).await.unwrap();
    let key = format!("{} {url}", auth.cache_identity());
    let disk = github_cache_path(root.path(), &key);
    let stored = load_github_payload(&disk).await.unwrap().unwrap();
    assert_eq!(stored.etag.as_deref(), Some("\"one\""));
    assert_eq!(stored.last_modified.as_deref(), Some("Tue, 10 Nov 2026 17:00:00 GMT"));
    // Removing only this entry models a fresh daemon with the same durable state.
    github_cache().lock().await.remove(&key);
    let second = github_json_read_at(&client, url.clone(), &auth, Duration::ZERO, Some(root.path())).await.unwrap();
    assert_eq!(first.value, second.value, "304 retains facts, so no changed observation");
    let third = github_json_read_at(&client, url.clone(), &auth, Duration::ZERO, Some(root.path())).await.unwrap();
    assert_eq!(third.value["head"], "b");
    assert_eq!(load_github_payload(&disk).await.unwrap().unwrap().etag.as_deref(), Some("\"two\""));
    let fourth = github_json_read_at(&client, url.clone(), &auth, Duration::ZERO, Some(root.path())).await.unwrap();
    assert_eq!(third.value, fourth.value);
    let other = GithubAuth::test("credential-two");
    assert_ne!(auth.cache_identity(), other.cache_identity());
    let private = github_json_read_at(&client, url, &other, Duration::ZERO, Some(root.path())).await.unwrap();
    assert_eq!(private.value["head"], "private");
    let requests = requests.await.unwrap();
    assert!(!requests[0].contains("if-none-match:"));
    assert!(requests[1].contains("if-none-match: \"one\""));
    assert!(requests[2].contains("if-none-match: \"one\""));
    assert!(requests[3].contains("if-none-match: \"two\""));
    assert!(!requests[4].contains("if-none-match:"));
}

#[tokio::test]
async fn last_modified_revalidates_without_etag() {
    let modified = "Tue, 10 Nov 2026 17:00:00 GMT";
    let (base, requests) = server(vec![
        reply("200 OK", &format!("Last-Modified: {modified}\r\n"), "{}"),
        reply("304 Not Modified", "", ""),
    ]).await;
    let auth = GithubAuth::test("modified-credential");
    for _ in 0..2 {
        github_json_read_at(&github_client(), base.clone(), &auth, Duration::ZERO, None).await.unwrap();
    }
    assert!(requests.await.unwrap()[1].contains(&format!("if-modified-since: {}", modified.to_ascii_lowercase())));
}

#[tokio::test]
async fn local_refusals_distinguish_secondary_from_primary_reset() {
    for (status, headers, delay) in [
        ("403 Forbidden", "x-ratelimit-remaining: 4200\r\nx-ratelimit-reset: 4102444800\r\nretry-after: 120\r\n", 120_000),
        ("429 Too Many Requests", "x-ratelimit-remaining: 4200\r\nx-ratelimit-reset: 4102444800\r\n", 60_000),
        ("403 Forbidden", "x-ratelimit-remaining: 4200\r\nx-ratelimit-reset: 4102444800\r\n", 60_000),
        ("403 Forbidden", "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: 4102444800\r\n", 0),
    ] {
        let (base, task) = server(vec![reply(status, &format!("{headers}x-ratelimit-resource: core\r\n"), "{}")]).await;
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let response = crate::github_http::send_read(github_client().get(base), &GithubAuth::test("rate-fixture")).await.unwrap();
        let error = github_response(response).await.unwrap_err();
        let limit = error.downcast_ref::<ProviderRateLimit>().unwrap();
        let details = error.downcast_ref::<crate::github_http::GithubResponseDetails>().unwrap();
        assert_eq!(details.resource.as_deref(), Some("core"));
        assert_eq!(details.status, if status.starts_with("403") { 403 } else { 429 });
        if delay == 0 { assert_eq!(limit.retry_at_unix_ms, 4_102_444_800_000); }
        else {
            let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
            assert!((before + delay..=after + delay).contains(&limit.retry_at_unix_ms));
        }
        task.await.unwrap();
    }
}

#[tokio::test]
async fn each_http_call_logs_wrapper_envelope_and_response_budget() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("gh-usage.jsonl");
    let (base, requests) = server(vec![
        reply("200 OK", "ETag: \"v1\"\r\nx-ratelimit-resource: core\r\nx-ratelimit-remaining: 42\r\n", "{}"),
        reply("304 Not Modified", "x-ratelimit-resource: core\r\nx-ratelimit-remaining: 42\r\n", ""),
        reply("403 Forbidden", "x-ratelimit-resource: graphql\r\nx-ratelimit-remaining: 100\r\nretry-after: 60\r\n", "{}"),
    ]).await;
    crate::github_http::TEST_USAGE_LOG.scope(log.clone(), async {
        let auth = GithubAuth::test("log-credential");
        for _ in 0..2 {
            github_json_read_at(&github_client(), base.clone(), &auth, Duration::ZERO, None).await.unwrap();
        }
        crate::github_http::send_read(github_client().post(format!("{base}/graphql"))
            .json(&json!({"query": "query { viewer { login } }"})), &auth).await.unwrap();
    }).await;
    requests.await.unwrap();
    let contents = std::fs::read_to_string(log).unwrap();
    let lines = contents.lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        for key in ["ts", "host", "invocation", "agent", "account", "category", "argv"] {
            assert!(line.get(key).is_some(), "wrapper field {key}");
        }
        assert!(chrono::DateTime::parse_from_rfc3339(line["ts"].as_str().unwrap()).is_ok());
        assert!(line["argv"].is_array());
    }
    assert_eq!(lines[0]["method"], "GET");
    assert_eq!(lines[0]["remaining"], 42);
    assert_eq!(lines[1]["status"], 304);
    assert_eq!(lines[1]["not_modified"], true);
    assert_eq!(lines[2]["path"], "/graphql");
    assert_eq!(lines[2]["resource"], "graphql");
    assert_eq!(lines[2]["remaining"], 100);
    assert_eq!(lines[2]["retry_after"], "60");
    assert!(!contents.contains("log-credential"), "credentials never belong in usage records");
}
