//! Reviewed v2 registrations for the two notice authority domains: producer sources and the
//! snapshots the notice owner's daemon writes. Person controls are native attention controls,
//! not a notice family. These manifests describe storage, not the joined NoticeSnapshot read
//! model. Storage field bounds do not replace native lineage or typed view/action validation.
use crate::{ValidationError, custom, error};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, sync::LazyLock};

pub const SOURCE_PREFIX: &str = "custom/attention/source/";
pub const SNAPSHOT_PREFIX: &str = "custom/attention/snapshot/";
/// The v1 control family's prefix stays reserved without a manifest, so generic rows written
/// there before the namespace was reserved stay hidden and nothing can register it again.
pub const RETIRED_CONTROL_PREFIX: &str = "custom/attention/control/";
pub const PREFIXES: [&str; 3] = [SOURCE_PREFIX, SNAPSHOT_PREFIX, RETIRED_CONTROL_PREFIX];
pub const VERSION: u32 = 2;
/// The actor every daemon writes snapshots as. It proves nothing alone: authority is the
/// claim's origin member checked against the fleet's notice-owner binding.
pub const RUNTIME_ACTOR: &str = "daemon/runtime";
pub const HASHES: [&str; 2] = [
    "96a618145580c78d6eedd1a9f14ef22adf04542b72cc719ac7d52e4a02cb69d4",
    "041f279b64fd2da4884869edcffde8fb88096b80cd1ba8e7d65139c3310f4cf2",
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

pub fn manifests() -> &'static [custom::Manifest; 2] {
    static MANIFESTS: LazyLock<[custom::Manifest; 2]> = LazyLock::new(|| [build("source"), build("snapshot")]);
    &MANIFESTS
}

/// A candidate identity selects a reviewed pin, never a caller-provided hash.
pub fn pinned_hash(manifest: &custom::Manifest) -> Option<&'static str> {
    ["attention.notice-source", "attention.notice-snapshot"]
        .iter().position(|kind| manifest.kind == *kind)
        .filter(|_| manifest.version == VERSION)
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
        error("reserved-notice-registration", "notice namespace requires a reviewed v2 manifest")
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
        fields.insert("policy".into(), field("object", true, &[]));
        let mut delivery = field("array", true, &[]);
        delivery.max_items = 4;
        fields.insert("delivery".into(), delivery);
        // The owner member whose daemon wrote the snapshot; a handover rebuilds snapshots.
        fields.insert("runtime".into(), field("subject-reference", true, &["host"]));
    }
    for (name, families) in [("producer", &["agent", "person"][..]), ("recipient", &["person"][..])] {
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
        kind: format!("attention.notice-{family}"), namespace: "attention".into(), version: VERSION,
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
    fn notice_v2_drops_the_control_family_and_binds_snapshots_to_a_host() {
        let [source, snapshot] = manifests();
        assert!(!source.claims[&source.creation_kind].fields.contains_key("runtime"));
        for kind in [&snapshot.creation_kind, "custom.attention.snapshot-updated"] {
            assert_eq!(snapshot.claims[kind].fields["runtime"].reference_families, ["host"]);
        }
        for v1 in [source, snapshot] {
            let mut v1 = v1.clone();
            v1.version = 1;
            let hash = hex::encode(Sha256::digest(serde_json::to_vec(&v1).unwrap()));
            assert!(validate_registration(&v1, &hash).is_err(), "{}", v1.kind);
        }
        let mut control = snapshot.clone();
        control.kind = "attention.notice-control".into();
        control.subject_prefix = RETIRED_CONTROL_PREFIX.into();
        let hash = hex::encode(Sha256::digest(serde_json::to_vec(&control).unwrap()));
        assert!(validate_registration(&control, &hash).is_err());
        assert!(is_reserved_subject("custom/attention/control/legacy"));
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
