//! Human-terminal-only pairing transport. The root fingerprint is deliberately separate.
use anyhow::{Context as _, Result};
use chrono::{DateTime, Local};
use fractal_client::PairingChallenge;
use qrcode::{QrCode, render::unicode};
use serde::Serialize;

pub(super) fn parse_gateway(value: &str) -> Result<reqwest::Url, String> {
    let invalid = || "gateway must be an HTTP or HTTPS URL without a query or fragment".to_owned();
    let url = reqwest::Url::parse(value).map_err(|_| invalid())?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("gateway URL must not contain userinfo (username or password)".to_owned());
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

/// Reject a known different member offline, before contacting the gateway.
pub(super) fn check_begin_member(
    gateway: &reqwest::Url,
    local_member: &str,
    peers: &[fractal_daemon::config::PeerConfig],
) -> Result<()> {
    for peer in peers {
        let Ok(url) = parse_gateway(&peer.url) else { continue };
        if url.as_str().trim_end_matches('/') == gateway.as_str().trim_end_matches('/')
            && peer.name.trim_start_matches("host/") != local_member.trim_start_matches("host/")
        {
            anyhow::bail!(
                "pairing would begin on {local_member}, but the gateway names member {}; run the pair command on {}",
                peer.name, peer.name,
            );
        }
    }
    Ok(())
}

/// Probe public identity without sending person authority or a device credential.
/// Three seconds bounds gateway reachability only; it is not a pairing retry timeout.
pub(super) async fn gateway_is_begin_member(
    gateway: &reqwest::Url,
    local_member: &str,
) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("{}/v1/health", gateway.as_str().trim_end_matches('/'));
    let Ok(response) = client.get(url).send().await else { return Ok(false) };
    if response.status().is_redirection() { return Ok(false) }
    let Ok(health) = response.json::<fractal_client::NativeValue<serde_json::Value>>().await else { return Ok(false) };
    let Some(node) = health.value["node"].as_str().filter(|node| !node.is_empty()) else {
        return Ok(false);
    };
    if fractal_daemon::api::client_host_id(node) != local_member {
        anyhow::bail!(
            "pairing would begin on {local_member}, but the gateway serves member {node}; run the pair command on {node}",
        );
    }
    Ok(true)
}

fn payload(gateway: &reqwest::Url, challenge: &PairingChallenge, expires_at_unix_ms: i64) -> Result<String> {
    // A struct, rather than a JSON map, fixes the wire field order as well as the field set.
    #[derive(Serialize)]
    struct Payload<'a> {
        #[serde(rename = "_tag")]
        tag: &'static str,
        version: u8,
        gateway: &'a str,
        id: &'a str,
        code: &'a str,
        expires_at_unix_ms: i64,
    }
    serde_json::to_string(&Payload {
        tag: "fractal.pairing",
        version: 1,
        gateway: gateway.as_str(),
        id: &challenge.pairing_id,
        code: &challenge.code,
        expires_at_unix_ms,
    })
    .context("could not encode pairing QR")
}

pub(super) fn render(
    gateway: Option<&reqwest::Url>,
    challenge: &PairingChallenge,
    json_output: bool,
    stdout_is_terminal: bool,
    now_unix_ms: i64,
) -> Result<Option<String>> {
    // Do not even construct the secret payload for machine-readable or redirected output.
    let Some(gateway) = gateway.filter(|_| !json_output && stdout_is_terminal) else {
        return Ok(None);
    };
    let expires = DateTime::parse_from_rfc3339(&challenge.expires_at)
        .map_err(|_| anyhow::anyhow!("invalid pairing expiry from member"))?;
    let expires_ms = expires.timestamp_millis();
    let content = payload(gateway, challenge, expires_ms)?;
    let qr = QrCode::new(content.as_bytes()).context("could not encode pairing QR")?;
    // Force black modules on a white quiet zone, regardless of the terminal's theme.
    let image = qr.render::<unicode::Dense1x2>().quiet_zone(true).build();
    let local_expiry = expires.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S %:z");
    let seconds = expires_ms.saturating_sub(now_unix_ms).max(0) / 1_000;
    Ok(Some(format!(
        "\nScan with Fractal iOS (copy the fingerprint separately):\n\x1b[30;47m{image}\x1b[0m\nExpires: {local_expiry} ({seconds} seconds remaining).\n"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_other_member_is_refused_before_begin_but_unknown_gateway_is_allowed() {
        let peers = vec![fractal_daemon::config::PeerConfig {
            name: "member-b".into(), url: "https://member-b.example/base/".into(),
        }];
        let gateway = parse_gateway("https://MEMBER-B.example:443/base").unwrap();
        let error = check_begin_member(&gateway, "host/member-a", &peers).unwrap_err().to_string();
        assert!(error.contains("host/member-a"));
        assert!(error.contains("member-b"));
        assert!(error.contains("run the pair command on member-b"));
        assert!(check_begin_member(&gateway, "host/member-b", &peers).is_ok());
        assert!(check_begin_member(&parse_gateway("https://unknown.example").unwrap(), "host/member-a", &peers).is_ok());
        // A base path can route to a different daemon: don't infer identity from hostname alone.
        assert!(check_begin_member(&parse_gateway("https://member-b.example/other").unwrap(), "host/member-a", &peers).is_ok());
    }

    async fn health_server(body: String) -> (reqwest::Url, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = parse_gateway(&format!("http://{}/base/", listener.local_addr().unwrap())).unwrap();
        let served = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /base/v1/health HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            stream.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
        });
        (url, served)
    }

    async fn real_health_body(node: &str) -> String {
        use std::sync::Arc;
        use tower::ServiceExt as _;
        use fractal_daemon::api::AppState;
        let root = tempfile::tempdir().unwrap();
        let state = AppState {
            browser_pairing_login: None,
            seat_admission: Default::default(),
            store: Arc::new(fractal_daemon::store::Store::open_memory(node).unwrap()),
            notify: Arc::new(tokio::sync::Notify::new()),
            event_notify: tokio::sync::watch::channel(0_u64).0,
            node: node.into(), api_socket: root.path().join("api.sock"),state_dir: root.path().into(), pty_root: root.path().join("pty"),
            pty_binary: "pty".into(), owning_person: None, fleet_id: None,
            configured_peers: vec![], client_relay: None, native_session_home: None,
            planner_default: Default::default(), harness_question_sink: None,
            assistant_receipts: Default::default(), private_notes: Default::default(),
        };
        let response = fractal_daemon::api::router(state).oneshot(
            axum::http::Request::builder().uri("/v1/health")
                .body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert!(response.status().is_success());
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        let encoded: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(encoded["api_version"], "st3.v1");
        assert_eq!(encoded["value"]["node"], node);
        body
    }

    #[tokio::test]
    async fn gateway_health_matching_member_continues_without_warning() {
        let (gateway, served) = health_server(real_health_body("member-a").await).await;
        assert!(gateway_is_begin_member(&gateway, "host/member-a").await.unwrap());
        served.await.unwrap();
    }

    #[tokio::test]
    async fn gateway_health_normalizes_whitespace_with_the_shared_host_identity() {
        let (gateway, served) = health_server(real_health_body("member a\twest").await).await;
        assert!(gateway_is_begin_member(&gateway, "host/member-a-west").await.unwrap());
        served.await.unwrap();
    }

    #[tokio::test]
    async fn gateway_health_other_member_is_refused_even_without_a_configured_peer() {
        let (gateway, served) = health_server(real_health_body("member-b").await).await;
        let error = gateway_is_begin_member(&gateway, "host/member-a").await.unwrap_err().to_string();
        assert!(error.contains("host/member-a"));
        assert!(error.contains("member-b"));
        assert!(error.contains("run the pair command on member-b"));
        served.await.unwrap();
    }

    #[tokio::test]
    async fn gateway_health_without_node_or_unreachable_requires_warning() {
        let (gateway, served) = health_server(r#"{"status":"ready"}"#.into()).await;
        assert!(!gateway_is_begin_member(&gateway, "host/member-a").await.unwrap());
        served.await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = parse_gateway(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        assert!(!gateway_is_begin_member(&gateway, "host/member-a").await.unwrap());
    }

    #[tokio::test]
    async fn gateway_health_redirect_warns_without_contacting_the_destination() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = parse_gateway(&format!("http://{}", first.local_addr().unwrap())).unwrap();
        let location = format!("http://{}/v1/health", destination.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let (mut stream, _) = first.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            stream.write_all(format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ).as_bytes()).await.unwrap();
        });
        assert!(!gateway_is_begin_member(&gateway, "host/member-a").await.unwrap());
        served.await.unwrap();
        // A connect would already be queued by the time the completed request returned.
        assert!(tokio::time::timeout(std::time::Duration::from_millis(100), destination.accept()).await.is_err());
    }

    fn challenge() -> PairingChallenge {
        serde_json::from_value(serde_json::json!({
            "kind": "pairing-challenge",
            "pairing_id": "pairing/0123456789abcdef01234567",
            "code": "ABCD2345",
            "expires_at": "2026-11-06T15:00:00Z",
            "person_root_fingerprint": "must-not-be-in-qr"
        })).unwrap()
    }

    #[test]
    fn pairing_qr_payload_is_exact_and_has_no_fingerprint() {
        let gateway = parse_gateway("https://example.test/base").unwrap();
        assert_eq!(
            payload(&gateway, &challenge(), 1_799_999_999_000).unwrap(),
            r#"{"_tag":"fractal.pairing","version":1,"gateway":"https://example.test/base","id":"pairing/0123456789abcdef01234567","code":"ABCD2345","expires_at_unix_ms":1799999999000}"#
        );
    }

    #[test]
    fn pairing_qr_is_only_rendered_for_human_terminals_with_gateway() {
        let gateway = parse_gateway("http://100.64.0.1:8080/base").unwrap();
        let challenge = challenge();
        for (json, tty) in [(true, true), (true, false), (false, false)] {
            assert!(render(Some(&gateway), &challenge, json, tty, 0).unwrap().is_none());
        }
        assert!(render(None, &challenge, false, true, 0).unwrap().is_none());
        let expires_ms = DateTime::parse_from_rfc3339(&challenge.expires_at).unwrap().timestamp_millis();
        let rendered = render(Some(&gateway), &challenge, false, true, expires_ms - 300_000)
            .unwrap().unwrap();
        assert!(rendered.contains("Scan with Fractal iOS"));
        assert!(rendered.contains("300 seconds remaining"));
        assert!(!rendered.contains("must-not-be-in-qr"));
    }

    #[test]
    fn pairing_qr_rejects_bad_gateways_without_echoing_them() {
        for value in ["not a url", "ftp://example.test", "https://example.test/?secret=code", "https://example.test/#secret", "https://example.test/?", "https://example.test/#"] {
            let error = parse_gateway(value).unwrap_err();
            assert!(!error.contains(value));
        }
        assert_eq!(parse_gateway("HTTPS://EXAMPLE.TEST:443/base").unwrap().as_str(), "https://example.test/base");
    }

    #[test]
    fn pairing_qr_rejects_gateway_userinfo_without_echoing_credentials() {
        for value in [
            "https://user:password@example.test",
            "https://user@example.test",
            "https://:password@example.test",
        ] {
            let error = parse_gateway(value).unwrap_err();
            assert_eq!(error, "gateway URL must not contain userinfo (username or password)");
            assert!(!error.contains(value));
        }
    }

    #[test]
    fn pairing_qr_gateway_flag_rejects_invalid_urls_and_assistant_pairing() {
        use clap::Parser as _;
        for args in [
            vec!["fractal", "devices", "pair", "phone", "--gateway", "ftp://example.test"],
            vec!["fractal", "devices", "pair", "phone", "--gateway", "https://example.test/?q=1"],
            vec!["fractal", "devices", "pair", "--assistant", "assistant/chatgpt", "--gateway", "https://example.test"],
        ] {
            assert!(crate::Cli::try_parse_from(args).is_err());
        }
        assert!(crate::Cli::try_parse_from([
            "fractal", "devices", "pair", "phone", "--gateway", "https://example.test/base", "--json"
        ]).is_ok());
    }
}
