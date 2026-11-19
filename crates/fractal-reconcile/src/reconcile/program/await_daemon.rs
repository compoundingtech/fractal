//! A capability barrier on the newest owner-origin replicated daemon startup fact.
//!
//! `daemon.started` advertises a build and feature versions. It is not a health probe or proof
//! that the process remains alive: repair/readiness and service completion need their own facts.
use super::{ProgramEngine, Request};
use crate::model::ClaimRecord;
use crate::store::Store;
use anyhow::Result;
use serde_json::{Value, json};

pub(super) fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    (1..=128).contains(&value.len())
        && bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn settle(
    store: &Store, engine: &ProgramEngine, request: &Request,
    host: &str, feature: &str, min_version: u64, version: Option<&str>,
) -> Result<bool> {
    let subject = format!("program-await-daemon/{}", super::sha256(request.idempotency_key().as_bytes()));
    let binding = json!({"host": host, "feature": feature, "minVersion": min_version, "version": version});
    let registration = engine.register_wait(request, &subject, "await-daemon", binding.clone())?;
    if ["host", "feature", "minVersion", "version"].iter().any(|key| registration[*key] != binding[*key]) {
        engine.fail(&request.step, request.attempt, "daemon wait key reused with a different request")?;
        return Ok(true);
    }
    let Some(value) = observed(store, host, feature, min_version, version)? else { return Ok(false) };
    engine.resolve(request, &subject, json!({"_tag": "Success", "value": value}))?;
    Ok(true)
}

fn observed(store: &Store, host: &str, feature: &str, min_version: u64, version: Option<&str>) -> Result<Option<Value>> {
    let subject = format!("daemon/{host}");
    let mut newest = None;
    // The noted subject read also tracks the absent case, so replication wakes the waiting run.
    // Stop on the newest *owner* claim, even if it lacks this feature. Never revive an older
    // capable build after the owner advertises an incompatible restart, and never let a foreign
    // origin advertise or suppress the owner's capabilities.
    store.claims_in_order(&subject, "daemon.started", true, |claim| {
        if claim.origin != host { return true }
        newest = Some(claim.clone());
        false
    })?;
    Ok(newest.as_ref().and_then(|claim| outcome(claim, host, feature, min_version, version)))
}

fn outcome(claim: &ClaimRecord, host: &str, feature: &str, min_version: u64, version: Option<&str>) -> Option<Value> {
    let fields = &claim.body["fields"];
    if claim.origin != host || fields["status"] != "running" { return None }
    let actual_version = fields["version"].as_str().filter(|version| !version.trim().is_empty())?;
    if version.is_some_and(|expected| expected != actual_version) { return None }
    let feature_version = fields.get("features")?.get(feature)?.as_u64()
        .filter(|version| *version >= min_version && *version <= 9_007_199_254_740_991)?;
    Some(json!({
        "host": host, "claim": claim.id, "version": actual_version,
        "feature": feature, "featureVersion": feature_version,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClaimInput;

    fn start(store: &Store, host: &str, version: &str, features: Value) -> ClaimRecord {
        store.append_claim(&ClaimInput {
            subject: format!("daemon/{host}"), kind: "daemon.started".into(), actor: None,
            fields: serde_json::from_value(json!({"status": "running", "version": version, "features": features})).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap()
    }

    #[test]
    fn absent_claim_and_missing_feature_remain_pending() {
        let store = Store::open_memory("amber").unwrap();
        assert_eq!(observed(&store, "amber", "capability", 1, None).unwrap(), None);
        start(&store, "amber", "build-one", json!({"different": 1}));
        assert_eq!(observed(&store, "amber", "capability", 1, None).unwrap(), None);
    }

    #[test]
    fn minimum_is_a_feature_version_not_a_lexical_build_comparison() {
        let store = Store::open_memory("amber").unwrap();
        let claim = start(&store, "amber", "build-one", json!({"capability": 2}));
        assert_eq!(observed(&store, "amber", "capability", 1, Some("build-one")).unwrap(), Some(json!({
            "host": "amber", "claim": claim.id, "version": "build-one", "feature": "capability", "featureVersion": 2,
        })));
        assert_eq!(observed(&store, "amber", "capability", 3, None).unwrap(), None);
        assert_eq!(observed(&store, "amber", "capability", 1, Some("build-two")).unwrap(), None);
    }

    #[test]
    fn newer_owner_restart_does_not_reuse_old_capabilities() {
        let store = Store::open_memory("amber").unwrap();
        start(&store, "amber", "capable", json!({"capability": 2}));
        start(&store, "amber", "older-build", json!({"capability": 0}));
        assert_eq!(observed(&store, "amber", "capability", 1, None).unwrap(), None);
    }

    #[test]
    fn foreign_origin_cannot_advertise_or_mask_owner_fact() {
        let store = Store::open_memory("amber").unwrap();
        start(&store, "birch", "forged", json!({"capability": 99}));
        assert_eq!(observed(&store, "birch", "capability", 1, None).unwrap(), None);
        let owner = Store::open_memory("birch").unwrap();
        let claim = start(&owner, "birch", "owner-build", json!({"capability": 2}));
        store.import_replication("birch", &owner.export_replication(0).unwrap()).unwrap();
        start(&store, "birch", "foreign-newer", json!({}));
        assert_eq!(observed(&store, "birch", "capability", 1, None).unwrap().unwrap()["claim"], claim.id);
    }

    #[test]
    fn malformed_feature_values_do_not_pass() {
        let store = Store::open_memory("amber").unwrap();
        for value in [json!("1"), json!(-1), json!(1.5), json!(true), json!(9_007_199_254_740_992_u64)] {
            let claim = start(&store, "amber", "build", json!({"capability": value}));
            assert_eq!(outcome(&claim, "amber", "capability", 1, None), None);
        }
    }

    #[test]
    fn identifiers_cannot_address_another_subject_or_json_path() {
        for value in ["", "../amber", "amber/other", "feature/key", " feature", "-feature", "host\n"] {
            assert!(!identifier(value), "accepted {value:?}");
        }
        assert!(identifier("amber-2"));
        assert!(identifier("capability_v2.read"));
    }
}
