//! Option D spike: loopback browser admission around the existing in-process client router.
use std::{collections::VecDeque, net::{Ipv4Addr, SocketAddr}, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::{Duration, Instant}};
use axum::{Router, body::Body, extract::{Request, State}, http::{header, Method, StatusCode}, middleware::{self, Next}, response::{IntoResponse, Response}};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use tokio::net::TcpListener;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:7410";
const COOKIE: &str = "fractal_local";
const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone)]
struct Web {
    port: u16,
    person: String,
    dist: PathBuf,
    policy: String,
    credential_path: PathBuf,
    credentials: Arc<Mutex<Credentials>>,
}
struct Credentials {
    token: String,
    sessions: VecDeque<(String, Instant)>,
}
fn secret() -> anyhow::Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("web random source: {error}"))?;
    Ok(hex::encode(bytes))
}
fn publish(path: &Path, port: u16, token: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("credential parent"))?;
    // tempfile creates the bearer rendezvous file with mode 0600, then atomically replaces it.
    write!(file, "http://127.0.0.1:{port}/?token={token}")?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `off` disables the listener. Assets are external so a Nix store path can be supplied.
pub async fn serve(listen: &str, dist: PathBuf, state: crate::api::AppState, person: Option<String>) -> anyhow::Result<()> {
    if listen == "off" { return std::future::pending().await; }
    let address: SocketAddr = listen.parse()?;
    anyhow::ensure!(address.ip() == Ipv4Addr::LOCALHOST && address.port() != 0, "web-listen requires 127.0.0.1 and a nonzero port");
    let person = person.ok_or_else(|| anyhow::anyhow!("local web requires a configured concrete person; use --web-listen off without one"))?;
    anyhow::ensure!(person.starts_with("person/") && person.matches('/').count() == 1 && person.len() > 7, "local web requires a concrete person");
    let _: axum::http::HeaderValue = person.parse()?;
    let dist = tokio::fs::canonicalize(dist).await?;
    let html = tokio::fs::read_to_string(dist.join("index.html")).await?;
    let listener = TcpListener::bind(address).await?;
    let token = secret()?;
    let credential_path = state.state_dir.join("web-launch-url");
    publish(&credential_path, address.port(), &token)?;
    let web = Web {
        port: address.port(), person, dist, policy: policy(&html, address.port()), credential_path,
        credentials: Arc::new(Mutex::new(Credentials { token, sessions: VecDeque::new() })),
    };
    // No forwarding to the privileged socket: the outer boundary permits only /v1/client/*.
    // Unix person admission is reused only after removing every caller-controlled credential.
    let app = crate::api::router(state)
        .fallback_service(Router::new().fallback(static_asset).with_state(web.clone()))
        .layer(middleware::from_fn_with_state(web, admission));
    eprintln!("fractal: local web listening at http://{address}; launch with fractal web");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn admission(State(web): State<Web>, mut request: Request, next: Next) -> Response {
    let host = request.headers().get(header::HOST).and_then(|value| value.to_str().ok()).unwrap_or("");
    if host != format!("127.0.0.1:{}", web.port) && host != format!("localhost:{}", web.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let origin = format!("http://{host}");
    let websocket = request.headers().contains_key(header::UPGRADE);
    if (websocket || !matches!(*request.method(), Method::GET | Method::HEAD))
        && request.headers().get(header::ORIGIN).and_then(|value| value.to_str().ok()) != Some(origin.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Never serve an administrative route or let a missing API route become an SPA response.
    let path = request.uri().path();
    if path.starts_with("/v1/") && !path.starts_with("/v1/client/") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let token = (request.method() == Method::GET && path == "/").then(|| request.uri().query())
        .flatten().and_then(|query| query.strip_prefix("token="));
    let cookie = request.headers().get(header::COOKIE).and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').find_map(|part| part.trim().strip_prefix("fractal_local=")));
    let result = {
        let mut credentials = web.credentials.lock().expect("web credentials poisoned");
        credentials.sessions.retain(|(_, issued)| issued.elapsed() < SESSION_LIFETIME);
        if let Some(token) = token {
            if token != credentials.token { return StatusCode::FORBIDDEN.into_response(); }
            let minted = secret().and_then(|session| {
                let next = secret()?;
                publish(&web.credential_path, web.port, &next)?;
                credentials.token = next;
                if credentials.sessions.len() == 32 { credentials.sessions.pop_front(); }
                credentials.sessions.push_back((session.clone(), Instant::now()));
                Ok(session)
            });
            match minted {
                Ok(session) => Some((StatusCode::SEE_OTHER, [
                    (header::LOCATION, "/".to_owned()),
                    (header::SET_COOKIE, format!("{COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200")),
                ]).into_response()),
                Err(_) => Some(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            }
        } else if !credentials.sessions.iter().any(|(session, _)| Some(session.as_str()) == cookie) {
            Some(StatusCode::UNAUTHORIZED.into_response())
        } else { None }
    };
    let mut response = if let Some(response) = result { response } else {
        request.headers_mut().remove(header::AUTHORIZATION);
        request.headers_mut().remove("x-fractal-person");
        request.headers_mut().insert("x-st3-person", web.person.parse().expect("validated person header"));
        next.run(request).await
    };
    response.headers_mut().insert("content-security-policy", web.policy.parse().expect("generated CSP"));
    response.headers_mut().insert("referrer-policy", "no-referrer".parse().unwrap());
    response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response.headers_mut().insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}

async fn static_asset(State(web): State<Web>, request: Request) -> Response {
    if request.uri().path().starts_with("/v1/") { return StatusCode::NOT_FOUND.into_response(); }
    if !matches!(*request.method(), Method::GET | Method::HEAD) { return StatusCode::METHOD_NOT_ALLOWED.into_response(); }
    let Ok(decoded) = urlencoding::decode(request.uri().path()) else { return StatusCode::BAD_REQUEST.into_response(); };
    if decoded.contains(['\0', '\\']) || decoded.split('/').any(|part| part == "..") { return StatusCode::NOT_FOUND.into_response(); }
    let mut path = web.dist.join(decoded.trim_start_matches('/'));
    if !path.is_file() {
        if Path::new(decoded.as_ref()).extension().is_some() { return StatusCode::NOT_FOUND.into_response(); }
        path = web.dist.join("index.html");
    }
    let Ok(path) = tokio::fs::canonicalize(path).await else { return StatusCode::NOT_FOUND.into_response(); };
    if !path.starts_with(&web.dist) { return StatusCode::NOT_FOUND.into_response(); }
    let content_type = match path.extension().and_then(|extension| extension.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8", "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8", "json" => "application/json", "svg" => "image/svg+xml",
        "png" => "image/png", "ico" => "image/x-icon", "woff2" => "font/woff2", "ttf" => "font/ttf",
        "wasm" => "application/wasm", "txt" => "text/plain; charset=utf-8", _ => "application/octet-stream",
    };
    let Ok(bytes) = tokio::fs::read(path).await else { return StatusCode::NOT_FOUND.into_response(); };
    let length = bytes.len();
    let body = if request.method() == Method::HEAD { Body::empty() } else { Body::from(bytes) };
    ([(header::CONTENT_TYPE, content_type.to_owned()), (header::CONTENT_LENGTH, length.to_string())], body).into_response()
}

// Mirrors csp.mts for trusted Vite output, not a general HTML parser. Build-time policy emission
// would avoid two implementations in production. Hash input follows browser CR normalization.
fn policy(html: &str, port: u16) -> String {
    let normalized = html.replace("\r\n", "\n").replace('\r', "\n");
    let hashes = |tag: &str| {
        let mut hashes = Vec::new();
        let mut rest = normalized.as_str();
        while let Some(start) = rest.find(&format!("<{tag}")) {
            rest = &rest[start + tag.len() + 1..];
            let Some(end) = rest.find('>') else { break; };
            let attributes = &rest[..end];
            rest = &rest[end + 1..];
            let Some(close) = rest.find(&format!("</{tag}>")) else { break; };
            let source = &rest[..close];
            if !source.is_empty() && !(tag == "script" && attributes.contains("src=")) {
                let hash = format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(Sha256::digest(source)));
                if !hashes.contains(&hash) { hashes.push(hash); }
            }
            rest = &rest[close + tag.len() + 3..];
        }
        hashes.join(" ")
    };
    format!("default-src 'self'; script-src 'self' 'wasm-unsafe-eval' {}; style-src 'self' 'sha256-38RhXrc7EdReTKsOm23ZPOCUgniTUUcjky8QOOrQx6o=' 'sha256-gYiS/BvZvRcK27JIXTuwhZ3hs2+VJ1X+2gUlE+farlg=' {}; style-src-attr 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self' ws://127.0.0.1:{port} ws://localhost:{port}; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'", hashes("script"), hashes("style"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn token_cookie_host_origin_and_client_boundary() {
        let root = tempfile::tempdir().unwrap();
        let web = Web {
            port: 7410, person: "person/prototype".into(), dist: root.path().into(),
            policy: policy("<script>one()</script><style>body{}</style>", 7410),
            credential_path: root.path().join("web-launch-url"),
            credentials: Arc::new(Mutex::new(Credentials { token: "once".into(), sessions: VecDeque::new() })),
        };
        let app = Router::new().fallback(|request: Request| async move {
            request.headers().get("x-st3-person").unwrap().to_str().unwrap().to_owned()
        }).layer(middleware::from_fn_with_state(web.clone(), admission));
        let call = |uri: &str, method: &str, host: &str, cookie: Option<&str>, origin: Option<&str>, upgrade: bool| {
            let mut request = Request::builder().uri(uri).method(method).header(header::HOST, host)
                .header("x-st3-person", "person/forged").header(header::AUTHORIZATION, "Bearer forged");
            if let Some(cookie) = cookie { request = request.header(header::COOKIE, cookie); }
            if let Some(origin) = origin { request = request.header(header::ORIGIN, origin); }
            if upgrade { request = request.header(header::UPGRADE, "websocket"); }
            app.clone().oneshot(request.body(Body::empty()).unwrap())
        };
        assert_eq!(call("/", "GET", "127.0.0.1:7410", None, None, false).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let login = call("/?token=once", "GET", "127.0.0.1:7410", None, None, false).await.unwrap();
        assert_eq!(login.status(), StatusCode::SEE_OTHER);
        let cookie = login.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap();
        assert!(login.headers()[header::SET_COOKIE].to_str().unwrap().contains("HttpOnly; SameSite=Strict"));
        assert_eq!(call("/?token=once", "GET", "127.0.0.1:7410", None, None, false).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_ne!(std::fs::read_to_string(&web.credential_path).unwrap(), "http://127.0.0.1:7410/?token=once");
        assert_eq!(call("/v1/client/agents", "GET", "evil.example:7410", Some(cookie), None, false).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(call("/v1/schema", "GET", "127.0.0.1:7410", Some(cookie), None, false).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(call("/v1/client/actions", "POST", "127.0.0.1:7410", Some(cookie), Some("http://evil.example"), false).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(call("/v1/client/events", "GET", "127.0.0.1:7410", Some(cookie), None, true).await.unwrap().status(), StatusCode::FORBIDDEN);
        let accepted = call("/v1/client/agents", "GET", "127.0.0.1:7410", Some(cookie), None, false).await.unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(accepted.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], b"person/prototype");
    }
}
