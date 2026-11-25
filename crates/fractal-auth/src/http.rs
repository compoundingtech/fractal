use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use http::{HeaderMap, HeaderValue};
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

use crate::{MemberKey, Sender, verify_signature};

const PROTOCOL: &str = "st3-replication-v1";
// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction.
// Senders keep x-st3-* until every member accepts aliases; sunset 60 days after
// the first shipping release. Signature domains and canonical bytes do not change.
const HEADER_FLEET: &str = "x-st3-fleet";
const HEADER_NODE: &str = "x-st3-node";
const HEADER_BODY: &str = "x-st3-body-sha256";
const HEADER_SIGNATURE: &str = "x-st3-signature";
const HEADER_REQUEST: &str = "x-st3-request-digest";
const HEADER_MEMBER_KEY: &str = "x-st3-member-key";
const HEADER_MEMBER_SIGNATURE: &str = "x-st3-member-signature";
const HEADER_ALIASES: [(&str, &str); 7] = [
    (HEADER_FLEET, "x-fractal-fleet"),
    (HEADER_NODE, "x-fractal-node"),
    (HEADER_BODY, "x-fractal-body-sha256"),
    (HEADER_SIGNATURE, "x-fractal-signature"),
    (HEADER_REQUEST, "x-fractal-request-digest"),
    (HEADER_MEMBER_KEY, "x-fractal-member-key"),
    (HEADER_MEMBER_SIGNATURE, "x-fractal-member-signature"),
];
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both
const MEMBER_SIGNATURE_DOMAIN: &str = "st3-member-v1";
#[cfg(any(test, feature = "test-support"))]
const EXCHANGE_PATH: &str = "/v1/peer/exchange";

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — DELETE at contraction.
// Accept both spellings until contraction, 60 days after the first shipping release;
// switch senders only after every member accepts aliases.
fn signed_fields(headers: &HeaderMap) -> Result<[Option<&str>; 7]> {
    let mut fields = [None; 7];
    for ((old, new), field) in HEADER_ALIASES.into_iter().zip(&mut fields) {
        let mut values = headers.get_all(old).iter().chain(headers.get_all(new).iter());
        if let Some(first) = values.next() {
            anyhow::ensure!(
                values.all(|value| value == first),
                "the signed message has conflicting {old}/{new} headers"
            );
            *field = first.to_str().ok();
        }
    }
    Ok(fields)
}
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct FleetAuth {
    fleet_id: String,
    secret: Arc<Vec<u8>>,
    /// This member's key. Set, every request and response also carries a member signature.
    member: Option<Arc<MemberKey>>,
}

impl FleetAuth {
    pub fn load(fleet_id: &str, path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("inspect the fleet secret {}", path.display()))?;
        anyhow::ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "the fleet secret must not grant group or other permissions"
        );
        let bytes =
            fs::read(path).with_context(|| format!("read the fleet secret {}", path.display()))?;
        Self::from_secret_bytes(fleet_id, bytes)
    }

    /// Parse secret-file bytes already read through the caller's file-access boundary.
    /// Accept the existing raw 32-byte or trimmed hexadecimal 64-character representation.
    pub fn from_secret_bytes(fleet_id: &str, bytes: Vec<u8>) -> Result<Self> {
        let hexadecimal = std::str::from_utf8(&bytes)
            .ok()
            .map(str::trim)
            .filter(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
        let secret = match hexadecimal {
            Some(value) => hex::decode(value).context("decode the hexadecimal fleet secret")?,
            None => bytes,
        };
        anyhow::ensure!(secret.len() == 32, "the fleet secret must contain 32 bytes");
        Ok(Self {
            fleet_id: fleet_id.into(),
            secret: Arc::new(secret),
            member: None,
        })
    }

    /// Sign every request and response with this member key as well.
    pub fn with_member_key(mut self, member: Option<Arc<MemberKey>>) -> Self {
        self.member = member;
        self
    }

    pub fn member_key(&self) -> Option<&str> {
        self.member.as_deref().map(MemberKey::public)
    }

    /// The fleet secret as the hexadecimal text a secret file holds.
    pub fn secret_hex(&self) -> String {
        hex::encode(self.secret.as_slice())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test(fleet_id: &str, secret: &[u8]) -> Self {
        Self {
            fleet_id: fleet_id.into(),
            secret: Arc::new(secret.to_vec()),
            member: None,
        }
    }

    fn canonical(
        &self,
        method: &str,
        path: &str,
        node: &str,
        body_digest: &str,
        request_digest: Option<&str>,
    ) -> String {
        format!(
            "{PROTOCOL}\n{method}\n{path}\n{}\n{node}\n{body_digest}\n{}",
            self.fleet_id,
            request_digest.unwrap_or_default()
        )
    }

    fn member_message(canonical: &str) -> Vec<u8> {
        format!("{MEMBER_SIGNATURE_DOMAIN}\n{canonical}").into_bytes()
    }

    // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — sender hold.
    // Keep old names until every member accepts aliases; sunset 60 days after first shipping release.
    // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both
    fn add_member_signature(&self, headers: &mut HeaderMap, canonical: &str) -> Result<()> {
        if let Some(member) = &self.member {
            headers.insert(HEADER_MEMBER_KEY, HeaderValue::from_str(member.public())?);
            headers.insert(
                HEADER_MEMBER_SIGNATURE,
                HeaderValue::from_str(&member.sign(&Self::member_message(canonical)))?,
            );
        }
        Ok(())
    }

    pub fn fleet_id(&self) -> &str {
        &self.fleet_id
    }

    pub fn body_digest(body: &[u8]) -> String {
        hex::encode(Sha256::digest(body))
    }

    fn signature(
        &self,
        method: &str,
        path: &str,
        node: &str,
        body_digest: &str,
        request_digest: Option<&str>,
    ) -> String {
        let canonical = self.canonical(method, path, node, body_digest, request_digest);
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts a secret of any length");
        mac.update(canonical.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn request_headers(&self, node: &str, body: &[u8]) -> Result<HeaderMap> {
        self.request_headers_for(EXCHANGE_PATH, node, body)
    }

    pub fn request_headers_for(&self, path: &str, node: &str, body: &[u8]) -> Result<HeaderMap> {
        self.request_headers_method("POST", path, node, body)
    }

    pub fn request_headers_method(
        &self,
        method: &str,
        path: &str,
        node: &str,
        body: &[u8],
    ) -> Result<HeaderMap> {
        let digest = Self::body_digest(body);
        let signature = self.signature(method, path, node, &digest, None);
        let mut headers = headers(&self.fleet_id, node, &digest, &signature, None)?;
        self.add_member_signature(
            &mut headers,
            &self.canonical(method, path, node, &digest, None),
        )?;
        Ok(headers)
    }

    pub fn response_headers_for(
        &self,
        path: &str,
        node: &str,
        body: &[u8],
        request_digest: &str,
    ) -> Result<HeaderMap> {
        let digest = Self::body_digest(body);
        let signature = self.signature("RESPONSE", path, node, &digest, Some(request_digest));
        let mut headers = headers(
            &self.fleet_id,
            node,
            &digest,
            &signature,
            Some(request_digest),
        )?;
        self.add_member_signature(
            &mut headers,
            &self.canonical("RESPONSE", path, node, &digest, Some(request_digest)),
        )?;
        Ok(headers)
    }

    pub fn verify(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
        body: &[u8],
        expected_node: Option<&str>,
        request_digest: Option<&str>,
    ) -> Result<String> {
        self.verify_sender(headers, method, path, body, expected_node, request_digest)
            .map(|sender| sender.name)
    }

    /// Check the fleet HMAC, then read the optional member key and check its signature.
    pub fn verify_sender(
        &self,
        headers: &HeaderMap,
        method: &str,
        path: &str,
        body: &[u8],
        expected_node: Option<&str>,
        request_digest: Option<&str>,
    ) -> Result<Sender> {
        fn field<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
            value.with_context(|| format!("the signed message has no valid {name} header"))
        }
        let [fleet, node, body_hash, signature, request, member_key, member_signature] =
            signed_fields(headers)?;
        anyhow::ensure!(
            field(fleet, HEADER_FLEET)? == self.fleet_id,
            "the fleet ID does not match"
        );
        let node = field(node, HEADER_NODE)?;
        if let Some(expected) = expected_node {
            anyhow::ensure!(
                node == expected,
                "the signed node does not match the configured peer"
            );
        }
        let digest = Self::body_digest(body);
        anyhow::ensure!(
            field(body_hash, HEADER_BODY)? == digest,
            "the signed body digest does not match"
        );
        if let Some(expected) = request_digest {
            anyhow::ensure!(
                field(request, HEADER_REQUEST)? == expected,
                "the response does not bind to this request"
            );
        }
        let signature = hex::decode(field(signature, HEADER_SIGNATURE)?)
            .context("the replication signature is not hexadecimal")?;
        let canonical = self.canonical(method, path, node, &digest, request_digest);
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts a secret of any length");
        mac.update(canonical.as_bytes());
        mac.verify_slice(&signature)
            .context("the replication signature does not match")?;
        let member_key = member_key.map(str::to_owned);
        let member_signature_valid = match (&member_key, member_signature) {
            (Some(key), Some(signature)) => {
                verify_signature(key, &Self::member_message(&canonical), signature)
            }
            _ => false,
        };
        Ok(Sender {
            name: node.into(),
            member_key,
            member_signature_valid,
        })
    }
}

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both — sender hold.
// Keep old names until every member accepts aliases; sunset 60 days after first shipping release.
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-wire-fractal-accept-both
fn headers(
    fleet: &str,
    node: &str,
    digest: &str,
    signature: &str,
    request_digest: Option<&str>,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in [
        (HEADER_FLEET, fleet),
        (HEADER_NODE, node),
        (HEADER_BODY, digest),
        (HEADER_SIGNATURE, signature),
    ] {
        headers.insert(name, HeaderValue::from_str(value)?);
    }
    if let Some(request_digest) = request_digest {
        headers.insert(HEADER_REQUEST, HeaderValue::from_str(request_digest)?);
    }
    Ok(headers)
}

#[cfg(test)]
mod header_tests {
    use super::*;

    fn signed_response() -> (FleetAuth, HeaderMap) {
        let member = Arc::new(MemberKey::generate().unwrap().0);
        let auth = FleetAuth::test("fleet", &[9; 32]).with_member_key(Some(member));
        let headers = auth
            .response_headers_for(EXCHANGE_PATH, "peer", b"body", "request")
            .unwrap();
        (auth, headers)
    }

    fn verify(auth: &FleetAuth, headers: &HeaderMap) -> Result<Sender> {
        auth.verify_sender(
            headers, "RESPONSE", EXCHANGE_PATH, b"body", Some("peer"), Some("request"),
        )
    }

    #[test]
    fn signed_headers_accept_old_new_mixed_and_equal_duplicates() {
        let (auth, old) = signed_response();
        assert_eq!(old.len(), HEADER_ALIASES.len());
        for (name, alias) in HEADER_ALIASES {
            assert!(old.contains_key(name), "sender must keep {name}");
            assert!(!old.contains_key(alias), "sender must not emit {alias}");
        }
        let mut new = HeaderMap::new();
        let mut mixed = HeaderMap::new();
        let mut both = old.clone();
        for (index, (name, alias)) in HEADER_ALIASES.into_iter().enumerate() {
            let value = old.get(name).unwrap();
            new.insert(alias, value.clone());
            mixed.insert(if index % 2 == 0 { name } else { alias }, value.clone());
            both.append(name, value.clone());
            both.append(alias, value.clone());
            both.append(alias, value.clone());
        }
        for headers in [&old, &new, &mixed, &both] {
            let sender = verify(&auth, headers).unwrap();
            assert_eq!(sender.name, "peer");
            assert_eq!(sender.member_key.as_deref(), auth.member_key());
            assert!(sender.member_signature_valid);
            assert!(auth.verify_sender(
                headers, "RESPONSE", EXCHANGE_PATH, b"tampered", Some("peer"), Some("request"),
            ).is_err());
            assert!(auth.verify_sender(
                headers, "RESPONSE", EXCHANGE_PATH, b"body", Some("peer"), Some("other"),
            ).is_err());
        }
    }

    #[test]
    fn signed_headers_reject_every_conflicting_alias_or_duplicate() {
        let (auth, old) = signed_response();
        for (name, alias) in HEADER_ALIASES {
            for duplicate_name in [name, alias] {
                let mut headers = old.clone();
                if duplicate_name == alias {
                    let value = headers.remove(name).unwrap();
                    headers.insert(alias, value);
                }
                headers.append(duplicate_name, HeaderValue::from_static("conflict"));
                let error = verify(&auth, &headers).unwrap_err().to_string();
                assert!(error.contains("conflicting"), "{name}: {error}");
            }
            let mut headers = old.clone();
            headers.insert(alias, HeaderValue::from_static("conflict"));
            let error = verify(&auth, &headers).unwrap_err().to_string();
            assert!(error.contains("conflicting"), "{name}: {error}");
        }
    }

    #[test]
    fn requests_reject_conflicting_unused_request_digest() {
        let (auth, _) = signed_response();
        let mut headers = auth.request_headers("peer", b"body").unwrap();
        headers.insert(HEADER_REQUEST, HeaderValue::from_static("one"));
        headers.insert("x-fractal-request-digest", HeaderValue::from_static("two"));
        let error = auth.verify(
            &headers, "POST", EXCHANGE_PATH, b"body", Some("peer"), None,
        ).unwrap_err().to_string();
        assert!(error.contains("conflicting"), "{error}");
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;

    #[test]
    fn raw_and_trimmed_hexadecimal_secrets_sign_identically() {
        let raw = vec![9; 32];
        let hexadecimal = format!(" \n{}\n", hex::encode(&raw));
        let raw = FleetAuth::from_secret_bytes("fleet", raw).unwrap();
        let hexadecimal =
            FleetAuth::from_secret_bytes("fleet", hexadecimal.into_bytes()).unwrap();
        let headers = raw.request_headers_for("/session", "peer", b"body").unwrap();
        assert_eq!(
            hexadecimal
                .verify(&headers, "POST", "/session", b"body", Some("peer"), None)
                .unwrap(),
            "peer"
        );
    }

    #[test]
    fn secret_bytes_reject_invalid_lengths_and_non_hexadecimal_text() {
        for bytes in [vec![], vec![9; 31], vec![9; 33], vec![b'x'; 64]] {
            assert!(
                FleetAuth::from_secret_bytes("fleet", bytes)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("32 bytes")
            );
        }
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn signed_messages_detect_tampering_and_wrong_fleets() {
        let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[7; 32]);
        let body = br#"{"hello":"fleet"}"#;
        let headers = auth.request_headers("node-a", body).unwrap();
        assert!(
            auth.verify(&headers, "POST", EXCHANGE_PATH, body, Some("node-a"), None)
                .is_ok()
        );
        assert!(
            auth.verify(
                &headers,
                "POST",
                EXCHANGE_PATH,
                br#"{"hello":"other"}"#,
                Some("node-a"),
                None
            )
            .is_err()
        );
        let other = FleetAuth::test("48608b46-bf75-442a-a462-787085dc574e", &[7; 32]);
        assert!(
            other
                .verify(&headers, "POST", EXCHANGE_PATH, body, Some("node-a"), None)
                .is_err()
        );
    }

    #[test]
    fn response_signatures_bind_to_one_request() {
        let auth = FleetAuth::test("1f91ca65-7793-48cc-866e-ac15690130e1", &[9; 32]);
        let body = b"response";
        let headers = auth
            .response_headers_for(EXCHANGE_PATH, "node-b", body, "request-a")
            .unwrap();
        assert!(
            auth.verify(
                &headers,
                "RESPONSE",
                EXCHANGE_PATH,
                body,
                Some("node-b"),
                Some("request-a")
            )
            .is_ok()
        );
        assert!(
            auth.verify(
                &headers,
                "RESPONSE",
                EXCHANGE_PATH,
                body,
                Some("node-b"),
                Some("request-b")
            )
            .is_err()
        );
    }

    #[test]
    fn fleet_secret_loading_accepts_private_raw_or_hex_files_only() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fleet-secret");
        fs::write(&path, [3_u8; 32]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).unwrap();

        fs::write(&path, format!("{}\n", hex::encode([4_u8; 32]))).unwrap();
        FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).unwrap();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(FleetAuth::load("1f91ca65-7793-48cc-866e-ac15690130e1", &path).is_err());
    }
}
