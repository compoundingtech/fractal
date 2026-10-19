#![allow(unused_imports)]
use crate::PublicationFactsStore;
use st3_attention::store::terminal_gate_evidence::native_source_shared::*;
use super::*;
use crate::graph::{parse_execution_intent, parse_intent};
use crate::store::{Store, append_claim_tx, project_mission_run_created, select_replicated_desired, select_replicated_mission};
use serde_json::json;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use st3_attention::store::terminal_gate_evidence::native_source_shared::*;
use st3_attention::store::terminal_gate_evidence::native_source_shared::{Binding, FINGERPRINT, GUARD, NativeSource, RAW_BYTES, ROWS, SOURCE_BYTES, bounded_row, decode, point};
use st3_attention::store::terminal_gate_evidence::native_source_shared::{required};
use st3_attention::store::terminal_gate_evidence::native_source_tests_shared::*;
use st3_attention::store::terminal_gate_evidence::native_source_tests_shared::{Fixture, seed_native};


#[test]
fn native_gate_unrelated_and_noop_work_at_zero_1024_and_100000_rows() {
    for n in [0, 1024, 100_000] {
        let f = Fixture::seeded();
        if n > 0 {
            f.populate_foreign(n);
        }
        // Bulk admission intentionally leaves bounded publication-maintenance pages.
        // Finish that independent work before measuring an idle terminal-gate transaction;
        // its commit would otherwise correctly drain a publication page after the scope ends.
        while !f.store.advance_publication_evidence().unwrap() {}
        {
            let writer = f.store.connection.write();
            writer
                .execute_batch(
                    /* sqlite-large-value: allow-for-now lv-hb8b81926488f owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE witness_writes(n INTEGER NOT NULL);
                INSERT INTO witness_writes VALUES(0);",
                )
                .unwrap();
            for table in ["test_terminal_members", "test_terminal_meta"] {
                for event in ["INSERT", "UPDATE", "DELETE"] {
                    writer
                        .execute_batch(&format!(
                            /* sqlite-large-value: allow-for-now lv-h73da0ee5ce63 owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER writes_{table}_{event}
                        AFTER {event} ON main.{table} BEGIN UPDATE witness_writes SET n=n+1; END;"
                        ))
                        .unwrap();
                }
            }
        }
        let before = f.store.connection.write().total_changes();
        let work = f.measured(|_| {});
        eprintln!("native source idle finish unrelated={n}: {work:?}");
        assert_eq!(work.statements, 1, "{n}: {work:?}");
        assert_eq!(work.fullscan_steps, 0, "{n}: {work:?}");
        assert!(work.vm_steps > 0 && work.vm_steps <= 40, "{n}: {work:?}");
        assert_eq!(f.store.connection.write().total_changes(), before);
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        let root = f.source.installer.root(&tx, VIEW).unwrap();
        let capture = f.source.state(&tx).unwrap();
        tx.commit().unwrap();
        drop(writer);
        // Establish the unrelated batch outside the measured INSERT. Report one claim
        // admission itself, including every native and fixture trigger WHEN predicate.
        f.store.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h8ac771d5e3c1 owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES('foreign','node',999,'fixture-foreign','1')",
            [],
        ).unwrap();
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        let insert_scope = SqliteWorkScope::start();
        tx.execute(/* sqlite-large-value: allow-for-now lv-h431c074919c2 owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
            VALUES('measured-unrelated','foreign','exec/measured-unrelated','runtime.observed','node','{}','[]','1')", []).unwrap();
        let insert_work = insert_scope.finish();
        eprintln!(
            "native source per unrelated claim INSERT prior_rows={n} insert_count=1: {insert_work:?}"
        );
        assert_eq!(insert_work.statements, 1, "{n}: {insert_work:?}");
        assert!(
            insert_work.vm_steps > 0 && insert_work.vm_steps <= 20_000,
            "{n}: {insert_work:?}"
        );
        assert_eq!(insert_work.fullscan_steps, 0, "{n}: {insert_work:?}");
        let finish_scope = SqliteWorkScope::start();
        f.source.finish(&tx).unwrap();
        let finish_work = finish_scope.finish();
        eprintln!("native source post-INSERT idle finish prior_rows={n}: {finish_work:?}");
        assert_eq!(finish_work.statements, 1, "{n}: {finish_work:?}");
        assert!(
            finish_work.vm_steps > 0 && finish_work.vm_steps <= 40,
            "{n}: {finish_work:?}"
        );
        assert_eq!(finish_work.fullscan_steps, 0, "{n}: {finish_work:?}");
        assert_eq!(f.source.state(&tx).unwrap(), capture);
        assert_eq!(f.source.installer.root(&tx, VIEW).unwrap(), root);
        tx.commit().unwrap();
        drop(writer);
        // At n=0 this is a genuine absent-key point operation; the larger sources update
        // one unrelated row. Neither may capture a source change or write a witness.
        let unrelated = f.measured(|tx| {
            tx.execute(/* sqlite-large-value: allow-for-now lv-ha6b3a3fc433e owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=body WHERE id='foreign/1'", [])
                .unwrap();
        });
        eprintln!("native source unrelated path rows={n}: {unrelated:?}");
        assert!(
            unrelated.statements > 0 && unrelated.statements <= 96 && unrelated.vm_steps <= 20_000,
            "{n}: {unrelated:?}"
        );
        assert_eq!(unrelated.fullscan_steps, 0, "{n}: {unrelated:?}");
        {
            let mut writer = f.store.connection.write();
            let tx = writer.transaction().unwrap();
            assert_eq!(f.source.state(&tx).unwrap(), capture);
            assert_eq!(f.source.installer.root(&tx, VIEW).unwrap(), root);
        }
        let work=f.measured(|tx|{
            tx.execute(/* sqlite-large-value: allow-for-now lv-h66a622cd5390 owner=st3_evidence::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=body WHERE subject='exec/one/probe' AND kind='runtime.observed'",[]).unwrap();
        });
        eprintln!("native source full no-op path unrelated={n}: {work:?}");
        assert!(
            work.statements > 0 && work.statements <= 96 && work.vm_steps <= 20_000,
            "{n}: {work:?}"
        );
        assert_eq!(work.fullscan_steps, 0, "{n}: {work:?}");
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        let after = f.source.installer.root(&tx, VIEW).unwrap();
        assert!(after.revision > root.revision);
        assert_eq!(after.generation, root.generation);
        let output_writes: i64 = tx
            .query_row("SELECT n FROM witness_writes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(output_writes, 0);
        assert_eq!(f.source.doctor_line(&tx)["status"], "warn");
    }
}
