//! Reviewed v1 registrations for the three notice authority domains.
//! These manifests describe storage, not the joined NoticeSnapshot read model.
//! Storage field bounds do not replace native lineage or typed view/action validation.
use crate::{ValidationError, custom, error};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, sync::LazyLock};

pub const SOURCE_PREFIX: &str = "custom/attention/source/";
pub const SNAPSHOT_PREFIX: &str = "custom/attention/snapshot/";
pub const CONTROL_PREFIX: &str = "custom/attention/control/";
pub const PREFIXES: [&str; 3] = [SOURCE_PREFIX, SNAPSHOT_PREFIX, CONTROL_PREFIX];
pub const HASHES: [&str; 3] = [
    "7a9c67075779112dd117ea13a7cc6bb9535e9501beb2e1d683d1516c62d07b24",
    "1f91f51979ce1b303c43453fa14fa8f0b2ec03fbb2304ff3b4a21ea99c30ff69",
    "01e8f20112982c35e6f316a4c750e11cfc3f84129f1a266ea1d15f568e0a648e",
];

/// Includes exact roots and ancestors, so a broader registration cannot squat.
pub fn is_reserved_subject(subject: &str) -> bool {
    let root = subject.trim_end_matches('/');
    PREFIXES.iter().any(|prefix| {
        subject.starts_with(prefix)
            || root == prefix.trim_end_matches('/')
            || prefix.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Reserve the claim namespace even when a manifest chooses another subject prefix.
pub fn is_reserved_kind(kind: &str) -> bool {
    kind == "custom.attention" || kind.starts_with("custom.attention.")
        || kind == "attention.notice" || kind.starts_with("attention.notice-")
}

pub fn manifests() -> &'static [custom::Manifest; 3] {
    static MANIFESTS: LazyLock<[custom::Manifest; 3]> = LazyLock::new(|| {
        [build("source"), build("snapshot"), build("control")]
    });
    &MANIFESTS
}

/// A candidate identity selects a reviewed pin, never a caller-provided hash.
pub fn pinned_hash(manifest: &custom::Manifest) -> Option<&'static str> {
    ["attention.notice-source", "attention.notice-snapshot", "attention.notice-control"]
        .iter().position(|kind| manifest.kind == *kind)
        .filter(|_| manifest.version == 1)
        .map(|index| HASHES[index])
}

/// Hash canonical `serde_json::to_vec(Manifest)` bytes, as registration does.
/// Checking both hashes prevents callers from pairing altered contents with a pin.
pub fn validate_registration(manifest: &custom::Manifest, hash: &str) -> Result<(), ValidationError> {
    manifest.validate()?;
    if !is_reserved_subject(&manifest.subject_prefix)
        && !is_reserved_kind(&manifest.kind)
        && !is_reserved_kind(&manifest.creation_kind)
        && !manifest.claims.keys().any(|kind| is_reserved_kind(kind))
    {
        return Ok(());
    }
    let expected = pinned_hash(manifest).ok_or_else(|| {
        error("reserved-notice-registration", "notice namespace requires a reviewed v1 manifest")
    })?;
    let bytes = serde_json::to_vec(manifest)
        .map_err(|err| error("reserved-notice-registration", err.to_string()))?;
    if hash != expected || hex::encode(Sha256::digest(bytes)) != expected {
        return Err(error("reserved-notice-registration", "notice manifest differs from its reviewed pin"));
    }
    Ok(())
}

fn field(value_type: &str, required: bool, families: &[&str]) -> custom::Field {
    serde_json::from_value(json!({
        "value_type": value_type, "required": required, "reference_families": families,
    })).expect("compiled notice field")
}

fn build(family: &str) -> custom::Manifest {
    let mut fields = BTreeMap::new();
    if family == "source" {
        for (name, value_type, required) in [
            ("schema_version", "integer", true), ("dedupe_key", "string", true),
            ("intent", "string", true), ("urgency", "string", true),
            ("requested", "boolean", true), ("summary", "string", true),
            ("speech", "string", false), ("body", "string", false),
            ("links", "array", true), ("source", "object", true),
            ("created_at", "string", true), ("expires_at", "string", false),
        ] {
            fields.insert(name.into(), field(value_type, required, &[]));
        }
        fields.get_mut("intent").expect("intent").values =
            ["done", "failed", "blocked", "fyi"].map(String::from).into();
        fields.get_mut("urgency").expect("urgency").values =
            ["info", "attention", "confirm"].map(String::from).into();
        fields.get_mut("links").expect("links").max_items = 10;
        fields.insert("resource".into(), field("subject-reference", true, &["resource"]));
    } else {
        fields.insert("source_id".into(), field("subject-reference", true, &["custom"]));
        let mut claim = field("string", true, &[]);
        claim.pattern = Some("[0-9a-f]{64}".into());
        fields.insert("creation_claim".into(), claim);
        if family == "snapshot" {
            fields.insert("policy".into(), field("object", true, &[]));
            let mut delivery = field("array", true, &[]);
            delivery.max_items = 4;
            fields.insert("delivery".into(), delivery);
        } else {
            for name in ["seen_at", "dismissed_at", "snoozed_until"] {
                fields.insert(name.into(), field("any", true, &[]));
            }
        }
    }
    for (name, families) in [
        ("producer", &["agent", "person"][..]), ("recipient", &["person"][..]),
        ("runtime", &["agent"][..]),
    ] {
        fields.insert(name.into(), field("subject-reference", family != "source" || name == "recipient", families));
    }
    let creation_kind = format!("custom.attention.{}-created", if family == "source" { "notice" } else { family });
    let mut claims = BTreeMap::new();
    if family != "source" {
        claims.insert(format!("custom.attention.{family}-updated"), custom::ClaimSchema {
            authority: custom::Authority::Owner, fields: fields.clone(), additional_fields: false,
        });
    }
    claims.insert(creation_kind.clone(), custom::ClaimSchema {
        authority: custom::Authority::Creator, fields, additional_fields: false,
    });
    custom::Manifest {
        kind: format!("attention.notice-{family}"), namespace: "attention".into(), version: 1,
        language: custom::LANGUAGE.into(), subject_prefix: format!("custom/attention/{family}/"),
        slots: BTreeMap::from([("creation".into(), custom::Slot {
            kind: creation_kind.clone(), select: custom::Select::First,
        })]),
        creation_kind, claims, fields: BTreeMap::new(), attention: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_reviewed_manifests_match_compiled_pins() {
        for (manifest, pin) in manifests().iter().zip(HASHES) {
            validate_registration(manifest, pin).unwrap();
        }
    }

    #[test]
    fn notice_reserved_prefix_squatting() {
        for prefix in ["custom/", "custom/attention/", SOURCE_PREFIX, "custom/attention/source/nested/"] {
            let mut manifest = manifests()[0].clone();
            manifest.kind = "attention.unrelated".into();
            manifest.subject_prefix = prefix.into();
            assert!(validate_registration(&manifest, HASHES[0]).is_err());
            assert!(is_reserved_subject(prefix));
        }
        assert!(!is_reserved_subject("custom/attention/source-other/"));
        assert!(!is_reserved_subject("custom/elsewhere/"));
    }

    #[test]
    fn notice_alternate_prefix_cannot_reuse_reserved_claims() {
        let mut manifest = manifests()[0].clone();
        manifest.kind = "attention.other".into();
        manifest.subject_prefix = "custom/attention/other/".into();
        assert!(validate_registration(&manifest, HASHES[0]).is_err());
        assert!(is_reserved_kind("custom.attention.future-notice-kind"));
    }

    #[test]
    fn notice_altered_manifest_rejects_both_forged_and_recomputed_hash() {
        let mut manifest = manifests()[0].clone();
        manifest.claims.get_mut(&manifest.creation_kind).unwrap().additional_fields = true;
        let recomputed = hex::encode(Sha256::digest(serde_json::to_vec(&manifest).unwrap()));
        assert!(validate_registration(&manifest, HASHES[0]).is_err());
        assert!(validate_registration(&manifest, &recomputed).is_err());
        assert!(validate_registration(&manifests()[0], &"0".repeat(64)).is_err());
    }
}
