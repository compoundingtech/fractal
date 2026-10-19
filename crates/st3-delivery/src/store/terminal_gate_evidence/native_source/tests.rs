#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_tests_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_tests_shared::seed_native;
use super::*;
use crate::store::Store;
use serde_json::json;
use smallclaims::sqlite::work::SqliteWorkScope;


#[test]
fn native_gate_derives_eight_native_facts_and_preserves_canonical_selection() {
    let f = Fixture::seeded();
    f.edit(|tx| {
        // A later arrival with an older canonical time must not replace the selected exit.
        tx.execute(/* sqlite-large-value: allow-for-now lv-hf2f951d67b74 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
            SELECT 'late-old-observation',batch_id,subject,kind,origin,?1,'[]','0' FROM claims
            WHERE subject='exec/one/probe' AND kind='runtime.observed'",
            [json!({"fields":{"status":"exited","exit_code":3,"incarnation_id":"older"},"evidence":[]}).to_string()]).unwrap();
    });
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    let facts = f.source.extract(&tx).unwrap().unwrap();
    let claim_id = facts["observed"]["claim"].as_str().unwrap();
    let key = smallclaims::store::canonical::claim_key(&tx, claim_id).unwrap();
    let native = crate::store::latest_claim_of_kind_tx(&tx, "exec/one/probe", "runtime.observed")
        .unwrap()
        .unwrap();
    assert_eq!(native.id, claim_id);
    assert_ne!(claim_id, "late-old-observation");
    assert_eq!(key.5, claim_id);
    assert_eq!(facts["desired"]["step"], "step-run/one/prepare");
    assert_eq!(facts["run"]["generation"], facts["generation"]["id"]);
    let text = witness(&Value::String(facts.to_string()))
        .unwrap()
        .into_witness()
        .unwrap();
    assert_eq!(f.source.doctor_line(&tx)["message"], text);
    assert!(text.contains("exit code 2") && text.contains("will not restart"));
}

#[test]
fn native_gate_action_domain_refusal_and_recovery_preserve_unknown() {
    let f = Fixture::seeded();
    for kind in NON_SELECTING {
        f.edit(|tx| {
            // Existing native record writer; deliberately minimal raw action fixture inputs.
            crate::store::append_claim_record_tx(
                tx,
                "node",
                "exec/one/probe",
                kind,
                None,
                &json!({"fields":{"status":"exited"}}),
                &[],
                None,
            )
            .unwrap();
        });
        assert_eq!(f.line()["status"], "warn", "{kind}");
    }
    f.edit(|tx| {
        crate::store::append_claim_record_tx(
            tx,
            "node",
            "exec/one/probe",
            "runtime.action.unlisted",
            None,
            &json!({"fields":{}}),
            &[],
            None,
        )
        .unwrap();
    });
    assert_eq!(f.line()["status"], "unknown");
    f.edit(|tx| {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h1bf7561e801e owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE subject='exec/one/probe' AND kind='runtime.action.unlisted'",
            [],
        )
        .unwrap();
    });
    assert_eq!(f.line()["status"], "warn");
}

#[test]
fn native_gate_changed_declaration_and_generation_retract_without_read_repair() {
    let f = Fixture::seeded();
    f.edit(|tx| {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-hfe38611d2206 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_generation='run-generation/other' WHERE subject='exec/one/probe'",
            [],
        )
        .unwrap();
    });
    assert_eq!(f.line()["status"], "unknown");
    f.edit(|tx| {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h620f72625573 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_generation='run-generation/one' WHERE subject='exec/one/probe'",
            [],
        )
        .unwrap();
    });
    assert_eq!(f.line()["status"], "warn");
    f.edit(|tx| {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h782c1ec9684f owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET current_generation_id='other' WHERE id='one'",
            [],
        )
        .unwrap();
    });
    assert_eq!(f.line()["status"], "unknown");
}

#[test]
fn native_gate_caps_complete_subject_and_related_batch_rows() {
    for subject in ["exec/one/probe", "exec/same-batch"] {
        let f = Fixture::seeded();
        f.edit(|tx| {
            let batch:String=tx.query_row("SELECT batch_id FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed'",[],|r|r.get(0)).unwrap();
            for n in 0..17 {
                tx.execute(/* sqlite-large-value: allow-for-now lv-hbdda366dd539 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms)
                    VALUES(?1,?2,?3,'harness.observed','node','{}','[]','1')",
                    params![format!("overflow/{n}"),batch,subject]).unwrap();
            }
        });
        let line = f.line();
        assert_eq!(line["status"], "unknown");
        assert!(line["message"].as_str().unwrap().contains("cap"));
    }
}

#[test]
fn native_gate_octet_header_guard_refuses_oversize_and_proves_bundled_opcodes() {
    let f = Fixture::seeded();
    let mut writer = f.store.connection.write();
    let version: String = writer
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, "3.46.0", "bundled libsqlite3-sys 0.30.1 version");
    let tx = writer.transaction().unwrap();
    let opcodes = tx
        .prepare("EXPLAIN SELECT typeof(body),octet_length(body) FROM claims WHERE rowid=?1")
        .unwrap()
        .query_map([1], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(6)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    // The actual bundled sqlite3.c defines OPFLAG_BYTELENARG = 0xc0.
    assert!(
        opcodes
            .iter()
            .any(|(op, p5)| op == "Column" && (*p5 & 0xc0) == 0xc0),
        "{opcodes:?}"
    );
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h87a1cf3260c7 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=?1 WHERE subject='exec/one/probe' AND kind='runtime.observed'",
        [json!({"padding": "x".repeat(RAW_BYTES + 1)}).to_string()],
    )
    .unwrap();
    let scope = SqliteWorkScope::start();
    let rowid = point(&tx, "claims", "id", &{
        tx.query_row(
            "SELECT id FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    })
    .unwrap()
    .unwrap();
    let mut bytes = 0;
    assert!(bounded_row(&tx, "claims", rowid, &[("body", RAW_BYTES)], &mut bytes).is_err());
    let work = scope.finish();
    assert_eq!(bytes, 0);
    assert_eq!(work.fullscan_steps, 0);
    assert!(work.vm_steps > 0 && work.vm_steps < 200);
    f.source.finish(&tx).unwrap();
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
}

#[test]
fn native_gate_malformed_deep_and_wrong_type_source_is_visible_unknown() {
    // Native expression indexes refuse malformed claim JSON before any source mutation.
    // Keep that protection; do not drop indexes to inject an impossible claim state.
    let f = Fixture::seeded();
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    let capture = f.source.state(&tx).unwrap();
    let root = f.source.installer.root(&tx, VIEW).unwrap();
    assert!(tx.execute(
        /* sqlite-large-value: allow-for-now lv-hc087d153ef4b owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body='{broken' WHERE subject='exec/one/probe' AND kind='runtime.observed'",
        [],
    ).is_err());
    assert_eq!(f.source.state(&tx).unwrap(), capture);
    assert_eq!(f.source.installer.root(&tx, VIEW).unwrap(), root);
    assert_eq!(f.source.doctor_line(&tx)["status"], "warn");

    // Bundled SQLite accepts x'00' as JSONB. That native admission does not make it
    // a supported TEXT source: capture stays pending until the header guard fences it.
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h8bfdc3464d7b owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=x'00' WHERE subject='exec/one/probe' AND kind='runtime.observed'",
        [],
    )
    .unwrap();
    let kind: String = tx.query_row(
        "SELECT typeof(body) FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(kind, "blob");
    assert!(f.source.state(&tx).unwrap().1);
    assert!(f.source.installer.root(&tx, VIEW).is_err());
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
    f.source.finish(&tx).unwrap();
    let line = f.source.doctor_line(&tx);
    assert_eq!(line["status"], "unknown");
    assert!(line["message"].as_str().unwrap().contains("type"));
    drop(tx);
    drop(writer);

    // Mission metadata has no claim JSON expression index. Its admitted corrupt TEXT
    // reaches the adapter's predecode guard, and must retain a visible unavailable gap.
    for raw in [
        "{broken".to_owned(),
        format!("{}0{}", "[".repeat(17), "]".repeat(17)),
    ] {
        let f = Fixture::seeded();
        f.edit(|tx| {
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h6e2bd7355fdb owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_revisions SET body=?1 WHERE mission_id='one'",
                [raw],
            )
            .unwrap();
        });
        let line = f.line();
        assert_eq!(line["status"], "unknown");
        assert!(
            line["message"]
                .as_str()
                .unwrap()
                .starts_with("native evidence incomplete:")
        );
    }

    // Wrong-type retained payload also remains unavailable. Header inspection must
    // refuse fixture metadata before decoding the previous witness.
    let f = Fixture::seeded();
    f.edit(|tx| {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h3ba3dd3dc8b7 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET payload=x'00',pending=1 WHERE id=1",
            [],
        )
        .unwrap();
    });
    let line = f.line();
    assert_eq!(line["status"], "unknown");
    assert!(line["message"].as_str().unwrap().contains("type"));
}

#[test]
fn native_gate_replace_on_each_protected_table_stays_pending_without_wrapper() {
    for (table, predicate) in [
        ("desired", "subject='exec/one/probe'"),
        ("mission_revisions", "mission_id='one'"),
        ("mission_definitions", "mission_id='one'"),
        ("mission_runs", "id='one'"),
        ("run_generations", "id='one'"),
        ("step_runs", "subject='step-run/one/prepare'"),
        (
            "claims",
            "subject='exec/one/probe' AND kind='runtime.observed'",
        ),
        (
            "batches",
            "id=(SELECT batch_id FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed')",
        ),
    ] {
        let f = Fixture::seeded();
        let mut writer = f.store.connection.write();
        writer
            .execute_batch("PRAGMA recursive_triggers=OFF")
            .unwrap();
        let tx = writer.transaction().unwrap();
        tx.execute(
            &format!(/* sqlite-large-value: allow-for-now lv-hb4f49d843690 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO {table} SELECT * FROM {table} WHERE {predicate}"),
            [],
        )
        .unwrap();
        assert!(f.source.state(&tx).unwrap().1, "{table}");
        assert!(f.source.installer.root(&tx, VIEW).is_err(), "{table}");
        assert_eq!(f.source.doctor_line(&tx)["status"], "unknown", "{table}");
    }
    // These tables are outside the local-only policy. Their initial insert already fences;
    // reset ONLY the fixture pending bit to isolate replacement capture, never readiness.
    for (table, insertion) in [
        ("replica_records", /* sqlite-large-value: allow-for-now lv-ha0cc82f1a405 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref,writer,sequence,envelope_hash,position,raw,state,updated_at_unix_ms)
            VALUES('private-record','node',1,'private-envelope',0,x'00','repaired','1')"),
        ("checkpoint_claims", /* sqlite-large-value: allow-for-now lv-h663c949ac48d owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id,writer,sequence,envelope_hash,subject,kind,predecessors,accepted_at_unix_ms,checkpoint)
            VALUES('private-dropped','node',1,'private-envelope','exec/one/probe','runtime.observed','[]',1,'private-checkpoint')"),
        ("checkpoint_envelopes", /* sqlite-large-value: allow-for-now lv-ha2bcb657ed3f owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes(writer,sequence,envelope_hash,accepted_at_unix_ms,checkpoint)
            VALUES('node',1,'private-envelope',1,'private-checkpoint')"),
    ] {
        let f = Fixture::seeded();
        let mut writer = f.store.connection.write();
        writer.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
        let tx = writer.transaction().unwrap();
        tx.execute(/* sqlite-large-value: allow-for-now lv-hd2ee05ca8568 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=trace-statement-and-value-representation */ insertion, []).unwrap();
        assert!(f.source.state(&tx).unwrap().2.is_some(), "{table}");
        tx.execute(/* sqlite-large-value: allow-for-now lv-hdd00f541c6ce owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET pending=0 WHERE id=1", []).unwrap();
        let revision = f.source.state(&tx).unwrap().0;
        tx.execute(&format!(/* sqlite-large-value: allow-for-now lv-hf3b6e8cda15f owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO {table} SELECT * FROM {table}"), []).unwrap();
        let after = f.source.state(&tx).unwrap();
        assert!(after.1 && after.0 > revision && after.2.is_some(), "{table}");
        assert!(f.source.installer.root(&tx, VIEW).is_err(), "{table}");
        assert_eq!(f.source.doctor_line(&tx)["status"], "unknown", "{table}");
    }
}

#[test]
fn native_gate_rowid_collision_after_extraction_refuses_final_publication() {
    let f = Fixture::seeded();
    let mut writer = f.store.connection.write();
    writer
        .execute_batch("PRAGMA recursive_triggers=OFF")
        .unwrap();
    let tx = writer.transaction().unwrap();
    let before = f.source.state(&tx).unwrap().0;
    let _facts = f.source.extract(&tx).unwrap().unwrap();
    // Same claim id / different subject: DELETE triggers cannot be relied upon.
    tx.execute(/* sqlite-large-value: allow-for-now lv-h05cb4548a816 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=prepare-body-and-predecessor-values */ "INSERT OR REPLACE INTO claims
        SELECT store_index,id,batch_id,'exec/replaced',kind,origin,actor,body,predecessors,accepted_at_unix_ms
        FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed'",[]).unwrap();
    assert!(f.source.state(&tx).unwrap().0 > before);
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
    assert!(f.source.installer.root(&tx, VIEW).is_err());
}

#[test]
fn native_gate_transaction_rollback_restores_source_root_and_witness() {
    let f = Fixture::seeded();
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    let root = f.source.installer.root(&tx, VIEW).unwrap();
    let capture = f.source.state(&tx).unwrap();
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h3e685df04d97 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=trace-statement-and-value-representation */ "SAVEPOINT native_rollback").unwrap();
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h69773a46cfd1 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled' WHERE subject='step-run/one/prepare'",
        [],
    )
    .unwrap();
    f.source.finish(&tx).unwrap();
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h3ca35965ab8b owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=trace-statement-and-value-representation */ "ROLLBACK TO native_rollback;RELEASE native_rollback")
        .unwrap();
    assert_eq!(f.source.state(&tx).unwrap(), capture);
    assert_eq!(f.source.installer.root(&tx, VIEW).unwrap(), root);
    assert_eq!(f.source.doctor_line(&tx)["status"], "warn");
}

#[test]
fn native_gate_repair_checkpoint_owned_set_and_lifecycle_gaps_never_pass() {
    for family in [
        "repair",
        "checkpoint",
        "owned-set",
        "rebuild",
        "migrate",
        "restore",
    ] {
        let f = Fixture::seeded();
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        match family {
            "owned-set" => {
                crate::store::append_claim_record_tx(
                    &tx,
                    "node",
                    "owned-set/private",
                    "owned-set.revised",
                    None,
                    &json!({}),
                    &[],
                    None,
                )
                .unwrap();
            }
            "repair" => {
                // A raw repair/rank insertion into the private graph is outside LOCAL v0.
                tx.execute(/* sqlite-large-value: allow-for-now lv-h856289cb2c01 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET gap='unsupported repair',pending=1 WHERE id=1",[]).unwrap();
            }
            _ => {
                // Explicit fixture lifecycle fence; no production dispatch is claimed.
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hc700e17438f3 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET gap=?1,pending=1 WHERE id=1",
                    [format!("unsupported {family}")],
                )
                .unwrap();
            }
        }
        f.source.finish(&tx).unwrap();
        assert_eq!(f.source.doctor_line(&tx)["status"], "unknown", "{family}");
        assert!(f.source.installer.root(&tx, VIEW).is_err());
    }
}

#[test]
fn native_gate_raw_reopen_mutation_and_nonempty_attachment_refuse() {
    let f = Fixture::seeded();
    let raw = Connection::open(f.dir.path().join("claims.sqlite3")).unwrap();
    let mutation = /* sqlite-large-value: allow-for-now lv-ha1817011db56 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled' WHERE subject='step-run/one/prepare'";
    // Plain SQLite connections cannot bypass the native projection writer functions.
    let error = raw.execute(/* sqlite-large-value: allow-for-now lv-h66b7a005d0aa owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=trace-statement-and-value-representation */ mutation, []).unwrap_err();
    assert!(
        error.to_string().contains("st_projection_change"),
        "{error}"
    );
    assert_eq!(f.line()["status"], "warn");
    // Register the existing native writer functions, without the private source wrapper
    // or finish. The committed mutation must still leave its diagnostic cut pending.
    smallclaims::store::configure_projection_writer(&raw).unwrap();
    raw.execute(/* sqlite-large-value: allow-for-now lv-h0a296dd068c2 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=trace-statement-and-value-representation */ mutation, []).unwrap();
    assert_eq!(f.line()["status"], "unknown");
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    assert!(f.source.installer.root(&tx, VIEW).is_err());
    assert!(NativeSource::attach(&tx, f.source.binding.clone()).is_err());
}

#[test]
fn native_gate_guard_binding_replacement_is_refused_or_sticky_unknown() {
    let f = Fixture::seeded();
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    assert!(tx.execute(/* sqlite-large-value: allow-for-now lv-h190a41d1c9e1 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO test_native_terminal_guard SELECT * FROM test_native_terminal_guard", []).is_err());
    assert_eq!(f.source.doctor_line(&tx)["status"], "warn");
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h470afadec02b owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET exec='exec/replaced' WHERE id=1",
        [],
    )
    .unwrap();
    f.source.finish(&tx).unwrap();
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h2be6446c7ee6 owner=st3_delivery::store::terminal_gate_evidence::native_source::tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE test_native_terminal_guard SET exec='exec/one/probe' WHERE id=1",
        [],
    )
    .unwrap();
    f.source.finish(&tx).unwrap();
    assert_eq!(f.source.doctor_line(&tx)["status"], "unknown");
    assert!(f.source.installer.root(&tx, VIEW).is_err());
}

#[test]
fn native_gate_guard_inventory_and_default_store_have_no_public_wiring() {
    let f = Fixture::seeded();
    let default = Store::open_memory("unregistered").unwrap();
    let mut writer = default.connection.write();
    let tx = writer.transaction().unwrap();
    let any:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name LIKE 'test_native_%' OR name LIKE 'test_terminal_%')",
        [],|r|r.get(0)).unwrap();
    assert!(!any);
    let mut writer = f.store.connection.write();
    let tx = writer.transaction().unwrap();
    let tables=tx.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'test_%' ORDER BY name")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(
        tables,
        [
            "test_native_terminal_guard",
            "test_terminal_members",
            "test_terminal_meta"
        ]
    );
    let before = tx.total_changes();
    drop(tx);
    drop(writer);
    assert_eq!(f.line()["status"], "warn");
    assert_eq!(f.store.connection.write().total_changes(), before);
}
