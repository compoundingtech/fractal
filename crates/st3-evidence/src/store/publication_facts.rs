//! The part of `store::publication_facts` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::PublicationStore;
use super::*;
pub use st3_store::store::publication_facts::*;
pub use st3_store::store::publication_facts::{PAGE, create_schema, flush, initialize, install_cache_observer, ready, reset};

pub trait PublicationFactsStore {
    fn publication_evidence_ready(&self) -> Result<bool>;
    /// At most 64 historical source rows and a 10ms processing budget per loan.
    /// The caller yields between jobs and stops on failure; the durable cursor resumes on reopen.
    fn advance_publication_evidence(&self) -> Result<bool>;
}

impl PublicationFactsStore for Store {

    fn publication_evidence_ready(&self) -> Result<bool> {
        ready(&self.readers.get())
    }

    fn advance_publication_evidence(&self) -> Result<bool> {
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        let pending: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_publication_pending) OR EXISTS(SELECT 1 FROM local_publication_batches)",
            [],
            |row| row.get(0),
        )?;
        // A loan does one bounded task. Exact deltas drain in runtime maintenance at commit;
        // otherwise this page touches only derived tables and creates no pending source delta.
        if !pending {
            backfill(&transaction)?;
        }
        transaction.commit()?;
        ready(&connection)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use st3_store::store::shared::*;
    use st3_store::store::publication_facts_tests_shared::*;
    use st3_store::store::publication_facts_tests_shared::{input};

    fn finish(store: &Store) {
        for _ in 0..10000 {
            if store.advance_publication_evidence().unwrap() {
                return;
            }
        }
        panic!("bootstrap did not finish");
    }

    fn oracle(store: &Store) {
        assert!(store.publication_evidence_ready().unwrap());
        let connection = store.readers.get();
        let query = format!(
            "SELECT id,subject,origin,kind,body FROM claims WHERE kind IN
            ('runtime.observed','runtime.action.succeeded') AND {UNREPAIRED_CLAIM}"
        );
        let rows = connection
            .prepare(&query)
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let mut expected = Vec::new();
        for (id, subject, origin, kind, body) in rows {
            let body: Value = serde_json::from_str(&body).unwrap();
            let fields = body.get("fields").unwrap_or(&body);
            if kind == "runtime.action.succeeded" && fields["action"] != "start" {
                continue;
            }
            expected.push((
                id.clone(),
                subject,
                origin,
                kind,
                fields["incarnation_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                canonical::sortable_key(&canonical::claim_key(&connection, &id).unwrap()),
            ));
        }
        expected.sort();
        let actual = connection
            .prepare(
                "SELECT claim_id,subject,origin,kind,incarnation,sort_key
            FROM local_publication_claims ORDER BY claim_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(actual, expected);
        let local = connection
            .prepare(
                "SELECT id,subject,COALESCE(json_extract(body,'$.fields.incarnation_id'),'')
            FROM local_observations WHERE kind='runtime.action.succeeded'
            AND json_extract(body,'$.fields.action')='start' ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let actual = connection
            .prepare(
                "SELECT observation_id,subject,incarnation FROM local_publication_starts
            ORDER BY observation_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(actual, local);
    }

    #[test]
    #[ignore = "private matching census fixture; native background/foreground contention; aggregates only"]
    fn publication_facts_native_foreground_wait_during_census_bootstrap() {
        let source =
            std::env::var_os("APPLY_RESTART_CENSUS_SCRATCH").expect("private matching fixture");
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("foreground.db");
        let store = Arc::new(Store::open(&path, "node").unwrap());
        {
            let mut writer=store.connection.write();
            let uri=format!("file:{}?mode=ro",std::path::Path::new(&source).display());
            writer.execute("ATTACH DATABASE ?1 AS matching",[uri]).unwrap();
            let tx=writer.transaction().unwrap();
            // Native schema and native writer; matching bodies and all canonical ordering
            // inputs are preserved. Unused envelope hash/raw/predecessor/time fields are
            // synthetic. This measures foreground admission, not replay/admission/health.
            tx.execute_batch("INSERT INTO main.batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
                  SELECT id,origin,replica_sequence,'fixture','0' FROM matching.batches;
                INSERT INTO main.claims(store_index,id,batch_id,subject,origin,kind,body,predecessors,accepted_at_unix_ms)
                  SELECT store_index,id,batch_id,subject,origin,kind,body,'[]',accepted_at_unix_ms FROM matching.claims;
                INSERT INTO main.replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,claim_id,updated_at_unix_ms)
                  SELECT 'fixture:'||source.rowid,batch.origin,batch.replica_sequence,'fixture:'||source.rowid,source.position,X'',source.state,source.claim_id,'0'
                  FROM matching.replica_records source JOIN matching.claims claim ON claim.id=source.claim_id
                  JOIN matching.batches batch ON batch.id=claim.batch_id;
                INSERT INTO main.local_observations(id,subject,kind,body,after_store_index,observed_at_unix_ms)
                  SELECT id,subject,kind,body,after_store_index,0 FROM matching.local_observations;").unwrap();
            reset(&tx).unwrap();
            tx.commit().unwrap();
            writer.execute_batch("DETACH DATABASE matching").unwrap();
        }
        assert!(!store.publication_evidence_ready().unwrap());
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_done = done.clone();
        let worker_store = store.clone();
        struct BootstrapDone(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for BootstrapDone {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        let worker = std::thread::spawn(move || {
            let _finished = BootstrapDone(worker_done.clone());
            let start = std::time::Instant::now();
            let mut pages = 0;
            loop {
                let complete = worker_store.advance_publication_evidence().unwrap();
                pages += 1;
                if complete {
                    break;
                }
                // Production releases the same background guard and yields between pages.
                std::thread::yield_now();
            }
            worker_done.store(true, std::sync::atomic::Ordering::Release);
            (pages, start.elapsed().as_secs_f64())
        });
        let mut waits = Vec::new();
        let mut holds = Vec::new();
        let mut totals = Vec::new();
        while !done.load(std::sync::atomic::Ordering::Acquire) {
            let started = std::time::Instant::now();
            let mut writer = store.connection.write();
            let wait = started.elapsed().as_micros();
            let acquired = std::time::Instant::now();
            let tx = writer.transaction().unwrap();
            let input = ClaimInput {
                subject: "agent/example/latency".into(),
                kind: "harness.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("info")),
                    ("code".into(), json!("bootstrap-contention-probe")),
                    (
                        "reason".into(),
                        json!(format!("foreground-{}", waits.len())),
                    ),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            };
            insert_local_observation_tx(&tx, "node", &input, now_ms()).unwrap();
            tx.commit().unwrap();
            drop(writer);
            waits.push(wait);
            holds.push(acquired.elapsed().as_micros());
            totals.push(started.elapsed().as_micros());
        }
        let (pages, seconds) = worker.join().unwrap();
        assert!(!waits.is_empty());
        let samples = waits.len();
        waits.sort();
        holds.sort();
        totals.sort();
        println!(
            "publication foreground contention samples={samples} bootstrap_pages={pages} bootstrap_seconds={seconds} wait_p95_us={} wait_max_us={} hold_p95_us={} hold_max_us={} total_p95_us={} total_max_us={}",
            waits[samples * 95 / 100],
            waits.last().unwrap(),
            holds[samples * 95 / 100],
            holds.last().unwrap(),
            totals[samples * 95 / 100],
            totals.last().unwrap()
        );
        assert!(store.publication_evidence_ready().unwrap());
    }

    #[test]
    #[ignore = "native delete counters: run alone with --ignored --test-threads=1 --nocapture"]
    fn publication_facts_native_delete_cost() {
        use smallclaims::sqlite::work;
        for after in [false, true] {
            for (batch_size, total_rows) in [(1, 100), (1000, 1000)] {
                for runtime in [false, true] {
                    let root = tempfile::tempdir().unwrap();
                    let path = root.path().join("delete.db");
                    let baseline = (!after).then(|| {
                        GraphStore::open(&path, "node", Arc::new(SmalltalkRuntime::default()))
                            .unwrap()
                    });
                    let revised = after.then(|| Store::open(&path, "node").unwrap());
                    let graph = baseline
                        .as_ref()
                        .unwrap_or_else(|| &revised.as_ref().unwrap().graph);
                    if !after {
                        let names=graph.readers.get().prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND name LIKE 'publication_%'").unwrap()
                            .query_map([],|row|row.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                        let writer = graph.connection.write();
                        for name in names {
                            writer
                                .execute_batch(&format!("DROP TRIGGER {name}"))
                                .unwrap();
                        }
                    }
                    let mut ids = Vec::new();
                    {
                        let mut writer = graph.connection.write();
                        let tx = writer.transaction().unwrap();
                        let mut batch = None;
                        for i in 0..total_rows {
                            let kind = if runtime {
                                "runtime.observed"
                            } else {
                                "runtime.action.requested"
                            };
                            let fields = if runtime {
                                json!({"status":"running","incarnation_id":format!("delete-{i}")})
                            } else {
                                json!({"action":"screen","reason":format!("delete-{i}")})
                            };
                            let row = append_claim_tx(
                                &tx,
                                "node",
                                "agent/example/worker",
                                kind,
                                None,
                                &json!({"fields":fields}),
                                &[],
                                batch.as_deref(),
                            )
                            .unwrap();
                            if batch_size > 1 {
                                batch = Some(row.batch_id);
                            }
                            ids.push(row.id);
                        }
                        tx.commit().unwrap();
                    }
                    if let Some(store) = &revised {
                        finish(store);
                    }
                    let mut costs = work::SqliteWork::default();
                    let mut holds = Vec::new();
                    let mut repair_cost = work::SqliteWork::default();
                    let mut repair_holds = Vec::new();
                    // One native delete statement per chunk, including maintenance and commit observer.
                    for chunk in ids.chunks(100) {
                        let mut writer = graph.connection.write();
                        let before = work::total();
                        let started = std::time::Instant::now();
                        let tx = writer.transaction().unwrap();
                        let arguments = std::iter::repeat_n("?", chunk.len())
                            .collect::<Vec<_>>()
                            .join(",");
                        tx.execute(
                            &format!("DELETE FROM claims WHERE id IN ({arguments})"),
                            rusqlite::params_from_iter(chunk),
                        )
                        .unwrap();
                        tx.commit().unwrap();
                        drop(writer);
                        holds.push(started.elapsed().as_micros());
                        let cost = work::total() - before;
                        costs.vm_steps += cost.vm_steps;
                        costs.statements += cost.statements;
                        if let Some(store) = &revised {
                            while !store.publication_evidence_ready().unwrap() {
                                let mut writer = graph.connection.write_background();
                                let before = work::total();
                                let started = std::time::Instant::now();
                                writer.transaction().unwrap().commit().unwrap();
                                drop(writer);
                                repair_holds.push(started.elapsed().as_micros());
                                let cost = work::total() - before;
                                repair_cost.vm_steps += cost.vm_steps;
                                repair_cost.statements += cost.statements;
                            }
                        }
                    }
                    holds.sort();
                    println!(
                        "publication delete after={after} runtime={runtime} batch_size={batch_size} rows={total_rows} chunk=100: vm_per_row={} statements={} chunk_hold_median_us={} chunk_hold_max_us={}",
                        costs.vm_steps / total_rows as u64,
                        costs.statements,
                        holds[holds.len() / 2],
                        holds.last().unwrap()
                    );
                    repair_holds.sort();
                    println!(
                        "publication delete repair after={after} runtime={runtime} batch_size={batch_size}: vm_per_deleted_row={} statements={} loans={} loan_hold_p95_us={} loan_hold_max_us={}",
                        repair_cost.vm_steps / total_rows as u64,
                        repair_cost.statements,
                        repair_holds.len(),
                        repair_holds
                            .get(repair_holds.len() * 95 / 100)
                            .copied()
                            .unwrap_or(0),
                        repair_holds.last().copied().unwrap_or(0)
                    );
                    if let Some(store) = &revised {
                        oracle(store);
                    }
                }
            }
        }
    }

    #[test]
    fn publication_facts_batch_deletion_coalesces_and_repairs_legacy_positions() {
        let store = Store::open_memory("node").unwrap();
        let mut ids = Vec::new();
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            let mut batch = None;
            for i in 0..1000 {
                let row = append_claim_tx(
                    &tx,
                    "node",
                    "agent/example/worker",
                    "runtime.observed",
                    None,
                    &json!({"fields":{"status":"running","incarnation_id":format!("batch-{i}")}}),
                    &[],
                    batch.as_deref(),
                )
                .unwrap();
                batch = Some(row.batch_id);
                ids.push(row.id);
            }
            tx.commit().unwrap();
        }
        finish(&store);
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        for id in &ids[..500] {
            tx.execute("DELETE FROM claims WHERE id=?1", [id]).unwrap();
        }
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM local_publication_batches",
                [],
                |row| row.get::<_, usize>(0)
            )
            .unwrap(),
            1
        );
        tx.commit().unwrap();
        drop(writer);
        assert!(!store.publication_evidence_ready().unwrap());
        finish(&store);
        oracle(&store);
    }

    #[test]
    fn publication_facts_native_checkpoint_trim_keeps_only_retained_sources() {
        let store = Store::open_memory("node").unwrap();
        for i in 0..4 {
            store
                .append_claim(&input("runtime.observed", &format!("inc-{i}")))
                .unwrap();
            // Legacy shared system receipts use the actor-null checkpoint slot. Modern
            // actor-null receipts are local; requester-authored shared receipts never drop.
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            append_claim_tx(
                &tx,
                "node",
                "agent/example/worker",
                "runtime.action.succeeded",
                None,
                &json!({"fields":{"action":"start","incarnation_id":"legacy",
                    "desired_token":"not-retained","reason":format!("receipt-{i}")}}),
                &[],
                None,
            )
            .unwrap();
            tx.commit().unwrap();
            drop(writer);
            // The newest envelope is protected independently of the kind's drop rule.
            store.seal_local_batches().unwrap();
        }
        let before: u64 = store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
            .unwrap();
        assert!(store.trim_checkpoint_for_test(now_ms() + 6 * 24 * 60 * 60 * 1000) > 0);
        finish(&store);
        oracle(&store);
        let after: u64 = store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
            .unwrap();
        assert!(after < before);
        assert!(
            store
                .readers
                .get()
                .query_row("SELECT COUNT(*) FROM checkpoint_claims", [], |row| row
                    .get::<_, u64>(0))
                .unwrap()
                > 0
        );
    }

    #[test]
    fn publication_facts_cover_native_append_local_retention_and_forget() {
        let store = Store::open_memory("node").unwrap();
        store
            .append_claim(&input("runtime.observed", "one"))
            .unwrap();
        store
            .append_claim(&input("runtime.action.succeeded", "one"))
            .unwrap();
        let mut local = input("runtime.action.succeeded", "local");
        local.actor = None;
        store.append_claim(&local).unwrap();
        oracle(&store);
        store.forget_current_views();
        oracle(&store);
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        tx.execute("UPDATE local_observations SET body=json_set(body,'$.fields.incarnation_id','replacement')",[]).unwrap();
        tx.commit().unwrap();
        drop(connection);
        oracle(&store);
        store
            .connection
            .write()
            .execute("DELETE FROM local_observations", [])
            .unwrap();
        oracle(&store);
    }

    #[test]
    fn publication_facts_replicated_admission_and_full_replay_match_recompute() {
        let source = Store::open_memory("writer").unwrap();
        source
            .append_claim(&input("runtime.observed", "one"))
            .unwrap();
        source
            .append_claim(&input("runtime.action.succeeded", "one"))
            .unwrap();
        let target = Store::open_memory("receiver").unwrap();
        target
            .import_replication("writer", &source.export_replication(0).unwrap())
            .unwrap();
        finish(&target);
        oracle(&target);
        target.replay_replication_graph().unwrap();
        assert!(!target.publication_evidence_ready().unwrap());
        finish(&target);
        oracle(&target);
    }

    #[test]
    fn publication_facts_native_record_repair_excludes_the_repaired_source() {
        let store = Store::open_memory("node").unwrap();
        let old = store
            .append_claim(&input("runtime.observed", "old"))
            .unwrap();
        let new = store
            .append_claim(&input("runtime.observed", "new"))
            .unwrap();
        store.seal_local_batches().unwrap();
        let record: String = store
            .readers
            .get()
            .query_row(
                "SELECT record_ref FROM replica_records WHERE claim_id=?1",
                [&old.id],
                |row| row.get(0),
            )
            .unwrap();
        // Reproduce a retained source whose envelope needs repair, then use the native
        // repair publication path. A receipt alone does not change record admission state.
        store
            .connection
            .write()
            .execute(
                "UPDATE replica_records SET state='invalid' WHERE record_ref=?1",
                [&record],
            )
            .unwrap();
        let source = format!(
            "version 2\nrepair {record:?} {{ replacement {:?}; reason \"synthetic native repair\"; }}",
            new.id
        );
        let intent = crate::graph::parse_intent(&source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source,
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(
                &intent,
                &preview.subject_tokens,
                "native-publication-repair",
            )
            .unwrap();
        finish(&store);
        oracle(&store);
        assert!(
            !store
                .readers
                .get()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_publication_claims WHERE claim_id=?1)",
                    [&old.id],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
    }

    #[test]
    fn publication_facts_repair_retraction_and_checkpoint_source_deletion() {
        let store = Store::open_memory("node").unwrap();
        let a = store
            .append_claim(&input("runtime.observed", "one"))
            .unwrap();
        let b = store
            .append_claim(&input("runtime.action.succeeded", "one"))
            .unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute(
                "INSERT INTO projection_digest_repaired_claims VALUES(?1)",
                [&a.id],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        oracle(&store);
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute(
                "DELETE FROM projection_digest_repaired_claims WHERE id=?1",
                [&a.id],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        oracle(&store);
        // Trimming and tombstoning delete this same source row; the tombstone never
        // reconstructs a missing launch body or declaration. Retained evidence is exact.
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute("DELETE FROM claims WHERE id=?1", [&b.id])
                .unwrap();
            tx.commit().unwrap();
        }
        oracle(&store);
        assert!(
            store
                .publication_launch("agent/example/worker", "one")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn publication_facts_late_canonical_claim_and_metadata_corrections_do_not_regress() {
        let store = Store::open_memory("node").unwrap();
        let first = store
            .append_claim(&input("runtime.observed", "first"))
            .unwrap();
        let late = store
            .append_claim(&input("runtime.observed", "late"))
            .unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            tx.execute(
                "UPDATE claims SET accepted_at_unix_ms='1' WHERE id=?1",
                [&late.id],
            )
            .unwrap();
            tx.execute(
                "UPDATE batches SET replica_sequence=replica_sequence+100 WHERE id=?1",
                [&first.batch_id],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        oracle(&store);
        let connection = store.readers.get();
        let selected: String = connection
            .query_row(
                "SELECT claim_id FROM local_publication_claims
            WHERE kind='runtime.observed' ORDER BY sort_key DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(selected, first.id);
    }

    #[test]
    fn publication_facts_second_open_preserves_schema_cookie() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("reopen.db");
        let store = Store::open(&path, "node").unwrap();
        let cookie: i64 = store
            .readers
            .get()
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        assert_eq!(
            cookie,
            reopened
                .readers
                .get()
                .query_row::<i64, _, _>("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap()
        );
        assert!(reopened.publication_evidence_ready().unwrap());
        // A genuinely older record trigger upgrades once; unchanged reopens do not
        // repeat that migration or leave the stale body installed.
        reopened.connection.write().execute_batch(
            "DROP TRIGGER directive_note_record_insert; CREATE TRIGGER directive_note_record_insert
             AFTER INSERT ON replica_records BEGIN SELECT 1; END;"
        ).unwrap();
        drop(reopened);
        let migrated = Store::open(&path, "node").unwrap();
        let definition: String = migrated
            .readers
            .get()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='directive_note_record_insert'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(definition.contains("local_directive_note_dirty"));
        let migrated_cookie: i64 = migrated
            .readers
            .get()
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        drop(migrated);
        let stable = Store::open(&path, "node").unwrap();
        assert_eq!(
            migrated_cookie,
            stable
                .readers
                .get()
                .query_row::<i64, _, _>("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap()
        );
    }

    #[test]
    fn publication_facts_bootstrap_is_durable_bounded_and_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("graph.db");
        let store = Store::open(&path, "node").unwrap();
        for _ in 0..(PAGE * 3) {
            store
                .append_claim(&input("runtime.observed", "one"))
                .unwrap();
        }
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            reset(&tx).unwrap();
            tx.commit().unwrap();
        }
        drop(store);
        let store = Store::open(&path, "node").unwrap();
        assert!(!store.publication_evidence_ready().unwrap());
        let desired = crate::graph::parse_intent(
            "version 2\nagent \"example/worker\" { command \"true\"; }",
            "node",
        )
        .unwrap();
        let check = super::super::publication::check(
            &store.readers.get(),
            &desired.subjects["agent/example/worker"],
            u64::MAX,
            "node",
        )
        .unwrap();
        assert_eq!(
            check.interruption,
            crate::publication::Interruption::Unknown
        );
        assert_eq!(
            check.unknown_reason,
            Some("publication-evidence-incomplete")
        );
        assert!(!store.advance_publication_evidence().unwrap());
        let count: u64 = store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM local_publication_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(count > 0 && count <= PAGE as u64);
        let cursor: u64 = store
            .readers
            .get()
            .query_row(
                "SELECT cursor FROM local_publication_bootstrap",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(store);
        let reopened = Store::open(&path, "node").unwrap();
        assert_eq!(
            cursor,
            reopened
                .readers
                .get()
                .query_row(
                    "SELECT cursor FROM local_publication_bootstrap",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap()
        );
        finish(&reopened);
        oracle(&reopened);
    }

    #[test]
    fn publication_facts_shuffled_deltas_equal_full_recompute() {
        // Deterministic property sweep: shuffled inserts, repairs, unrepaired restores,
        // updates, local trimming and source deletions across several canonical batches.
        for seed in 1..=8_u64 {
            let store = Store::open_memory("node").unwrap();
            let mut random = seed;
            let mut claims = Vec::new();
            for step in 0..80 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                match (random >> 32) % 7 {
                    0 | 1 => claims.push(
                        store
                            .append_claim(&input(
                                if step % 2 == 0 {
                                    "runtime.observed"
                                } else {
                                    "runtime.action.succeeded"
                                },
                                &format!("inc-{step}"),
                            ))
                            .unwrap(),
                    ),
                    2 => {
                        let mut local = input("runtime.action.succeeded", &format!("local-{step}"));
                        local.actor = None;
                        store.append_claim(&local).unwrap();
                    }
                    action => {
                        if let Some(claim) = claims.get((random as usize) % claims.len().max(1)) {
                            let mut connection = store.connection.write();
                            let tx = connection.transaction().unwrap();
                            match action {
                                3 => {
                                    tx.execute("INSERT OR IGNORE INTO projection_digest_repaired_claims VALUES(?1)",[&claim.id]).unwrap();
                                }
                                4 => {
                                    tx.execute(
                                        "DELETE FROM projection_digest_repaired_claims WHERE id=?1",
                                        [&claim.id],
                                    )
                                    .unwrap();
                                }
                                5 => {
                                    tx.execute(
                                        "UPDATE claims SET accepted_at_unix_ms=?2 WHERE id=?1",
                                        params![claim.id, (random % 1000).to_string()],
                                    )
                                    .unwrap();
                                }
                                _ => {
                                    tx.execute("DELETE FROM claims WHERE id=?1", [&claim.id])
                                        .unwrap();
                                    tx.execute("DELETE FROM local_observations WHERE id%2=0", [])
                                        .unwrap();
                                }
                            }
                            tx.commit().unwrap();
                        }
                    }
                }
                finish(&store);
                oracle(&store);
            }
        }
    }

    #[test]
    fn publication_facts_pending_loan_drains_only_one_page() {
        let store = Store::open_memory("node").unwrap();
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        for i in 0..192 {
            let claim = input("runtime.observed", &format!("pending-{i}"));
            append_claim_tx(
                &tx,
                "node",
                &claim.subject,
                &claim.kind,
                claim.actor.as_deref(),
                &json!({"fields": claim.fields}),
                &[],
                None,
            )
            .unwrap();
        }
        tx.commit().unwrap();
        drop(connection);
        let pending = || {
            store
                .readers
                .get()
                .query_row(
                    "SELECT COUNT(*) FROM local_publication_pending",
                    [],
                    |row| row.get::<_, usize>(0),
                )
                .unwrap()
        };
        let before = pending();
        assert!(before >= 128);
        assert!(!store.advance_publication_evidence().unwrap());
        let after = pending();
        assert!(after < before);
        assert!(before - after <= PAGE);
        finish(&store);
        oracle(&store);
    }
}
