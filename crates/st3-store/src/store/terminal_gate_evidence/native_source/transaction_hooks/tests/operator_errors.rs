//! These controls are absent from non-test builds and never open a production Store.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_operator_errors_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_operator_errors_shared_delivery::{Pages, pages, sqlite};
use super::super::operator_error::{Cell, Disposition, Origin, classify};
use rusqlite::ffi;

#[test]
fn managed_operator_error_typed_primary_extended_and_unknown_policy() {
    // Synthetic code classification, not an assertion that each I/O failure occurred.
    for code in [
        ffi::SQLITE_BUSY,
        ffi::SQLITE_LOCKED,
        ffi::SQLITE_NOMEM,
        ffi::SQLITE_READONLY,
        ffi::SQLITE_INTERRUPT,
        ffi::SQLITE_IOERR,
        ffi::SQLITE_CORRUPT,
        ffi::SQLITE_FULL,
        ffi::SQLITE_CANTOPEN,
        ffi::SQLITE_NOTADB,
        ffi::SQLITE_IOERR_READ,
        ffi::SQLITE_BUSY_SNAPSHOT,
        ffi::SQLITE_READONLY_DBMOVED,
    ] {
        let original = sqlite(code).context("original operator context");
        assert_eq!(classify(&original), Disposition::Infrastructure);
        let cell = Cell::default();
        assert!(cell.preserve(original, Origin::Witness).unwrap().is_none());
        let retained = cell.take().unwrap().unwrap();
        assert_eq!(
            retained
                .downcast_ref::<rusqlite::Error>()
                .unwrap()
                .sqlite_error()
                .unwrap()
                .extended_code,
            code
        );
        assert!(format!("{retained:#}").contains("original operator context"));
        assert!(cell.take().unwrap().is_none());
    }
    for code in [
        ffi::SQLITE_CONSTRAINT,
        ffi::SQLITE_CONSTRAINT_TRIGGER,
        ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
        ffi::SQLITE_CONSTRAINT_UNIQUE,
        ffi::SQLITE_CONSTRAINT_CHECK,
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY,
        ffi::SQLITE_CONSTRAINT_NOTNULL,
    ] {
        let cell = Cell::default();
        assert_eq!(classify(&sqlite(code)), Disposition::Fence);
        let original = cell
            .preserve(sqlite(code), Origin::Witness)
            .unwrap()
            .unwrap();
        assert_eq!(
            original
                .downcast_ref::<rusqlite::Error>()
                .unwrap()
                .sqlite_error()
                .unwrap()
                .extended_code,
            code
        );
        assert!(cell.take().unwrap().is_none());
        assert!(cell.constraint_reason().unwrap().unwrap().len() <= 96);
    }
    for original in [
        sqlite(ffi::SQLITE_ERROR),
        sqlite(ffi::SQLITE_SCHEMA),
        sqlite(ffi::SQLITE_MISUSE),
        sqlite(255),
        rusqlite::Error::InvalidQuery.into(),
    ] {
        assert_eq!(classify(&original), Disposition::Unclassified);
        let cell = Cell::default();
        assert!(cell.preserve(original, Origin::Witness).unwrap().is_none());
        assert!(cell.take().unwrap().unwrap().is::<rusqlite::Error>());
    }
    assert_eq!(
        classify(&anyhow::anyhow!("explicit logical completeness refusal")),
        Disposition::Fence
    );
}

