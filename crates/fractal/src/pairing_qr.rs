//! Human-terminal-only pairing transport. The root fingerprint is deliberately separate.
use anyhow::{Context as _, Result};
use chrono::{DateTime, Local};
use fractal_client::PairingChallenge;
use qrcode::{QrCode, render::unicode};
use serde::Serialize;

pub(super) fn parse_gateway(value: &str) -> Result<reqwest::Url, String> {
    let invalid = || "gateway must be an HTTP or HTTPS URL without a query or fragment".to_owned();
    let url = reqwest::Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

/// Configuration can identify a different member without sending local person authority
/// to the gateway. Unknown URLs remain the server's completion-origin check to decide.
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
