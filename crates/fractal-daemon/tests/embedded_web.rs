use fractal_daemon::{api, web};
use reqwest::{Client, StatusCode, header};

#[tokio::test]
async fn browser_admission_matrix() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("index.html"), "<!doctype html><title>Fractal</title>").unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let prepared = web::prepare(&address.to_string(), root.path().into(), root.path(), Some("person/robin".into())).await.unwrap();
    let launch = std::fs::read_to_string(root.path().join("web-launch-url")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(root.path().join("web-launch-url")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let server = tokio::spawn(prepared.serve(api::tests_shared::state(root.path())));
    let client = Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let url = format!("http://{address}");
    assert_eq!(client.get(&url).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let login = client.get(&launch).send().await.unwrap();
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let set_cookie = login.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly; SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap();
    assert_eq!(client.get(&launch).send().await.unwrap().status(), StatusCode::FORBIDDEN);
    let get = |path: &str| client.get(format!("{url}{path}")).header(header::COOKIE, cookie);
    assert_eq!(get("/").header(header::HOST, "forged.invalid").send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(get("/").header(header::ORIGIN, "https://forged.invalid").send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(get("/v1/schema").send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(get("/v1/client/events").header(header::UPGRADE, "websocket").send().await.unwrap().status(), StatusCode::FORBIDDEN);
    let page = get("/").send().await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.headers()["content-security-policy"].to_str().unwrap().contains("frame-ancestors 'none'"));
    // A missing client route remains an API error, never the SPA or a privileged route.
    assert_eq!(get("/v1/client/not-a-route").send().await.unwrap().status(), StatusCode::NOT_FOUND);
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let mut upgrade = format!("ws://{address}/v1/client/collections/stream").into_client_request().unwrap();
    upgrade.headers_mut().insert(header::COOKIE, cookie.parse().unwrap());
    upgrade.headers_mut().insert(header::ORIGIN, url.parse().unwrap());
    let (socket, response) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    drop(socket);
    server.abort();
}

#[tokio::test]
async fn missing_assets_or_person_only_degrades_browser_listener() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    for (dist, person) in [
        (root.path().join("missing"), Some("person/robin".into())),
        (root.path().to_path_buf(), None),
    ] {
        std::fs::write(root.path().join("web-launch-url"), "stale").unwrap();
        let prepared = web::prepare(web::DEFAULT_LISTEN, dist, root.path(), person).await.unwrap();
        assert_eq!(prepared.status.0.name, "local-web");
        assert_eq!(prepared.status.0.status, "warn");
        assert!(!root.path().join("web-launch-url").exists());
        let app = api::router(api::tests_shared::state(root.path()));
        let response = app.oneshot(Request::builder().uri("/v1/schema").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
