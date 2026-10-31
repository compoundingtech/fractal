//! Portable, offline rename proofs. Only the caller's fleet anchor is trusted.
//!
//! Envelopes retain their exact sealed CBOR payloads. Writer chains prove a historical
//! cut, not that no later envelope exists; callers pin progress with `after_revision`.
//! Bundles disclose full retained authenticated prefixes, not just rename claims, and
//! are marked `disclosure: "fleet-internal"`; sharing requires the fleet's consent.
//! Membership's authenticated incarnation floor can start a fresh chain above one
//! without a predecessor. Missing interior sequences never establish a new chain.

use std::fmt;
use serde::{Deserialize, Serialize};
use crate::claim::ReplicaEnvelope;

pub mod history;
mod authentication;
mod rename;
pub use rename::verify_rename_proof;
pub use history::{VerifiedHistory, verify_sealed_history};

pub const BUNDLE_SCHEMA: &str = "fractal.rename-proof.v1";
pub const VERDICT_SCHEMA: &str = "fractal.rename-proof-verdict.v1";

/// Redundant descriptive fields are checked against the authenticated claims, never trusted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofBundle {
    pub schema: String,
    /// Full retained envelopes are for consented fleet-internal disclosure only.
    pub disclosure: String,
    pub fleet_id: String,
    /// Informational only; the verifier compares it to, but never trusts it over, its caller.
    pub fleet_anchor: String,
    pub identity_kind: String,
    pub storage_id: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub declaration: String,
    pub chain: Vec<String>,
    pub envelopes: Vec<ReplicaEnvelope>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofStep {
    pub revision: String,
    pub from: String,
    pub to: String,
    pub claim_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofVerdict {
    pub schema: String,
    pub disclosure: String,
    pub ok: bool,
    pub identity_kind: String,
    pub storage_id: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub declaration: String,
    pub request_actor: String,
    pub kind: String,
    pub storage_identity: String,
    pub from: String,
    pub to: String,
    pub revision: String,
    pub actor: String,
    pub reason: String,
    pub owner_host: String,
    pub signer_member: String,
    pub signer_key: String,
    pub fleet_id: String,
    /// Signed revision tokens and their edges, oldest first.
    pub chain: Vec<RenameProofStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProofError {
    pub code: String,
    pub message: String,
}

impl ProofError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    /// Stable process status shared by offline CLI consumers.
    pub fn exit_code(&self) -> i32 {
        match self.code.as_str() {
            "invalid-signature" => 2,
            "broken-chain" => 3,
            "untrusted-membership" => 4,
            "stale-revision" => 5,
            _ => 6,
        }
    }
}

impl fmt::Display for ProofError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ProofError {}

fn malformed(message: impl Into<String>) -> ProofError {
    ProofError::new("malformed-bundle", message)
}
fn chain_error(message: impl Into<String>) -> ProofError {
    ProofError::new("broken-chain", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_exit_codes_are_stable() {
        for (code, expected) in [
            ("invalid-signature", 2), ("broken-chain", 3), ("untrusted-membership", 4),
            ("stale-revision", 5), ("malformed-bundle", 6),
        ] {
            let error = ProofError::new(code, "example");
            assert_eq!(error.exit_code(), expected);
            let encoded = serde_json::to_value(&error).expect("error encodes");
            assert_eq!(encoded["code"], code);
        }
    }
}
