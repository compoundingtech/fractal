//! Provenance tests that publish through the store; they live here because the provenance module sits
//! below the store.

#![allow(unused_imports)]

#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::provenance_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::provenance_tests_shared_missions::{publish};
use crate::model::{IntentInput, St3Error};
use std::collections::BTreeMap;
use crate::provenance::tests::*;
use crate::provenance::*;
use crate::{parse_intent, store::Store};

#[test]
fn provenance_never_changes_the_mission_or_step_hash() {
    let plain = parse_intent(&source("Build.", None), "orchard").unwrap();
    let with = parse_intent(&source("Build.", Some("Approved source.")), "orchard").unwrap();
    assert_eq!(plain.missions, with.missions);
    assert_eq!(
        serde_json::to_vec(&plain.missions).unwrap(),
        serde_json::to_vec(&with.missions).unwrap()
    );
    assert_ne!(plain.source_hash, with.source_hash);
    assert_eq!(with.mission_provenance["orchard"].decision.len(), 2);
}

#[test]
fn provenance_rejects_malformed_and_oversized_blocks() {
    let valid = source("Build.", Some("Source."));
    for invalid in [
        valid.replace("reason \"Source.\"", "reason \"\""),
        valid.replace(
            "reason \"Source.\"",
            "reason \"Source.\"; reason \"Duplicate.\"",
        ),
        valid.replace(
            "renderer \"orchard-renderer-v1\"",
            "command \"do-something\"",
        ),
        valid.replace("missions/orchard.kdl", "../orchard.kdl"),
        valid.replace(&"a".repeat(40), "deadbeef"),
        valid.replace("reason \"Source.\"", "reason \"Source.\" \"extra\""),
        valid.replace("source {", "source unexpected=\"value\" {"),
        source("Build.", Some(&"x".repeat(2049))),
        valid.replace(
            "decision \"decision/opaque-approved-scope\"",
            &"decision \"opaque\";".repeat(33),
        ),
        valid.replace(
            "decision \"decision/opaque-approved-scope\"",
            &format!("decision {:?};", "x".repeat(1025)),
        ),
    ] {
        assert!(parse_intent(&invalid, "orchard").is_err(), "{invalid}");
    }
    let full = valid.replace(
        "decision \"decision/opaque-approved-scope\"",
        &format!("decision {:?};", "x".repeat(1024)).repeat(16),
    );
    assert!(
        parse_intent(&full, "orchard")
            .unwrap_err()
            .message
            .contains("16 KiB")
    );
}

