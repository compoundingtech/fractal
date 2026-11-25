//! Canonical signed local request assertions; graph admission stays at home.
use super::{ExecutionBinding, FractalError, internal, local};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use fractal_auth::MemberKey;

const DOMAIN: &[u8] = b"fractal.light-node.run-request.v1\0";
pub const MAX_ASSERTION_LIFETIME_MS: u64 = 30_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestIdentity {
    pub request_id: String,
    pub method: String,
    /// The exact HTTP origin-form path and query, before any actor/body rewriting.
    pub target: String,
    pub payload_sha256: String,
}

impl RequestIdentity {
    pub fn new(request_id: String, method: String, target: String, payload: &[u8]) -> Self {
        Self { request_id, method, target, payload_sha256: hex::encode(Sha256::digest(payload)) }
    }

    pub fn validate(&self) -> Result<(), FractalError> {
        if self.request_id.is_empty() || self.request_id.len() > 128
            || self.method.is_empty() || !self.method.bytes().all(|byte| byte.is_ascii_uppercase())
            || !self.target.starts_with('/') || self.target.starts_with("//")
            || self.target.bytes().any(|byte| byte.is_ascii_control() || byte == b' ' || byte == b'#')
            || self.payload_sha256.len() != 64
            || !self.payload_sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        { return Err(refused("the request identity is not canonical")); }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAssertion {
    pub version: u32,
    pub binding: ExecutionBinding,
    pub request: RequestIdentity,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub signature: String,
}

fn refused(reason: &str) -> FractalError {
    FractalError::new("run-request-refused", reason)
}

impl RequestAssertion {
    pub fn message(&self) -> Result<Vec<u8>, FractalError> {
        let mut bytes = DOMAIN.to_vec();
        serde_json::to_writer(&mut bytes, &(
            self.version, &self.binding, &self.request,
            self.issued_at_unix_ms, self.expires_at_unix_ms,
        )).map_err(internal)?;
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), FractalError> {
        self.binding.validate()?;
        self.request.validate()?;
        if self.version != 1 || self.binding.run.is_none()
            || self.expires_at_unix_ms <= self.issued_at_unix_ms
            || self.expires_at_unix_ms - self.issued_at_unix_ms > MAX_ASSERTION_LIFETIME_MS
        { return Err(refused("the assertion version, run scope or lifetime is invalid")); }
        Ok(())
    }

    /// Resolve the bearer credential only locally under the journal barrier. The signed payload
    /// carries the admitted identity, never the secret. `key` is the selected host's signing key.
    pub fn sign_local(
        connection: &Connection, secret: &str, request: RequestIdentity,
        key: &MemberKey, now_ms: u64, lifetime_ms: u64,
    ) -> Result<Self, FractalError> {
        let binding = local::local_binding(connection, secret, now_ms)?;
        let expires_at_unix_ms = now_ms.checked_add(lifetime_ms)
            .ok_or_else(|| refused("the assertion expiry overflows"))?;
        let mut assertion = Self {
            version: 1, binding, request, issued_at_unix_ms: now_ms,
            expires_at_unix_ms, signature: String::new(),
        };
        assertion.validate()?;
        assertion.signature = key.sign(&assertion.message()?);
        Ok(assertion)
    }

    pub fn identity_hash(&self) -> Result<String, FractalError> {
        let bytes = serde_json::to_vec(&(&self.binding, &self.request)).map_err(internal)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}
