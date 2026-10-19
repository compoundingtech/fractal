#![allow(unused_imports)]
pub use st3_store::store::resource_preparation::tests_shared_seats::*;
use crate::store::resource_preparation::*;
use crate::store::resource_preparation_shared_seats::*;


pub fn fixture() -> (tempfile::TempDir, Store, String) {
    // Fast-path controls must exercise the eligible file-backed mode. Keep the directory
    // alive in each caller; separate controls below retain shared-memory behavior.
    let directory = tempfile::tempdir().unwrap();
    let (store, revision) = fixture_for(
        Store::open(&directory.path().join("claims.sqlite3"), "prepared-fixture").unwrap(),
    );
    (directory, store, revision)
}

pub fn fixture_for(store: Store) -> (Store, String) {
    let source = r#"version 2
resource "fixture" { kind "vcs.pull-request" }
observer "fixture" {
    resource "resource/fixture"
    provider "github.pull-request"
    locator "invented/fixture#1"
    field "state"
}
"#;
    let intent = crate::graph::parse_test_intent(source, "prepared-fixture").unwrap();
    let planned = store
        .mission(
            &intent,
            IntentInput {
                kdl: source.into(),
                source_name: None,
            },
        )
        .unwrap();
    assert!(planned.blockers.is_empty());
    store
        .apply(&intent, &planned.subject_tokens, "fixture")
        .unwrap();
    let revision = store
        .selected_desired_revision("observer/fixture")
        .unwrap()
        .unwrap();
    (store, revision)
}
