#![allow(unused_imports)]
use crate::AttentionStore;
use st3_seats::SeatsStoreCore;
use st3_seats::MissionsStore;
use st3_seats::SeatsStore;
use st3_store::graph::parse_test_intent as parse_intent;
use super::*;
use super::tests::*;

#[test]
fn cleanup_preserves_legacy_run_owned_uri_resources_for_other_seats() {
    for mode in ["run", "eval"] {
        let store = Store::open_memory("node").unwrap();
        publish_mission(
            &store,
            r#"
version 2
mission "legacy-resource" state="ready" timeout="1m" {
  goal "Dispose of one run without deleting another seat's URI."
  step "work" { agentless }
}
"#,
            "publish-legacy-resource-mission",
        );
        let run = store.create_mission_run(&MissionRunRequest {
            mission: "legacy-resource".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/test".into()),
            mode: Some(mode.into()),
            inputs: BTreeMap::new(),
            idempotency_key: "legacy-resource-run".into(),
        }).unwrap();
        let reader = parse_intent(
            r#"version 2
agent "shared/reader" {
  workspace "/tmp"
  command "true"
  resource "issue" uri="https://github.com/example/project/issues/42"
}
"#,
            "node",
        ).unwrap();
        store.apply_internal(&reader, "legacy-reader").unwrap();
        let references = crate::graph::declared_resources(&reader.subjects["agent/shared/reader"].desired);
        let resource = &references[0].subject;
        let mut legacy = reader.subjects[resource].clone();
        legacy.owner_run = Some(run.subject.clone());
        legacy.owner_generation = Some(run.generation.clone());
        legacy.owner_step = Some(run.steps[0].subject.clone());
        // Seed the persisted projection produced by the old materializer. The shared
        // body's content and original claim must survive, not be replaced during cleanup.
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h9ef29d1daec1 owner=st3_attention::store::legacy_uri_resource_cleanup_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_run=?1, owner_generation=?2, owner_step=?3, revision=?4
             WHERE subject=?5",
            params![
                legacy.owner_run, legacy.owner_generation, legacy.owner_step,
                desired_revision(&legacy), resource,
            ],
        ).unwrap();
        let original = store.claims_for(resource, Some("intent.desired")).unwrap();
        assert!(!store.apply_internal(&reader, "legacy-reader-again").unwrap().changed);
        if mode == "eval" {
            assert!(store.retire_eval_owned_desired(&run.subject).unwrap().is_empty());
        } else {
            store.discard_desired_owned_by(&run.subject).unwrap();
        }
        let uris = store.declared_resource_uris(&references).unwrap();
        assert_eq!(
            uris.get(resource).map(String::as_str),
            Some("https://github.com/example/project/issues/42"),
        );
        assert_eq!(
            store.claims_for(resource, Some("intent.desired")).unwrap()
                .iter().map(|claim| &claim.id).collect::<Vec<_>>(),
            original.iter().map(|claim| &claim.id).collect::<Vec<_>>(),
        );
    }
}
