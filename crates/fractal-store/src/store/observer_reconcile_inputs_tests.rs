use super::*;

fn fixture() -> Store {
    let store = Store::open_memory("test").unwrap();
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        append_claim_tx(&transaction, &store.origin, "resource/source", "resource.observed",
            None, &json!({"fields":{}}), &[], None).unwrap();
        transaction.commit().unwrap();
    }
    for (subject, owner) in [("resource/first", "mission-run/first"), ("resource/second", "mission-run/second")] {
        store.connection.write().execute(
            "INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run)
             VALUES(?1,'resource','test',(SELECT id FROM claims LIMIT 1),'{}',?2)",
            params![subject, owner],
        ).unwrap();
    }
    store
}

#[test]
fn desired_inputs_rebuild_only_changed_keys_and_track_raw_removals() {
    let store = fixture();
    let mut inputs = DesiredInputs::default();
    inputs.refresh(&store).unwrap();
    let initial = store.reconcile_input_work();
    let first = inputs.desired.as_ptr();
    drop(store.connection.write());
    inputs.refresh(&store).unwrap();
    assert_eq!(store.reconcile_input_work(), initial, "no-change loans do no input work");
    assert_eq!(inputs.desired.as_ptr(), first, "unchanged roster is retained in place");
    {
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        append_claim_tx(&transaction, &store.origin, "resource/unrelated", "resource.observed",
            None, &json!({"fields":{}}), &[], None).unwrap();
        transaction.commit().unwrap();
    }
    inputs.refresh(&store).unwrap();
    assert_eq!(store.reconcile_input_work(), initial, "unrelated claim rebuilds zero subjects");
    store.connection.write().execute(
        "UPDATE desired SET body='{\"changed\":true}' WHERE subject='resource/first'", [],
    ).unwrap();
    inputs.refresh(&store).unwrap();
    let changed = store.reconcile_input_work();
    assert_eq!(changed["full_rebuilds"], initial["full_rebuilds"]);
    assert_eq!(changed["subjects_rebuilt"].as_u64(), initial["subjects_rebuilt"].as_u64().map(|count| count + 1));
    assert_eq!(inputs.desired[0].desired["changed"], true);
    store.connection.write().execute("DELETE FROM desired WHERE subject='resource/first'", []).unwrap();
    inputs.refresh(&store).unwrap();
    assert_eq!(inputs.desired.len(), 1);
    assert_eq!(inputs.desired[0].subject, "resource/second");
    assert_eq!(store.reconcile_input_work()["full_rebuilds"], initial["full_rebuilds"]);
}

#[test]
fn projection_marker_changes_evaluate_only_owning_run_fence() {
    let store = fixture();
    let mut inputs = DesiredInputs::default();
    inputs.refresh(&store).unwrap();
    assert!(inputs.pending_subjects(&store).unwrap().is_empty());
    let initial = store.reconcile_input_work();
    assert!(inputs.pending_subjects(&store).unwrap().is_empty());
    assert_eq!(store.reconcile_input_work(), initial);
    let aggregate = Aggregate::RunPart("first".into());
    let key = pending_run_rebuild_key(&aggregate).unwrap();
    store.connection.write().execute("INSERT INTO meta(key,value) VALUES(?1,?2)",
        params![key, serde_json::to_string(&aggregate).unwrap()]).unwrap();
    assert_eq!(inputs.pending_subjects(&store).unwrap(), BTreeSet::from(["resource/first".into()]));
    let changed = store.reconcile_input_work();
    assert_eq!(changed["projection_fences_rebuilt"].as_u64(), initial["projection_fences_rebuilt"].as_u64().map(|count| count + 1));
    store.connection.write().execute("DELETE FROM meta WHERE key=?1", [&key]).unwrap();
    assert!(inputs.pending_subjects(&store).unwrap().is_empty());
    assert_eq!(store.reconcile_input_work()["projection_fences_rebuilt"].as_u64(), initial["projection_fences_rebuilt"].as_u64().map(|count| count + 2));
}
