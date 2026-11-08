use super::*;

fn parse(arguments: &[&str]) -> (bool, SubjectDesiredArgs) {
    let matches = command_graph().try_get_matches_from(arguments).unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    let Command::Subject { command: SubjectCommand::Desired(args) } = cli.command else {
        panic!("expected subject desired");
    };
    (cli.json, args)
}

#[test]
fn desired_accepts_exact_subjects_kind_and_global_json() {
    let (json, args) = parse(&[
        "fractal", "subject", "desired", "--json", "agent/example/worker",
        "mission/example", "--kind", "agent",
    ]);
    assert!(json);
    assert_eq!(args.subjects, ["agent/example/worker", "mission/example"]);
    assert_eq!(args.kind.as_deref(), Some("agent"));
    assert!(!args.all);
}

#[test]
fn desired_accepts_explicit_and_implicit_all() {
    for arguments in [
        vec!["fractal", "subject", "desired", "--all", "--kind", "mission"],
        vec!["fractal", "--json", "subject", "desired", "--kind", "mission"],
    ] {
        let (_, args) = parse(&arguments);
        assert!(args.subjects.is_empty());
        assert_eq!(args.kind.as_deref(), Some("mission"));
    }
    let (_, args) = parse(&["fractal", "subject", "desired"]);
    assert!(args.subjects.is_empty());
    assert!(!args.all);
    assert!(args.kind.is_none());
}

#[test]
fn desired_refuses_all_with_named_subjects() {
    for arguments in [
        vec!["fractal", "subject", "desired", "--all", "agent/example/worker"],
        vec!["fractal", "subject", "desired", "agent/example/worker", "--all"],
    ] {
        let error = command_graph().try_get_matches_from(arguments).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn desired_kind_requires_a_value() {
    assert!(command_graph().try_get_matches_from([
        "fractal", "subject", "desired", "--kind",
    ]).is_err());
}

#[tokio::test]
async fn desired_get_encodes_repeated_subjects_and_preserves_filters_on_continuation() {
    use axum::{Json, Router, extract::RawQuery, routing::get};

    let queries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let recorded = queries.clone();
    let app = Router::new().route(
        "/v1/desired",
        get(move |RawQuery(query): RawQuery| {
            let recorded = recorded.clone();
            async move {
                let mut queries = recorded.lock().await;
                queries.push(query.unwrap());
                Json(json!({
                    "api_version": "st3.v1", "value": {
                        "items": [],
                        "next_cursor": (queries.len() == 1).then_some("opaque+/=&cursor"),
                        "store_index": 42,
                    }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(Endpoint::Http(format!("http://{}", listener.local_addr().unwrap())));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (_, args) = parse(&[
        "fractal", "subject", "desired", "agent/café & ready?", "mission/a+b=#",
        "--kind", "kind &=?",
    ]);

    let result = subject_desired_items(&client, &args).await;
    server.abort();
    assert!(result.unwrap().is_empty());
    let selection = "limit=1000&subject=agent%2Fcaf%C3%A9%20%26%20ready%3F&subject=mission%2Fa%2Bb%3D%23&kind=kind%20%26%3D%3F";
    assert_eq!(*queries.lock().await, [
        selection.to_owned(),
        format!("{selection}&cursor=opaque%2B%2F%3D%26cursor"),
    ]);
}

#[tokio::test]
async fn desired_collects_more_than_one_page_before_returning_the_array() {
    use axum::{Json, Router, extract::RawQuery, routing::get};

    let queries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let recorded = queries.clone();
    let app = Router::new().route(
        "/v1/desired",
        get(move |RawQuery(query): RawQuery| {
            let recorded = recorded.clone();
            async move {
                let mut queries = recorded.lock().await;
                queries.push(query.unwrap());
                let first = queries.len() == 1;
                let items: Vec<Value> = (if first { 0..1000 } else { 1000..1001 })
                    .map(|index| json!({
                        "subject": format!("agent/example/{index:04}"),
                        "kind": "agent",
                        "desired": {},
                        "desired_token": null,
                    }))
                    .collect();
                Json(json!({
                    "api_version": "st3.v1", "value": {
                        "items": items,
                        "next_cursor": first.then_some("next-page"),
                        "store_index": 42,
                    }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(Endpoint::Http(format!("http://{}", listener.local_addr().unwrap())));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (_, args) = parse(&["fractal", "subject", "desired", "--all"]);

    let result = subject_desired_items(&client, &args).await;
    server.abort();
    let result = serde_json::to_value(result.unwrap()).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 1001);
    assert_eq!(result[0]["subject"], "agent/example/0000");
    assert_eq!(result[1000]["subject"], "agent/example/1000");
    assert_eq!(*queries.lock().await, [
        "limit=1000",
        "limit=1000&cursor=next-page",
    ]);
}

#[tokio::test]
async fn desired_cursor_invalidation_fails_without_restart_or_partial_result() {
    use axum::{Json, Router, extract::RawQuery, http::StatusCode, routing::get};

    const CHILD_ENDPOINT: &str = "ST3_TEST_DESIRED_CURSOR_ENDPOINT";
    if let Ok(endpoint) = std::env::var(CHILD_ENDPOINT) {
        let client = Client::new(Endpoint::Http(endpoint));
        let (_, args) = parse(&["fractal", "subject", "desired"]);
        let error = run_subject(&client, SubjectCommand::Desired(args), true).await.unwrap_err();
        assert_eq!(fractal_daemon::client::http_status(&error), Some(422));
        assert_eq!(fractal_daemon::client::api_error_code(&error), Some("desired-cursor-expired"));
        return;
    }

    let queries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let recorded = queries.clone();
    let app = Router::new().route(
        "/v1/desired",
        get(move |RawQuery(query): RawQuery| {
            let recorded = recorded.clone();
            async move {
                let mut queries = recorded.lock().await;
                queries.push(query.unwrap());
                if queries.len() == 1 {
                    (StatusCode::OK, Json(json!({
                        "api_version": "st3.v1", "value": {
                            "items": [{
                                "subject": "agent/example/partial",
                                "kind": "agent",
                                "desired": {},
                                "desired_token": null,
                            }],
                            "next_cursor": "pinned-cut",
                            "store_index": 42,
                        }
                    })))
                } else {
                    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({
                        "code": "desired-cursor-expired",
                        "message": "store cut changed",
                    })))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "subject_desired_cli_tests::desired_cursor_invalidation_fails_without_restart_or_partial_result",
                "--nocapture",
            ])
            .env(CHILD_ENDPOINT, endpoint)
            .output()
            .unwrap()
    }).await.unwrap();
    server.abort();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("agent/example/partial"));
    assert_eq!(*queries.lock().await, [
        "limit=1000",
        "limit=1000&cursor=pinned-cut",
    ]);
}

#[tokio::test]
async fn desired_all_preserves_newer_members_and_unavailable_items_in_json_output() {
    use axum::{Json, Router, routing::get};

    const CHILD_ENDPOINT: &str = "ST3_TEST_DESIRED_NEWER_MEMBER_ENDPOINT";
    if let Ok(endpoint) = std::env::var(CHILD_ENDPOINT) {
        let client = Client::new(Endpoint::Http(endpoint));
        let (_, args) = parse(&["fractal", "subject", "desired", "--all", "--json"]);
        run_subject(&client, SubjectCommand::Desired(args), true).await.unwrap();
        return;
    }

    let items = json!([
        {
            "subject": "agent/example/newer", "kind": "agent", "desired": {},
            "member": {"kind": "future-member-kind", "future_field": {"version": 2}},
            "desired_token": "newer-token",
            "actual": {"unavailable": {"code": "member-version-unproven"}},
        },
        {
            "subject": "agent/example/normal", "kind": "agent", "desired": {},
            "desired_token": "normal-token", "actual": {"revision": "normal-revision"},
        },
    ]);
    let response_items = items.clone();
    let app = Router::new().route("/v1/desired", get(move || {
        let items = response_items.clone();
        async move {
            Json(json!({
                "api_version": "st3.v1",
                "value": {"items": items, "next_cursor": null, "store_index": 42},
            }))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "subject_desired_cli_tests::desired_all_preserves_newer_members_and_unavailable_items_in_json_output",
                "--nocapture",
            ])
            .env(CHILD_ENDPOINT, endpoint)
            .output()
            .unwrap()
    }).await.unwrap();
    server.abort();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let array_start = stdout.find("[\n").expect("CLI prints a pretty JSON array");
    let mut values = serde_json::Deserializer::from_str(&stdout[array_start..])
        .into_iter::<Value>();
    assert_eq!(values.next().unwrap().unwrap(), items);
}
