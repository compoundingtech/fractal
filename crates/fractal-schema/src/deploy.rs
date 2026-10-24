//! Additive deployment audit records, not deployment authorization.
//!
//! Signatures use the existing smallclaims principal chain. The verifier must additionally
//! resolve typed claim references, authorize signers using its build-time policy, and bind
//! consumption to the GO actor and host. A schema-valid record alone never grants a GO.
use std::collections::BTreeMap;

use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::{ValidationError, error};

pub const KINDS: [&str; 6] = [
    "deploy.go", "deploy.go-revoked", "deploy.go-consumed", "deploy.stop",
    "deploy.stop-lifted", "deploy.compat-signoff",
];
pub const VERSION: u64 = 1;
pub const MAX_GO_LIFETIME_SECONDS: i64 = 4 * 60 * 60;

/// Package identity is an exact tuple; package paths and NAR hashes are not claim IDs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package<'a> {
    pub package_path: &'a str,
    pub nar_hash: &'a str,
    pub commit: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Combined<'a> {
    pub stop_id: &'a str,
    pub plan_id: &'a str,
}

fn invalid(message: impl Into<String>) -> ValidationError {
    error("invalid-deploy-claim", message)
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn commit(value: &str) -> Result<(), ValidationError> {
    if ![40, 64].contains(&value.len())
        || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("deployment commits require a full lowercase Git object ID"));
    }
    Ok(())
}

pub(crate) fn validate_claim(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    if !KINDS.contains(&kind) {
        return Ok(());
    }
    if fields.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(error("unsupported-deploy-version", "deployment readers require version 1"));
    }
    for (name, value) in fields {
        if value.as_str().is_some_and(|text| !nonempty(text)) {
            return Err(invalid(format!("deployment field `{name}` must not be empty")));
        }
    }
    match kind {
        "deploy.go" => {
            for name in ["old", "new"] {
                let package = Package::deserialize(&fields[name])
                    .map_err(|e| invalid(format!("invalid {name} package: {e}")))?;
                if !nonempty(package.package_path) || !nonempty(package.nar_hash) {
                    return Err(invalid("deployment package path and NAR hash must not be empty"));
                }
                commit(package.commit)?;
            }
            let issued = DateTime::parse_from_rfc3339(fields["issuedAt"].as_str().unwrap())
                .map_err(|e| invalid(format!("invalid issuedAt: {e}")))?;
            let expires = DateTime::parse_from_rfc3339(fields["expiresAt"].as_str().unwrap())
                .map_err(|e| invalid(format!("invalid expiresAt: {e}")))?;
            let lifetime = expires.signed_duration_since(issued);
            if lifetime <= chrono::Duration::zero()
                || lifetime > chrono::Duration::seconds(MAX_GO_LIFETIME_SECONDS)
            {
                return Err(invalid("GO expiry must be after issue and at most four hours later"));
            }
            if let Some(value) = fields.get("combined") {
                let combined = Combined::deserialize(value)
                    .map_err(|e| invalid(format!("invalid combined binding: {e}")))?;
                if !nonempty(combined.stop_id) || !nonempty(combined.plan_id) {
                    return Err(invalid("combined binding needs nonempty stopId and planId"));
                }
            }
        }
        "deploy.stop" => {
            let hosts = fields["hosts"].as_array().unwrap();
            if hosts.is_empty() || hosts.iter().any(|host| host.as_str().is_none_or(|s| !nonempty(s))) {
                return Err(invalid("STOP hosts must be a nonempty array of nonempty host names"));
            }
        }
        "deploy.compat-signoff" => {
            commit(fields["oldCommit"].as_str().unwrap())?;
            commit(fields["newCommit"].as_str().unwrap())?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cardinality, Retention, WritePolicy, registry};
    use serde_json::json;

    fn go() -> BTreeMap<String, Value> {
        serde_json::from_value(json!({
            "version": 1, "host": "host-a", "actor": "agent/example-executor",
            "operation": "install", "trainId": "train/example",
            "old": {"packagePath": "/nix/store/old-example", "narHash": "sha256-example-old", "commit": "a".repeat(40)},
            "new": {"packagePath": "/nix/store/new-example", "narHash": "sha256-example-new", "commit": "b".repeat(40)},
            "issuedAt": "2026-10-24T10:00:00Z", "expiresAt": "2026-10-24T14:00:00Z",
            "failureRestore": false, "previewDigest": "example-preview-digest",
            "compatSignoffId": "example-signoff-claim"
        })).unwrap()
    }

    fn validate(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
        registry().validate_claim("resource/deploy/example", kind, fields).map(|_| ())
    }

    #[test]
    fn deploy_registry_is_additive_durable_and_not_publicly_writable() {
        for kind in KINDS {
            let spec = registry().claim(kind).unwrap();
            assert_eq!(spec.retention, Retention::Durable);
            assert_eq!(spec.cardinality, Cardinality::Append);
            assert_eq!(spec.write_policy, WritePolicy::SystemOnly);
            assert!(spec.projection.is_none());
            assert!(!spec.wakes_reconciler);
        }
    }

    #[test]
    fn deploy_all_kinds_require_version_one_and_exact_fields() {
        let examples = [
            ("deploy.go", go()),
            ("deploy.go-revoked", serde_json::from_value(json!({"version":1,"goId":"example-go"})).unwrap()),
            ("deploy.go-consumed", serde_json::from_value(json!({"version":1,"goId":"example-go","operation":"rollback","result":"failed"})).unwrap()),
            ("deploy.stop", serde_json::from_value(json!({"version":1,"stopId":"example-stop","scope":"installer-entry","hosts":["host-a","host-b"],"liftCondition":"example release"})).unwrap()),
            ("deploy.stop-lifted", serde_json::from_value(json!({"version":1,"stopId":"example-stop","releaseGoId":"example-release"})).unwrap()),
            ("deploy.compat-signoff", serde_json::from_value(json!({"version":1,"trainId":"train/example","oldCommit":"a".repeat(40),"newCommit":"b".repeat(40)})).unwrap()),
        ];
        for (kind, fields) in examples {
            validate(kind, &fields).unwrap();
            for name in fields.keys() {
                let mut missing = fields.clone();
                missing.remove(name);
                assert!(validate(kind, &missing).is_err(), "{kind}: {name}");
            }
            for version in [json!(0), json!(2), json!("1"), json!(null)] {
                let mut invalid = fields.clone();
                invalid.insert("version".into(), version);
                assert!(validate(kind, &invalid).is_err(), "{kind}");
            }
            let mut extra = fields;
            extra.insert("unknown".into(), json!(true));
            assert!(validate(kind, &extra).is_err());
        }
    }

    #[test]
    fn deploy_go_binds_preview_packages_and_bounded_issue_interval() {
        let mut fields = go();
        validate("deploy.go", &fields).unwrap();
        fields.insert("combined".into(), json!({"stopId":"example-stop","planId":"example-plan"}));
        validate("deploy.go", &fields).unwrap();
        for (name, value) in [
            ("expiresAt", json!("2026-10-24T14:00:00.001Z")),
            ("expiresAt", json!("2026-10-24T10:00:00Z")),
            ("expiresAt", json!("2026-10-24T09:00:00Z")),
            ("issuedAt", json!("not-a-time")),
            ("previewDigest", json!("")),
            ("compatSignoffId", json!(" ")),
            ("operation", json!("switch")),
            ("actor", json!("person/example")),
            ("old", json!({"packagePath":"/nix/store/example","narHash":"hash"})),
            ("new", json!({"packagePath":"/nix/store/example","narHash":"hash","commit":"short"})),
            ("combined", json!({"stopId":"example-stop"})),
            ("combined", json!({"stopId":"example-stop","planId":"example-plan","previewDigest":"extra"})),
        ] {
            let mut invalid = fields.clone();
            invalid.insert(name.into(), value);
            assert!(validate("deploy.go", &invalid).is_err(), "{name}");
        }
    }

    #[test]
    fn deploy_stop_hosts_and_compat_commits_are_checked() {
        for hosts in [json!([]), json!([""]), json!(["host-a", 1])] {
            let fields = serde_json::from_value(json!({"version":1,"stopId":"example-stop","scope":"installer-entry","hosts":hosts,"liftCondition":"example release"})).unwrap();
            assert!(validate("deploy.stop", &fields).is_err());
        }
        let fields = serde_json::from_value(json!({"version":1,"trainId":"train/example","oldCommit":"short","newCommit":"b".repeat(40)})).unwrap();
        assert!(validate("deploy.compat-signoff", &fields).is_err());
    }
}
