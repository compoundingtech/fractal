//! The part of `store::mission_ivm` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`

use crate::MissionsStore;
use super::*;
use smallclaims::ivm::{Definition, LocalChange, Readiness, View, Views, source_cut};
pub use fractal_missions::store::mission_ivm::*;
pub use fractal_missions::store::mission_ivm::{MissionSelection, SOURCE, VIEW, clean, clock_page, definitions, flush, next_deadline, row, rows, window};

#[cfg(test)]
mod tests {
    use super::super::tests::{exchange_from, receive_and_project};
    use super::*;
    use smallclaims::ivm::{SourceCut, events};
    use fractal_missions::store::directive_notes::tests_shared_seats::*;
    use fractal_missions::store::directive_notes::tests_shared_seats::{FLEET, advertise, claim, declare, enable, store};
    use fractal_missions::store::mission_ivm_tests_shared_seats::*;
    use fractal_missions::store::mission_ivm_tests_shared_seats::{checkpoint, fixture, parity, register, start};
    use fractal_missions::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
    use fractal_missions::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::{Fixture, Saved, binding, observation};

    fn publish(store: &Store, id: &str, key: &str) {
        let source = format!(
            "version 2\nmission {id:?} state=\"ready\" {{ concurrent-runs max=4; goal \"Prepare a sample.\"; step \"build\" {{ goal \"Build a sample.\"; }} }}\n"
        );
        let intent = crate::parse_intent(&source, store.origin()).unwrap();
        store.apply_internal(&intent, key).unwrap();
    }

    #[test]
    fn real_store_selection_parity_and_bounded_seeks() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        for id in ["orchard", "harbor", "summit", "__st3/internal"] {
            publish(&store, id, &format!("publish-{id}"));
            parity(&store, &views, at);
            if id.starts_with("__st3/") {
                continue;
            }
            for n in 0..4 {
                start(&store, id, &format!("start-{id}-{n}"));
            }
            parity(&store, &views, at);
        }
        let connection = store.readers.get();
        let row = row(&connection, &views, at, "mission/orchard")
            .unwrap()
            .unwrap();
        assert_eq!(row["counts"]["running"], 4);
        assert_eq!(row["state"], "running");
        assert!(window(&connection, &views, true, at, None, None, 502).is_err());
    }

    #[test]
    fn changes_are_pending_until_flushed_and_rollback_is_atomic() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        publish(&store, "orchard", "publish");
        assert!(window(&store.readers.get(), &views, false, at, None, None, 3).is_err());
        parity(&store, &views, at);
        let run = start(&store, "orchard", "start");
        parity(&store, &views, at);
        let before = row(&store.readers.get(), &views, at, "mission/orchard").unwrap();
        let token = views.readiness(&store.readers.get(), VIEW, 1).unwrap();
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            tx.execute(
                "UPDATE mission_runs SET status='failed',phase='terminal' WHERE id=?1",
                [&run.id],
            )
            .unwrap();
            assert!(
                row(&tx, &views, at, "mission/orchard").is_err(),
                "unflushed projected source"
            );
            flush(&tx, &views, at, 1).unwrap();
            assert_ne!(row(&tx, &views, at, "mission/orchard").unwrap(), before);
            // Drop the source+counter+output+generation transaction without committing.
        }
        assert_eq!(
            row(&store.readers.get(), &views, at, "mission/orchard").unwrap(),
            before
        );
        assert_eq!(
            views.readiness(&store.readers.get(), VIEW, 1).unwrap(),
            token
        );
        parity(&store, &views, at);
    }

    #[test]
    fn old_new_membership_and_definition_removal_preserve_counts() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        publish(&store, "orchard", "publish");
        let run = start(&store, "orchard", "start");
        parity(&store, &views, at);
        {
            let writer = store.connection.write();
            writer
                .execute(
                    "UPDATE mission_runs SET mission_id='harbor',status='standing' WHERE id=?1",
                    [&run.id],
                )
                .unwrap();
        }
        parity(&store, &views, at);
        let old = row(&store.readers.get(), &views, at, "mission/orchard")
            .unwrap()
            .unwrap();
        assert_eq!(old["counts"], json!({}));
        let new = row(&store.readers.get(), &views, at, "mission/harbor")
            .unwrap()
            .unwrap();
        assert_eq!(new["counts"], json!({"standing":1}));
        {
            let writer = store.connection.write();
            writer
                .execute(
                    "DELETE FROM mission_definitions WHERE mission_id='orchard'",
                    [],
                )
                .unwrap();
        }
        parity(&store, &views, at);
        assert!(
            row(&store.readers.get(), &views, at, "mission/orchard")
                .unwrap()
                .is_none()
        );
        let changed = views
            .changed_keys(&store.readers.get(), VIEW, 1, 0, 100, None)
            .unwrap();
        assert!(
            changed.keys.iter().any(|key| key.key == "mission/orchard"),
            "removal retained in key feed"
        );
    }

    #[test]
    fn unrelated_writes_and_identical_run_updates_touch_no_output() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        publish(&store, "orchard", "publish");
        let run = start(&store, "orchard", "start");
        parity(&store, &views, at);
        let token = views.readiness(&store.readers.get(), VIEW, 1).unwrap();
        {
            let writer = store.connection.write();
            writer.execute_batch("CREATE TEMP TABLE mission_mutations(kind TEXT);
                CREATE TEMP TRIGGER mission_output_updated AFTER UPDATE ON local_mission_selection BEGIN INSERT INTO mission_mutations VALUES('update'); END;
                CREATE TEMP TRIGGER mission_output_inserted AFTER INSERT ON local_mission_selection BEGIN INSERT INTO mission_mutations VALUES('insert'); END;
                CREATE TEMP TRIGGER mission_output_deleted AFTER DELETE ON local_mission_selection BEGIN INSERT INTO mission_mutations VALUES('delete'); END;").unwrap();
            writer.execute("UPDATE mission_runs SET status=status,updated_at_unix_ms=updated_at_unix_ms WHERE id=?1",[&run.id]).unwrap();
        }
        store
            .append_claim(&ClaimInput {
                subject: "resource/sample".into(),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([("kind".into(), json!("custom.example.sample"))]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        parity(&store, &views, at);
        assert_eq!(
            views.readiness(&store.readers.get(), VIEW, 1).unwrap(),
            token
        );
        let count: u64 = store
            .connection
            .write()
            .query_row("SELECT COUNT(*) FROM mission_mutations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "unchanged tokens cannot conceal row rewrites");
    }

    #[test]
    fn replicated_permutations_and_duplicates_match_real_store_oracle() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let source = Store::open_memory("cedar").unwrap();
        for id in ["orchard", "harbor", "summit"] {
            publish(&source, id, &format!("publish-{id}"));
            start(&source, id, &format!("start-{id}"));
        }
        let exchange = exchange_from(&source, &ReplicationInventory::default());
        assert!(
            exchange.envelopes.len() >= 3,
            "meaningful envelope permutation"
        );
        for reverse in [false, true] {
            let (target, views) = fixture();
            let mut permuted = exchange.clone();
            if reverse {
                permuted.envelopes.reverse();
            }
            receive_and_project(&target, "cedar", &permuted);
            parity(&target, &views, at);
            receive_and_project(&target, "cedar", &permuted);
            parity(&target, &views, at);
            let expected = source.mission_collection_page(true, 0, 501, None).unwrap();
            assert_eq!(
                window(&target.readers.get(), &views, true, at, None, None, 501).unwrap(),
                expected
            );
        }
        // Split: reverse order in batches of one and two, each batch's predecessor delivered
        // again with it, with parity against the real store after every batch.
        for width in [1, 2] {
            let (target, views) = fixture();
            let order = exchange.envelopes.iter().rev().cloned().collect::<Vec<_>>();
            let mut previous = Vec::new();
            for chunk in order.chunks(width) {
                let mut batch = chunk.to_vec();
                batch.extend(previous.iter().cloned());
                receive_and_project(&target, "cedar", &fractal_store::store::tests_shared_seats::exchange_of("cedar", batch));
                parity(&target, &views, at);
                previous = chunk.to_vec();
            }
            assert_eq!(
                window(&target.readers.get(), &views, true, at, None, None, 501).unwrap(),
                source.mission_collection_page(true, 0, 501, None).unwrap()
            );
        }
    }

    #[test]
    fn captured_time_grace_deadlines_and_retirement_match_the_store() {
        struct Clock;
        impl Drop for Clock {
            fn drop(&mut self) {
                smallclaims::store::set_thread_clock(None);
            }
        }
        let _reset = Clock;
        let at = 1_800_000_000_000u128;
        smallclaims::store::set_thread_clock(Some(at));
        let (store, views) = fixture();
        publish(&store, "orchard", "publish");
        let run = start(&store, "orchard", "start");
        store
            .set_mission_run_state(&run.id, "cancelled", "terminal", Some("sample ended"))
            .unwrap();
        parity(&store, &views, at);
        let deadline = at + RECENTLY_ENDED_MS;
        for moment in [deadline - 1, deadline, deadline + 1] {
            smallclaims::store::set_thread_clock(Some(moment));
            parity(&store, &views, moment);
            let ids = window(&store.readers.get(), &views, false, moment, None, None, 5).unwrap();
            assert_eq!(!ids.is_empty(), moment <= deadline);
            assert_eq!(
                next_deadline(&store.readers.get(), &views, moment).unwrap(),
                (moment <= deadline).then_some(deadline + 1)
            );
        }
        smallclaims::store::set_thread_clock(Some(deadline + 1));
        store
            .retire_mission("mission/orchard", "person/avery", "retire")
            .unwrap();
        parity(&store, &views, deadline + 1);
        assert!(
            window(
                &store.readers.get(),
                &views,
                false,
                deadline + 1,
                None,
                None,
                5
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            next_deadline(&store.readers.get(), &views, deadline + 1).unwrap(),
            None
        );
    }

    #[test]
    fn filters_apply_before_slots_and_selected_headers_are_batched() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        for (n, id) in [
            "alpha",
            "birch",
            "cedar",
            "grove-100%",
            "grove_under",
            "Äpfel",
            "Σύνοδος",
        ]
        .iter()
        .enumerate()
        {
            publish(&store, id, &format!("publish-{n}"));
        }
        parity(&store, &views, at);
        let connection = store.readers.get();
        for filter in [
            "grove",
            "%",
            "_",
            "mission/grove",
            "absent",
            "ä",
            "Ä",
            "ÄPFEL",
            "ΣΎ",
        ] {
            let all = window(&connection, &views, true, at, None, None, 501).unwrap();
            let expected = all
                .into_iter()
                .filter(|(id, _)| id.to_lowercase().contains(&filter.to_lowercase()))
                .take(1)
                .collect::<Vec<_>>();
            assert_eq!(
                window(&connection, &views, true, at, None, Some(filter), 1).unwrap(),
                expected
            );
        }
        let ids = window(&connection, &views, true, at, None, None, 501)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        let selected = rows(&connection, &views, at, &ids).unwrap();
        assert_eq!(selected.len(), 7);
        for id in ids {
            assert_eq!(
                selected.get(&id),
                row(&connection, &views, at, &id).unwrap().as_ref()
            );
        }
    }

    #[test]
    fn reopen_preserves_rows_and_source_mutations_fence_instead_of_replaying() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.sqlite3");
        let store = Store::open(&path, "birch").unwrap();
        let views = register(&store);
        publish(&store, "orchard", "publish");
        start(&store, "orchard", "start");
        parity(&store, &views, at);
        let before = row(&store.readers.get(), &views, at, "mission/orchard").unwrap();
        let token = views.readiness(&store.readers.get(), VIEW, 1).unwrap();
        drop(store);
        let reopened = Store::open(&path, "birch").unwrap();
        assert_eq!(
            row(&reopened.readers.get(), &views, at, "mission/orchard").unwrap(),
            before
        );
        assert_eq!(
            views.readiness(&reopened.readers.get(), VIEW, 1).unwrap(),
            token
        );
        let claim = reopened
            .latest_claim("mission/orchard", Some("mission.published"))
            .unwrap()
            .unwrap();
        reopened
            .connection
            .write()
            .execute(
                "UPDATE claims SET accepted_at_unix_ms='1' WHERE id=?1",
                [claim.id],
            )
            .unwrap();
        assert_eq!(
            views.readiness(&reopened.readers.get(), VIEW, 1).unwrap(),
            Readiness::Fenced
        );
        assert!(row(&reopened.readers.get(), &views, at, "mission/orchard").is_err());
        let mut writer = reopened.connection.write();
        let tx = writer.transaction().unwrap();
        assert!(flush(&tx, &views, at, 128).is_err());
        assert!(!clean(&tx).unwrap(), "fenced source retains recovery work");
    }

    #[test]
    fn clock_pages_refuse_partial_membership_and_use_indexed_candidates() {
        let _clock = clock_snapshot();
        let at = now_ms();
        let (store, views) = fixture();
        for id in ["orchard", "harbor", "summit"] {
            publish(&store, id, &format!("publish-{id}"));
            let run = start(&store, id, &format!("start-{id}"));
            store
                .set_mission_run_state(&run.id, "cancelled", "terminal", None)
                .unwrap();
        }
        parity(&store, &views, at);
        let future: u128 = store
            .readers
            .get()
            .query_row(
                "SELECT MAX(grace_until_ms)+1 FROM local_mission_selection",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
            .into();
        assert!(
            window(&store.readers.get(), &views, false, future, None, None, 5).is_err(),
            "read never repairs overdue membership"
        );
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        assert_eq!(clock_page(&tx, &views, future, 1).unwrap(), 1);
        assert!(
            window(&tx, &views, false, future, None, None, 5).is_err(),
            "partial time page remains unavailable"
        );
        assert_eq!(clock_page(&tx, &views, future, 1).unwrap(), 1);
        assert_eq!(clock_page(&tx, &views, future, 1).unwrap(), 1);
        assert_eq!(clock_page(&tx, &views, future, 1).unwrap(), 0);
        assert!(
            window(&tx, &views, false, future, None, None, 5)
                .unwrap()
                .is_empty()
        );
        assert!(
            window(&tx, &views, false, at, None, None, 5).is_err(),
            "captured time cannot move backwards"
        );
        for (sql, index) in [
            (
                "SELECT mission_id FROM local_mission_selection WHERE visible=1 AND updated_ms<=1 AND (updated_ms<1 OR mission_id>'sample') ORDER BY updated_ms DESC,mission_id ASC LIMIT 5",
                "local_mission_selection_current_order",
            ),
            (
                "SELECT mission_id FROM local_mission_selection WHERE visible=1 AND grace_until_ms IS NOT NULL AND grace_until_ms<1 ORDER BY grace_until_ms LIMIT 5",
                "local_mission_selection_due",
            ),
            (
                "SELECT id FROM mission_runs WHERE mission_id='orchard' ORDER BY created_at_unix_ms DESC,id DESC LIMIT 1",
                "mission_ivm_latest_run",
            ),
        ] {
            let plan = tx
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains(index), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
        tx.commit().unwrap();
    }
}
