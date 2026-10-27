#![allow(unused_imports)]
use crate::store::person_work::*;

pub fn mission(claim: &ClaimRecord) -> MissionSpec {
    serde_json::from_value(claim.body["fields"]["mission_spec"].clone()).unwrap()
}

// The local creation index is intentionally included in this history-preservation oracle.
pub fn revision_row(store: &Store, mission: &MissionSpec) -> (String, String, String, u64) {
    store
        .readers
        .get()
        .query_row(
            "SELECT state,body,claim_id,created_index FROM mission_revisions
             WHERE mission_id=?1 AND revision=?2",
            params![mission.id, mission.revision],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

pub fn ordered_claims(store: &Store, a: &ClaimRecord, b: &ClaimRecord) -> (ClaimRecord, ClaimRecord) {
    let reader = store.readers.get();
    if canonical::claim_key(&reader, &a.id).unwrap() < canonical::claim_key(&reader, &b.id).unwrap()
    {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

pub fn definition_row(store: &Store, spec: &MissionSpec) -> (String, String, String) {
    store
        .readers
        .get()
        .query_row(
            "SELECT revision,state,claim_id FROM mission_definitions WHERE mission_id=?1",
            [&spec.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

pub fn receive_both(first: &Store, second: &Store) -> Store {
    let target = Store::open_memory("cedar").unwrap();
    target.project_replication_backlog().unwrap();
    super::tests::receive(first, &target);
    super::tests::receive(second, &target);
    target
}

pub fn retain_noncanonical_source(store: &Store, spec: &MissionSpec, claim: &ClaimRecord) {
    // Model the existing first-arrival projection; the two actual source claims stay intact.
    store
        .connection
        .batched(|tx| -> Result<()> {
            assert_eq!(
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hfc9942c76914 owner=fractal_store::store::person_work::revision_provenance_tests_shared_missions TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_revisions SET claim_id=?3 WHERE mission_id=?1 AND revision=?2",
                    params![spec.id, spec.revision, claim.id],
                )?,
                1
            );
            Ok(())
        })
        .unwrap()
        .unwrap();
}

/// Publish `mission_id` on `store` with a revision below `below`: the order in which comparing
/// (revision, claim id) alone would keep a runtime definition already projected.
pub fn publish_below(store: &Store, mission_id: &str, below: &str) -> MissionSpec {
    let (source, intent) = (0..)
        .map(|attempt| {
            let source = format!(
                "version 2\nmission {mission_id:?} state=\"ready\" {{\n  goal \"Published under a \
                 runtime update's id ({attempt}).\";\n  step \"update\" {{ agentless }}\n}}\n"
            );
            let intent = crate::graph::parse_intent(&source, "birch").unwrap();
            (source, intent)
        })
        .find(|(_, intent)| intent.missions[mission_id].revision.as_str() < below)
        .unwrap();
    let planned = store
        .mission(
            &intent,
            crate::model::IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(&intent, &planned.subject_tokens, "same-id-publication")
        .unwrap();
    intent.missions[mission_id].clone()
}
