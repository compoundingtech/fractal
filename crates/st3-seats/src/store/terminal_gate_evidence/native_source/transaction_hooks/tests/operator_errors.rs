#![allow(unused_imports)]
use st3_attention::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
use super::*;
use rusqlite::{ErrorCode, ffi};
use st3_attention::store::terminal_gate_evidence::native_source_transaction_hooks_operator_error_shared_seats::*;
use st3_attention::store::terminal_gate_evidence::native_source_transaction_hooks_operator_error_shared_seats::{Cell, DelegatingOperator, Disposition, Observed, Origin, Scope, State, classify};
use st3_attention::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
use st3_attention::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::{Fixture, Saved, binding, observation};


#[test]
fn managed_operator_error_bootstrap_reopen_and_policy_lifetime_are_not_readiness() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
    let adapter = Adapter::install(&store).unwrap();
    let weak = Arc::downgrade(&adapter.operator_error);
    adapter.operator_error.arm_probe(true); // nonexistent probe: unclassified SQL ERROR
    assert!(adapter.bootstrap(&store, binding(), true).is_err());
    assert_eq!(
        adapter
            .operator_error
            .observed()
            .unwrap()
            .unwrap()
            .disposition,
        Disposition::Unclassified
    );
    assert!(store.connection.write().query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='test_native_terminal_guard')", [], |r|r.get::<_,bool>(0)).is_ok_and(|v| !v));
    drop(adapter);
    drop(store);
    assert!(weak.upgrade().is_none());
    let f = Fixture::new(true);
    let path = f.dir.path().join("claims.sqlite3");
    let weak = Arc::downgrade(&f.adapter.operator_error);
    let before = f.saved().0;
    drop(f.adapter);
    drop(f.store);
    assert!(weak.upgrade().is_none());
    let store = Store::open(&path, "node").unwrap();
    let adapter = Adapter::matching_reopen(&store, binding()).unwrap();
    let f = Fixture {
        adapter,
        store,
        dir: f.dir,
    };
    assert!(f.adapter.operator_error.observed().unwrap().is_none());
    let pending = f.read(|tx| f.source().state(tx).unwrap().1);
    f.commit(|_| {});
    if pending {
        assert_eq!(f.line()["status"], "unknown");
    } else {
        assert_eq!(f.saved().0, before);
        assert_eq!(f.line()["status"], "warn");
    }
    assert!(f.adapter.operator_error.observed().unwrap().is_none());
}

#[test]
fn managed_operator_error_old_policy_reopen_refuses_without_apply_or_adoption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let source = NativeSource::attach(&tx, binding()).unwrap();
        crate::store::terminal_gate_evidence::native_source_tests_shared::seed_native(&tx, &source.binding);
        source.finish(&tx).unwrap();
        assert_eq!(source.doctor_line(&tx)["status"], "warn");
        tx.commit().unwrap();
    }
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let adapter = Adapter::matching_reopen(&store, binding()).unwrap();
    let f = Fixture {
        adapter,
        store,
        dir,
    };
    assert_eq!(f.line()["status"], "unknown"); // wrapper policy fingerprint differs
    assert!(f.read(|tx| f.source().installer.root(tx, VIEW).is_err()));
    f.store.append_claim(&observation(&f, 7)).unwrap(); // binding failure fences, not callback storage
    assert_eq!(f.line()["status"], "unknown");
    assert!(f.adapter.operator_error.observed().unwrap().is_none());
    assert!(f.read(|tx| f.source().installer.root(tx, VIEW).is_err()));
}
