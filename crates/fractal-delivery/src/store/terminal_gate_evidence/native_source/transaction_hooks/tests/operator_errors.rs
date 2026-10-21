#![allow(unused_imports)]
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_operator_error_shared_seats::{Cell, Disposition, Origin};
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_delivery::*;
use super::*;
use rusqlite::{ErrorCode, ffi};
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_delivery::*;
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_delivery::{answer, update_exit};
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_operator_errors_shared_delivery::*;
use fractal_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_operator_errors_shared_delivery::{Pages, pages, sqlite};


fn report(f: &Fixture, phase: &str) {
    f.raw(|c| eprintln!("operator FULL {phase}: {:?}", pages(c)));
}

#[test]
fn managed_operator_error_real_full_aborts_all_callers_and_writer_recovers() {
    // As in2293, only fresh layout setup may repeat, never semantic assertions.
    for setup in 0..4 {
        let f = Fixture::new(true);
        let mut old_max = None;
        f.raw(|c| {
            c.execute_batch(
                /* sqlite-large-value: allow-for-now lv-ha60d509d545d owner=fractal_delivery::store::terminal_gate_evidence::native_source::transaction_hooks::tests::operator_errors TODO=chunk-or-reference-variable-columns */ "CREATE TABLE test_terminal_operator_probe(payload BLOB NOT NULL); VACUUM;",
            )
            .unwrap();
            let initial = pages(c);
            eprintln!("operator FULL setup={setup} after-table-seed-vacuum: {initial:?}");
            assert_eq!(initial.free, 0, "probe allocation must not use free pages");
            assert!(initial.size > 0 && initial.size <= 65536);
            assert_eq!(initial.journal, "wal");
            old_max = Some(initial.max);
            let actual: i64 = c
                .query_row(
                    /* sqlite-large-value: allow-for-now lv-hf46189c5926f owner=fractal_delivery::store::terminal_gate_evidence::native_source::transaction_hooks::tests::operator_errors TODO=trace-statement-and-value-representation */ &format!("PRAGMA max_page_count={}", initial.count),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(actual, initial.count);
            let capped = pages(c);
            assert_eq!(capped.count, initial.count);
            assert_eq!(capped.max, initial.count);
            assert_eq!(capped.free, 0);
            eprintln!("operator FULL setup={setup} before: {capped:?}");
        });
        let before = f.saved();
        f.reset_counts();
        f.adapter.operator_error.arm_probe(true);
        let ran = Arc::new(AtomicUsize::new(0));
        let writer = f.store.connection.write();
        let a_ran = ran.clone();
        let a = f.enqueue(move |tx| {
            update_exit(tx, 7);
            a_ran.fetch_add(1, Ordering::Relaxed);
            true
        });
        let b_ran = ran.clone();
        let b = f.enqueue(move |tx| {
            update_exit(tx, 8);
            b_ran.fetch_add(1, Ordering::Relaxed);
            true
        });
        drop(writer);
        let outcomes = [answer(a), answer(b)];
        let counts = f.counts();
        eprintln!("operator FULL setup={setup} layout={counts:?} acknowledgements={outcomes:?}");
        report(&f, "after-fault");
        if counts != (1, 1) {
            continue;
        }
        assert_eq!(
            ran.load(Ordering::Relaxed),
            2,
            "native FULL before callback is setup failure"
        );
        let seen = f.adapter.operator_error.observed().unwrap().unwrap();
        assert_eq!(seen.primary, Some(ErrorCode::DiskFull));
        assert_eq!(seen.origin, Origin::Probe);
        assert_eq!(seen.disposition, Disposition::Infrastructure);
        assert!(outcomes.iter().all(Result::is_err));
        assert_eq!(f.saved(), before); // native/capture/root/witness/guard all restored
        assert!(f.adapter.operator_error.take().unwrap().is_none());
        f.raw(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM test_terminal_operator_probe",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let max = old_max.unwrap();
            let restored: i64 = c
                .query_row(/* sqlite-large-value: allow-for-now lv-h28c9f90473a5 owner=fractal_delivery::store::terminal_gate_evidence::native_source::transaction_hooks::tests::operator_errors TODO=trace-statement-and-value-representation */ &format!("PRAGMA max_page_count={max}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(restored, max);
        });
        f.adapter.operator_error.arm_probe(false);
        report(&f, "after-restore");
        // Actual public batched writer and persistent output, after possible automatic rollback.
        f.store.append_claim(&observation(&f, 7)).unwrap();
        assert!(
            f.line()["message"]
                .as_str()
                .unwrap()
                .contains("exit code 7")
        );
        assert_eq!(f.line()["status"], "warn");
        assert!(f.saved().0.generation > before.0.generation);
        assert!(f.adapter.operator_error.observed().unwrap().is_none());
        assert!(!f.read(|tx| f.source().scoped_state(tx).unwrap().3));
        report(&f, "after-clean-commit");
        return;
    }
    panic!("failed to establish one outer batch for FULL control");
}

#[test]
fn managed_operator_error_witness_constraint_fences_without_aborting_admission() {
    let f = Fixture::new(true);
    let before = f.saved();
    f.raw(|c| c.execute_batch(/* sqlite-large-value: allow-for-now lv-hc32465ca1f77 owner=fractal_delivery::store::terminal_gate_evidence::native_source::transaction_hooks::tests::operator_errors TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER test_terminal_operator_constraint BEFORE UPDATE OF body ON main.test_terminal_members WHEN NEW.key='gate/native' BEGIN SELECT RAISE(ABORT,'private witness constraint'); END;").unwrap());
    let native = f.store.append_claim(&observation(&f, 7)).unwrap();
    assert_eq!(f.counts(), (1, 1));
    let seen = f.adapter.operator_error.observed().unwrap().unwrap();
    assert_eq!(seen.primary, Some(ErrorCode::ConstraintViolation));
    assert_eq!(seen.extended, Some(ffi::SQLITE_CONSTRAINT_TRIGGER));
    assert_eq!(seen.origin, Origin::Witness);
    assert_eq!(seen.disposition, Disposition::Fence);
    assert!(f.adapter.operator_error.take().unwrap().is_none());
    f.read(|tx| {
        assert!(
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)",
                [&native.id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert!(f.source().installer.root(tx, VIEW).is_err());
        let body: String = tx
            .query_row(
                "SELECT body FROM test_terminal_members WHERE key='gate/native'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, before.3); // callback DML rolled back, old witness cannot be read as current
    });
    assert_eq!(f.line()["status"], "unknown");
    assert!(f.line()["message"].as_str().unwrap().contains("constraint"));
    f.raw(|c| {
        c.execute_batch(/* sqlite-large-value: allow-for-now lv-hf954e39720ff owner=fractal_delivery::store::terminal_gate_evidence::native_source::transaction_hooks::tests::operator_errors TODO=bound-or-reference-values-and-reclamation */ "DROP TRIGGER test_terminal_operator_constraint")
            .unwrap()
    });
    f.store.append_claim(&observation(&f, 0)).unwrap();
    assert_eq!(f.line()["status"], "unknown"); // sticky gap: no automatic re-attestation
    assert!(f.adapter.operator_error.observed().unwrap().is_none());
}

#[test]
fn managed_operator_error_first_fault_reentry_and_lent_reset_are_private() {
    let fenced = Cell::default();
    assert!(
        fenced
            .preserve(sqlite(ffi::SQLITE_CONSTRAINT_TRIGGER), Origin::Witness)
            .unwrap()
            .is_some()
    );
    let first_reason = fenced.constraint_reason().unwrap().unwrap();
    assert!(
        fenced
            .preserve(anyhow::anyhow!("later logical refusal"), Origin::Witness)
            .unwrap()
            .is_some()
    );
    assert_eq!(fenced.constraint_reason().unwrap().unwrap(), first_reason);
    assert!(fenced.take().unwrap().is_none());
    let cell = Arc::new(Cell::default());
    assert!(
        cell.preserve(sqlite(ffi::SQLITE_FULL), Origin::Probe)
            .unwrap()
            .is_none()
    );
    assert!(
        cell.preserve(sqlite(ffi::SQLITE_BUSY), Origin::Witness)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        cell.observed().unwrap().unwrap().primary,
        Some(ErrorCode::DiskFull)
    );
    assert_eq!(
        cell.take()
            .unwrap()
            .unwrap()
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(ErrorCode::DiskFull)
    );
    let scope = cell.enter().unwrap();
    assert!(cell.enter().is_err());
    assert!(cell.reset().is_err());
    drop(scope);
    assert_eq!(cell.observed().unwrap().unwrap().origin, Origin::Reentry);
    assert!(cell.take().unwrap().is_some());
    cell.reset().unwrap();
    assert!(cell.observed().unwrap().is_none());
    let f = Fixture::new(true);
    let other = Fixture::new(true);
    for rollback in [true, false] {
        let before = f.saved();
        f.reset_counts();
        let mut writer = f.store.connection.write();
        let tx = writer.transaction().unwrap();
        // Synthetic stale callback memory; proves reset when rollback/drop never finalized.
        assert!(
            f.adapter
                .operator_error
                .preserve(sqlite(ffi::SQLITE_FULL), Origin::Witness)
                .unwrap()
                .is_none()
        );
        update_exit(&tx, 7);
        if rollback {
            tx.rollback().unwrap();
        } else {
            drop(tx);
        }
        drop(writer);
        assert_eq!(f.counts(), (1, 0));
        assert_eq!(f.saved(), before);
        assert!(other.adapter.operator_error.observed().unwrap().is_none());
        f.commit(|_| {});
        assert_eq!(f.counts(), (2, 1));
        assert!(f.adapter.operator_error.observed().unwrap().is_none());
        assert!(f.adapter.operator_error.take().unwrap().is_none());
        assert_eq!(f.saved().0, before.0);
    }
}
