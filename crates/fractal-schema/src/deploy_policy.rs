//! Build-bound deployment authorization, applied only after the existing principal judge.
//!
//! Packages opt in with `st3.override { deployPolicyFile = policyFile; }`. Neither
//! runtime environment variables nor command-line arguments select the policy.
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::{ValidationError, error, registry};

/// Required allowlists are exact signer subjects, not keys or a second trust root.
/// Empty signer lists authorize nobody. Missing policy refuses verification entirely.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub version: u64,
    pub refused_verifier_hosts: Vec<String>,
    pub consumption_signers: Vec<String>,
    pub go_signers: Vec<String>,
    pub stop_signers: Vec<String>,
    pub compat_signoff_signers: Vec<String>,
    #[serde(skip)]
    content_hash: String,
}

impl Policy {
    /// The immutable package input captured during compilation; never a runtime lookup.
    pub fn packaged_path() -> Option<&'static str> {
        option_env!("ST3_DEPLOY_POLICY_FILE").filter(|path| !path.is_empty())
    }

    pub fn load_packaged() -> Result<Self, ValidationError> {
        let path = Self::packaged_path().ok_or_else(|| {
            error("missing-deploy-policy", "deployment verification requires a packaged policy")
        })?;
        let bytes = std::fs::read(path).map_err(|cause| {
            error("unreadable-deploy-policy", format!("cannot read packaged deployment policy `{path}`: {cause}"))
        })?;
        Self::from_json(&bytes)
    }

    /// Parse fixtures and policy bytes without selecting any runtime policy path.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ValidationError> {
        let mut policy: Self = serde_json::from_slice(bytes)
            .map_err(|cause| error("invalid-deploy-policy", format!("invalid deployment policy: {cause}")))?;
        if policy.version != 1 {
            return Err(error("unsupported-deploy-policy-version", "deployment policy requires version 1"));
        }
        for (name, signers) in [
            ("consumptionSigners", &policy.consumption_signers),
            ("goSigners", &policy.go_signers),
            ("stopSigners", &policy.stop_signers),
            ("compatSignoffSigners", &policy.compat_signoff_signers),
        ] {
            for signer in signers {
                if !signer.starts_with("agent/") || registry().validate_subject(signer).is_err() {
                    return Err(error("invalid-deploy-policy-signer", format!("`{name}` requires full agent identities: `{signer}`")));
                }
            }
        }
        policy.content_hash = hex::encode(Sha256::digest(bytes));
        Ok(policy)
    }

    /// Lowercase SHA-256 of the exact parsed bytes, including JSON whitespace.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        json!({
            "version": 1,
            "refusedVerifierHosts": ["host-b"],
            "consumptionSigners": ["agent/example-executor"],
            "goSigners": ["agent/example-manager"],
            "stopSigners": ["agent/example-manager"],
            "compatSignoffSigners": ["agent/example-reviewer"]
        })
    }

    fn parse(value: &Value) -> Result<Policy, ValidationError> {
        Policy::from_json(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn strict_version_fields_and_required_arrays() {
        parse(&fixture()).unwrap();
        for version in [json!(0), json!(2), json!("1"), json!(null)] {
            let mut value = fixture();
            value["version"] = version;
            assert!(parse(&value).is_err());
        }
        for key in fixture().as_object().unwrap().keys() {
            let mut value = fixture();
            value.as_object_mut().unwrap().remove(key);
            assert!(parse(&value).is_err(), "missing {key}");
            let mut value = fixture();
            value[key] = Value::Null;
            assert!(parse(&value).is_err(), "null {key}");
        }
        for key in ["unknown", "contentHash", "content_hash", "policyPath"] {
            let mut value = fixture();
            value[key] = json!("not permitted");
            assert!(parse(&value).is_err(), "unknown {key}");
        }
        assert!(Policy::from_json(b"{\"version\":1,\"version\":1}").is_err());
    }

    #[test]
    fn signer_subjects_are_exact_agent_identities() {
        for key in ["consumptionSigners", "goSigners", "stopSigners", "compatSignoffSigners"] {
            for signer in ["", "agent/", "person/example", "example", "agent//example", "agent/example/", "agent/example worker", "agent/example\n"] {
                let mut value = fixture();
                value[key] = json!([signer]);
                assert_eq!(parse(&value).unwrap_err().code, "invalid-deploy-policy-signer");
            }
        }
    }

    #[test]
    fn empty_authorization_lists_remain_empty() {
        let mut value = fixture();
        for key in ["consumptionSigners", "goSigners", "stopSigners", "compatSignoffSigners"] {
            value[key] = json!([]);
        }
        let policy = parse(&value).unwrap();
        assert!(policy.consumption_signers.is_empty());
        assert!(policy.go_signers.is_empty());
        assert!(policy.stop_signers.is_empty());
        assert!(policy.compat_signoff_signers.is_empty());
    }

    #[test]
    fn metadata_hashes_exact_content() {
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        let policy = Policy::from_json(&bytes).unwrap();
        assert_eq!(policy.content_hash(), hex::encode(Sha256::digest(&bytes)));
        let mut padded = bytes;
        padded.push(b'\n');
        assert_ne!(policy.content_hash(), Policy::from_json(&padded).unwrap().content_hash());
    }

    // Run this same assertion in a child with hostile runtime environment inputs.
    // No process-global mutation or unsafe set_var is needed.
    #[test]
    fn packaged_policy_ignores_runtime_environment_child() {
        assert_eq!(Policy::packaged_path(), option_env!("ST3_DEPLOY_POLICY_FILE").filter(|path| !path.is_empty()));
        match option_env!("ST3_DEPLOY_POLICY_TEST_MODE") {
            Some("absent") => assert!(Policy::packaged_path().is_none()),
            Some("packaged") => {
                assert!(Policy::packaged_path().is_some());
                Policy::load_packaged().unwrap();
            }
            Some("unreadable") => {
                assert!(Policy::packaged_path().is_some());
                assert_eq!(Policy::load_packaged().unwrap_err().code, "unreadable-deploy-policy");
            }
            None => {}
            Some(mode) => panic!("unknown packaged policy test mode: {mode}"),
        }
        match Policy::packaged_path() {
            None => assert_eq!(Policy::load_packaged().unwrap_err().code, "missing-deploy-policy"),
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => {
                    let expected = Policy::from_json(&bytes).unwrap();
                    assert_eq!(Policy::load_packaged().unwrap().content_hash(), expected.content_hash());
                }
                Err(_) => assert_eq!(Policy::load_packaged().unwrap_err().code, "unreadable-deploy-policy"),
            },
        }
    }

    #[test]
    fn runtime_environment_cannot_override_compiled_path() {
        let override_path = std::env::temp_dir()
            .join(format!("example-deploy-policy-override-{}.json", std::process::id()));
        std::fs::write(&override_path, serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "deploy_policy::tests::packaged_policy_ignores_runtime_environment_child"])
            .env("ST3_DEPLOY_POLICY_FILE", &override_path)
            .env("DEPLOY_POLICY_FILE", &override_path)
            .status();
        std::fs::remove_file(override_path).unwrap();
        assert!(status.unwrap().success());
    }
}
