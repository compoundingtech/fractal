//! Current statement proof controls; synthetic corruption is explicitly reader-only.
use super::*;

#[test]
fn empty_effect_authority_cost_does_not_grow_with_subject_history() {
    let store = Store::open_memory("amber").unwrap();
    direct(&store, &bundle("unowned", false), "legacy").unwrap();
    let captured = store.desired_subjects().unwrap().remove(0);
    let mut inserted = 0;
    let mut prior_vm = None;
    for history in [100, 10_000] {
        {
            // Reader-cost fixture only: these rows model retained runtime history, not
            // admission, canonical ordering, replication, or an owned-set receipt.
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            let batch = format!("reader-history-{history}");
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-haeeb16a679c7 owner=fractal_store::store::owned_sets_tests::empty_authority_guard TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                 VALUES(?1,'amber',?2,?1,'1')",
                params![batch, history],
            )
            .unwrap();
            {
                let mut insert = tx
                    .prepare(
                        /* sqlite-large-value: allow-for-now lv-h78aebb0d7874 owner=fractal_store::store::owned_sets_tests::empty_authority_guard TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
                         VALUES(?1,?2,?3,'harness.usage','amber',NULL,?4,'[]','1')",
                    )
                    .unwrap();
                for index in inserted..history {
                    for subject in [captured.subject.as_str(), "agent/unrelated-history"] {
                        insert
                            .execute(/* sqlite-large-value: allow-for-now lv-h44b70415ce36 owner=fractal_store::store::owned_sets_tests::empty_authority_guard TODO=trace-statement-and-value-representation */ params![
                                format!("reader-history-{subject}-{index}"),
                                batch,
                                subject,
                                json!({"fields": {"input_tokens": index}}).to_string()
                            ])
                            .unwrap();
                    }
                }
            }
            tx.commit().unwrap();
        }
        inserted = history;
        store.owned_desired_guard(&captured).unwrap();
        let before = smallclaims::sqlite::work::total();
        let (result, reads) = smallclaims::touched::record(|| store.owned_desired_guard(&captured));
        result.unwrap();
        let spent = smallclaims::sqlite::work::total() - before;
        assert!(reads.contains("kind:owned-set.revised"));
        assert!(reads.contains(&captured.subject));
        assert_eq!(spent.statements, 1, "history={history} {spent:?}");
        assert_eq!(spent.fullscan_steps, 0, "history={history} {spent:?}");
        assert!(spent.vm_steps <= 100, "history={history} {spent:?}");
        if let Some(prior) = prior_vm {
            assert!(spent.vm_steps <= prior + 10, "history={history} {spent:?}");
        }
        prior_vm = Some(spent.vm_steps);
        println!("empty-authority history-per-subject={history} cost={spent:?}");
    }
}

#[test]
fn empty_proof_errors_and_pinned_reads_do_not_authorize_effects() {
    let store = Store::open_memory("amber").unwrap();
    let declaration = bundle("unowned", false)
        .subjects
        .into_values()
        .next()
        .unwrap();
    store.owned_desired_guard(&declaration).unwrap();
    #[cfg(debug_assertions)]
    store
        .read_snapshot(|_| -> Result<()> {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    store.owned_desired_guard(&declaration)
                }))
                .is_err(),
                "an effect probe cannot reuse an enclosing read snapshot"
            );
            Ok(())
        })
        .unwrap();
    store.owned_desired_guard(&declaration).unwrap();
    // Isolated empty-store SQL failure injection; no admission/replication claim is made.
    store
        .connection
        .write()
        .execute_batch(/* sqlite-large-value: allow-for-now lv-he75efb18e0cc owner=fractal_store::store::owned_sets_tests::empty_authority_guard TODO=prepare-body-and-predecessor-values */ "DROP TABLE claims")
        .unwrap();
    let (failed, reads) = smallclaims::touched::record(|| store.owned_desired_guard(&declaration));
    assert_eq!(failed.unwrap_err().code, "internal");
    assert!(reads.contains("kind:owned-set.revised"));
    assert!(reads.contains(&declaration.subject));
}
