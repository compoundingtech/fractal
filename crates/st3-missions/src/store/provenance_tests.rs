#![allow(unused_imports)]
use crate::MissionsStore;
use crate::model::{IntentInput, St3Error};
use std::collections::BTreeMap;
use crate::provenance::tests::*;
use crate::provenance::*;
use crate::parse_intent;
use crate::store::Store;
use st3_store::store::provenance_tests_shared_missions::*;
use st3_store::store::provenance_tests_shared_missions::{publish};


#[test]
fn publication_fixes_provenance_per_revision_and_keeps_history() {
    let store = Store::open_memory("orchard").unwrap();
    let first = source("Build.", Some("First source."));
    publish(&store, &first, "first").unwrap();
    let revision = store
        .mission_spec("orchard", None)
        .unwrap()
        .unwrap()
        .revision;
    let original = store
        .mission_provenance("orchard", &revision)
        .unwrap()
        .unwrap();
    publish(&store, &first, "repeat").unwrap();
    publish(&store, &source("Build.", None), "omit").unwrap();
    let mutation = source("Build.", Some("Changed source."));
    assert_eq!(
        publish(&store, &mutation, "mutate").unwrap_err().code,
        "immutable-mission-provenance"
    );
    assert_eq!(
        store.mission_provenance("orchard", &revision).unwrap(),
        Some(original.clone())
    );
    publish(
        &store,
        &source("Build again.", Some("Second source.")),
        "second",
    )
    .unwrap();
    let next = store
        .mission_spec("orchard", None)
        .unwrap()
        .unwrap()
        .revision;
    assert_ne!(revision, next);
    assert_eq!(
        store.mission_provenance("orchard", &revision).unwrap(),
        Some(original)
    );
    assert_eq!(
        store
            .mission_provenance("orchard", &next)
            .unwrap()
            .unwrap()
            .reason,
        "Second source."
    );
    publish(&store, &source("Build without metadata.", None), "plain").unwrap();
    let plain = store.mission_spec("orchard", None).unwrap().unwrap();
    assert!(
        store
            .mission_provenance("orchard", &plain.revision)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        publish(
            &store,
            &source("Build without metadata.", Some("Late source.")),
            "late"
        )
        .unwrap_err()
        .code,
        "immutable-mission-provenance"
    );
}

#[test]
fn renderer_and_references_are_inert_literals() {
    let source =
        source("Build.", Some("Source.")).replace("orchard-renderer-v1", "$(do-not-execute)");
    let intent = parse_intent(&source, "orchard").unwrap();
    assert!(intent.subjects.is_empty());
    assert!(intent.document_refs.is_empty());
    assert_eq!(
        intent.mission_provenance["orchard"].source.renderer,
        "$(do-not-execute)"
    );
    let bare = source.replace("doc/orchard/proof@opaque-reference", "doc/orchard/proof");
    let parsed = parse_intent(&bare, "orchard").unwrap();
    assert!(parsed.document_refs.is_empty());
    let resolved = crate::graph::resolve_document_references(
        &bare,
        &BTreeMap::from([("doc/orchard/proof".into(), "b".repeat(64))]),
    )
    .unwrap();
    assert_eq!(
        parse_intent(&resolved, "orchard")
            .unwrap()
            .mission_provenance,
        parsed.mission_provenance
    );
}
