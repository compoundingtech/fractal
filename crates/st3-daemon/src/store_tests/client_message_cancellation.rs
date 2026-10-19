//! Client-message selector tests that need the missions and attention stores; they live in st3-daemon because
//! those stores do.

#![allow(unused_imports)]

use crate::MissionsStore;
use std::collections::BTreeMap;

use crate::model::*;
use crate::store::Store;
use crate::store::client_messages::*;
use serde_json::{Value, json};

fn commit_budget(store: &Store) -> FoldBudget {
    *store.smalltalk.client_message_commit_budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn assert_commit_budget(store: &Store) -> FoldBudget {
    let budget = commit_budget(store);
    assert!(budget.subjects <= BACKFILL_SUBJECTS, "{budget:?}");
    assert!(budget.claims <= FOLD_CLAIMS, "{budget:?}");
    assert!(budget.scratch_rows <= BACKFILL_CLEAR_ROWS, "{budget:?}");
    budget
}

fn paged_backfill_rows(store: &Store, person: Option<&str>, actor: Option<&str>, history: bool) -> Vec<Value> {
    store.read_snapshot(|through| {
        let mut after = None;
        let mut result = Vec::new();
        loop {
            let mut rows = store.client_messages_page(person, actor, history, through, after.as_ref(), 5)?;
            let more = rows.len() > 5;
            rows.truncate(5);
            for (message, metadata, current) in rows {
                after = Some((metadata["sent_at"].as_str().unwrap().parse::<u128>()?, message.subject.clone()));
                result.push(json!({"message": message, "metadata": metadata, "current": current}));
            }
            if !more { return Ok(result); }
        }
    }).unwrap()
}

#[test]
fn cancellation_apply_with_many_claimants_has_one_selector_commit_allowance() {
    let store = Store::open_memory("node").unwrap();
    let count = BACKFILL_SUBJECTS * 3 + 3;
    let mut source = String::from(
        "version 2\nagent \"worker\" { workspace \".\"; command \"true\" }\n\
         mission \"cancel-batch\" state=\"ready\" {\n goal \"Cancel all leased steps.\"\n",
    );
    for index in 0..count {
        source.push_str(&format!(
            "step \"work-{index}\" {{ assigned-to \"agent/node.worker\" }}\n",
        ));
    }
    source.push_str("}\n");
    let intent = crate::graph::parse_test_intent(&source, "node").unwrap();
    let planned = store.mission(
        &intent, IntentInput { kdl: source, source_name: None },
    ).unwrap();
    store.apply(&intent, &planned.subject_tokens, "publish-cancel-batch").unwrap();
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "cancel-batch".into(), revision: None, workspace: ".".into(),
        requester: Some("person/test".into()), mode: None, inputs: BTreeMap::new(),
        idempotency_key: "run-cancel-batch".into(),
    }).unwrap();
    // The real cancellation helper consumes the persisted claimant on every
    // open step; seed those leases together rather than involving a daemon.
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        for step in &run.steps {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h2abf5edd5f84 owner=st3_daemon::store_tests::client_message_cancellation TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET lease_owner='agent/node.worker' WHERE subject=?1",
                [&step.subject],
            ).unwrap();
        }
        transaction.commit().unwrap();
    }
    while store.maintain_client_message_selectors().unwrap() {}
    let baseline = paged_backfill_rows(&store, None, None, true);
    let cancellation = format!(
        "version 2\nmission-run {:?} {{ cancellation \"operator\" {{ reason \"cancel batch\" }} }}\n",
        run.subject,
    );
    let intent = crate::graph::parse_test_intent(&cancellation, "node").unwrap();
    let planned = store.mission(
        &intent, IntentInput { kdl: cancellation, source_name: None },
    ).unwrap();
    store.apply(&intent, &planned.subject_tokens, "cancel-batch").unwrap();
    assert_commit_budget(&store);
    assert!(store.client_message_selectors_pending().unwrap());
    assert_eq!(paged_backfill_rows(&store, None, None, true), baseline);
    let cancellation_claims: usize = store.readers.get().query_row(
        "SELECT COUNT(*) FROM claims WHERE kind='message.sent' AND subject LIKE 'message/mission-cancelled-%'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(cancellation_claims, count);
    while store.client_message_selectors_pending().unwrap() {
        store.maintain_client_message_selectors().unwrap();
        assert_commit_budget(&store);
    }
    let rows = paged_backfill_rows(&store, Some("agent/node.worker"), None, true);
    assert_eq!(rows.iter().filter(|row|
        row["message"]["title"] == "Mission work cancelled").count(), count);
}
