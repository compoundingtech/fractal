//! Reviewed v1 registrations for the four notice authority domains.
//! These manifests describe storage, not the joined NoticeSnapshot read model.
//! Storage field bounds do not replace native lineage or typed view/action validation.
use crate::{ValidationError, custom, error};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, sync::LazyLock};

pub const SOURCE_PREFIX: &str = "custom/attention/source/";
pub const SNAPSHOT_PREFIX: &str = "custom/attention/snapshot/";
pub const CONTROL_PREFIX: &str = "custom/attention/control/";
pub const RECEIPT_PREFIX: &str = "custom/attention/receipt/";
pub const PREFIXES: [&str; 4] = [SOURCE_PREFIX, SNAPSHOT_PREFIX, CONTROL_PREFIX, RECEIPT_PREFIX];
pub const HASHES: [&str; 4] = [
    "1282a67945367e1557ab423f0ad6e4547c48629d9cbc0479ad70b15ef7c6a530",
    "975f19c2cd036faa8ba19c663a5557e2cad01ba3eb4f2c274773f5a2685aec10",
    "ea890a46d937566b480ea782047f3e6d923810414e5b0e52e0c8a883561efbb0",
    "0d9b72fca0f63e4bbcc2509d0fe6cfc3b11b81d16255fc02c5b00c89a93084eb",
];

/// Stable domain identity: the immutable source UUID, never a runtime snapshot subject.
pub fn id_for_source(source: &str) -> Result<String, ValidationError> {
    static UUID: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(
        "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
    ).expect("compiled notice source UUID"));
    let suffix = source.strip_prefix(SOURCE_PREFIX).filter(|suffix| UUID.is_match(suffix))
        .ok_or_else(|| error("invalid-notice-source", "notice identity requires a canonical source UUID"))?;
    Ok(format!("notice/{suffix}"))
}

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

pub fn manifests() -> &'static [custom::Manifest; 4] {
    static MANIFESTS: LazyLock<[custom::Manifest; 4]> = LazyLock::new(|| {
        [build("source"), build("snapshot"), build("control"), build("receipt")]
    });
    &MANIFESTS
}

/// A candidate identity selects a reviewed pin, never a caller-provided hash.
pub fn pinned_hash(manifest: &custom::Manifest) -> Option<&'static str> {
    ["attention.notice-source", "attention.notice-snapshot", "attention.notice-control", "attention.notice-receipt"]
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
            let mut binding = field("string", true, &[]);
            binding.pattern = Some("[0-9a-f]{64}".into());
            fields.insert("runtime_binding".into(), binding);
            fields.insert("policy".into(), field("object", true, &[]));
            let mut delivery = field("array", true, &[]);
            delivery.max_items = 4;
            fields.insert("delivery".into(), delivery);
        } else if family == "receipt" {
            let mut binding = field("string", true, &[]);
            binding.pattern = Some("[0-9a-f]{64}".into());
            fields.insert("owner_binding".into(), binding);
            let mut surface = field("string", true, &[]);
            surface.values = ["tray", "speech", "push", "log"].map(String::from).into();
            fields.insert("surface".into(), surface);
            fields.insert("outcome".into(), field("object", true, &[]));
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
        if name == "runtime" && family != "snapshot" { continue; }
        fields.insert(name.into(), field("subject-reference", family != "source" || name == "recipient", families));
    }
    let creation_kind = format!("custom.attention.{}-created", if family == "source" { "notice" } else { family });
    let mut claims = BTreeMap::new();
    if family != "source" && family != "receipt" {
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
    fn notice_identity_is_derived_from_source_not_runtime() {
        let source = "custom/attention/source/01900000-0000-7000-8000-000000000002";
        assert_eq!(id_for_source(source).unwrap(), "notice/01900000-0000-7000-8000-000000000002");
        for invalid in [SNAPSHOT_PREFIX, "custom/attention/source/not-a-uuid", "custom/attention/source/../source"] {
            assert!(id_for_source(invalid).is_err());
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

    #[test]
    fn notice_receipt_registration_is_pinned_and_creation_only() {
        let manifest = &manifests()[3];
        assert_eq!(manifest.creation_kind, crate::notice_receipt::KIND);
        assert_eq!(manifest.claims.len(), 1);
        assert_eq!(pinned_hash(manifest), Some(HASHES[3]));
        validate_registration(manifest, HASHES[3]).unwrap();
        let fields = &manifest.claims[&manifest.creation_kind].fields;
        assert_eq!(fields.len(), 7);
        assert!(!fields.contains_key("runtime"));
        let mut changed = manifest.clone();
        changed.claims.get_mut(&manifest.creation_kind).unwrap().additional_fields = true;
        let hash = hex::encode(Sha256::digest(serde_json::to_vec(&changed).unwrap()));
        assert!(validate_registration(&changed, &hash).is_err());
    }

    #[test]
    fn notice_receipt_namespace_rejects_root_ancestor_descendant_and_kind_squats() {
        for prefix in ["custom/attention/receipt", "custom/attention/", RECEIPT_PREFIX,
            "custom/attention/receipt/nested/"] {
            let mut manifest = manifests()[3].clone();
            manifest.kind = "attention.other".into();
            manifest.subject_prefix = prefix.into();
            assert!(is_reserved_subject(prefix));
            assert!(validate_registration(&manifest, HASHES[3]).is_err());
        }
        let mut manifest = manifests()[3].clone();
        manifest.subject_prefix = "custom/unrelated/receipt/".into();
        assert!(is_reserved_kind(&manifest.kind));
        assert!(is_reserved_kind(&manifest.creation_kind));
        assert!(validate_registration(&manifest, HASHES[3]).is_err());
    }
}
