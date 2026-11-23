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
            /* sqlite-large-value: allow-for-now lv-h862543af3dec owner=fractal_store::store::observer_reconcile_inputs_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO desired(subject,kind,revision,claim_id,body,owner_run)
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
    assert!(inputs.changed_subjects.is_empty());
    assert_eq!(store.reconcile_input_work(), initial, "no-change loans do no input work");
    assert_eq!(inputs.desired.as_ptr(), first, "unchanged roster is retained in place");
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h97cc452a3510 owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET body=body WHERE subject='resource/first'", []).unwrap();
    inputs.refresh(&store).unwrap();
    assert_eq!(store.reconcile_input_work(), initial, "same-value writes rebuild zero inputs");
    {
        let writer = store.connection.write();
        writer.execute_batch(/* sqlite-large-value: allow-for-now lv-hb4664af9faaf owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "BEGIN; UPDATE desired SET body='{\"rollback\":true}' WHERE subject='resource/first'; ROLLBACK").unwrap();
    }
    inputs.refresh(&store).unwrap();
    assert_eq!(store.reconcile_input_work(), initial, "rolled-back writes rebuild zero inputs");
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
        /* sqlite-large-value: allow-for-now lv-h5351379a643a owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET body='{\"changed\":true}' WHERE subject='resource/first'", [],
    ).unwrap();
    inputs.refresh(&store).unwrap();
    assert_eq!(inputs.changed_subjects, ["resource/first"]);
    let changed = store.reconcile_input_work();
    assert_eq!(changed["full_rebuilds"], initial["full_rebuilds"]);
    assert_eq!(changed["subjects_rebuilt"].as_u64(), initial["subjects_rebuilt"].as_u64().map(|count| count + 1));
    assert_eq!(inputs.desired[0].desired["changed"], true);
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h0b5273afe7b4 owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='resource/first'", []).unwrap();
    inputs.refresh(&store).unwrap();
    assert_eq!(inputs.desired.len(), 1);
    assert_eq!(inputs.desired[0].subject, "resource/second");
    assert_eq!(store.reconcile_input_work()["full_rebuilds"], initial["full_rebuilds"]);
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h61d6335d5dfb owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET revision='revision/changed' WHERE subject='resource/second'", []).unwrap();
    inputs.refresh(&store).unwrap();
    assert_eq!(inputs.changed_subjects, ["resource/second"],
        "revision-only changes invalidate authoritative family consumers of this journal");
}

#[test]
fn desired_journal_rollover_bounds_deleted_keys_and_rebuilds_stale_rosters() {
    let store = fixture();
    let mut inputs = DesiredInputs::default();
    inputs.refresh(&store).unwrap();
    let stale = inputs.generation.unwrap();
    let initial = store.reconcile_input_work();
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hab0cd697de5c owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject='resource/first'", [],
    ).unwrap();
    const BATCH: usize = 128;
    for batch in 0..=CHANGED_SUBJECT_LIMIT / BATCH {
        {
            let mut writer = store.connection.write();
            let transaction = writer.transaction().unwrap();
            {
                let mut insert = transaction.prepare(
                    /* sqlite-large-value: allow-for-now lv-h58a0560e9304 owner=fractal_store::store::observer_reconcile_inputs_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO desired(subject,kind,revision,claim_id,body)
                     VALUES(?1,'resource','test',(SELECT id FROM claims LIMIT 1),'{}')",
                ).unwrap();
                for item in 0..BATCH {
                    insert.execute(/* sqlite-large-value: allow-for-now lv-h13caf0b5c1cf owner=fractal_store::store::observer_reconcile_inputs_tests TODO=trace-statement-and-value-representation */ [format!("resource/churn-{}", batch * BATCH + item)]).unwrap();
                }
            }
            transaction.commit().unwrap();
        }
        assert!(store.fractal.reconcile_inputs.0.lock().unwrap().changed.len() <= CHANGED_SUBJECT_LIMIT);
        store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h403d3ff20c2a owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE subject GLOB 'resource/churn-*'", [],
        ).unwrap();
        let state = store.fractal.reconcile_inputs.0.lock().unwrap();
        assert!(state.changed.len() <= CHANGED_SUBJECT_LIMIT);
        assert_eq!(state.rows.len(), 1, "deleted keys remain only in the bounded journal");
    }
    let (current, changes) = store.desired_changes_since(Some(stale));
    assert!(current > stale);
    assert!(changes.is_none(), "a cursor predating eviction must rebuild");
    assert!(inputs.refresh(&store).unwrap());
    assert_eq!(inputs.desired.iter().map(|entry| entry.subject.as_str()).collect::<Vec<_>>(),
        ["resource/second"], "full refresh removes declarations missed while the consumer lagged");
    let rebuilt = store.reconcile_input_work();
    assert_eq!(rebuilt["full_rebuilds"].as_u64(), initial["full_rebuilds"].as_u64().map(|count| count + 1));
    assert!(!inputs.refresh(&store).unwrap());
    assert_eq!(store.reconcile_input_work(), rebuilt, "the current cursor does no extra work");
    assert_eq!(store.desired_changes_since(Some(current)), (current, Some(Vec::new())));
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
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h4c364653eba0 owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2)",
        params![key, serde_json::to_string(&aggregate).unwrap()]).unwrap();
    assert_eq!(inputs.pending_subjects(&store).unwrap(), BTreeSet::from(["resource/first".into()]));
    let changed = store.reconcile_input_work();
    assert_eq!(changed["projection_fences_rebuilt"].as_u64(), initial["projection_fences_rebuilt"].as_u64().map(|count| count + 1));
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-hbe733ff69aee owner=fractal_store::store::observer_reconcile_inputs_tests TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [&key]).unwrap();
    assert!(inputs.pending_subjects(&store).unwrap().is_empty());
    assert_eq!(store.reconcile_input_work()["projection_fences_rebuilt"].as_u64(), initial["projection_fences_rebuilt"].as_u64().map(|count| count + 2));
}
