#![allow(unused_imports)]
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_delivery::*;
use rusqlite::{ErrorCode, ffi};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::{Saved, binding};
use crate::store::{ClaimInput, append_claim_record_tx};
use serde_json::json;
use std::sync::mpsc::Receiver;
use std::time::Duration;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::{Adapter, BootstrapToken};
use crate::store::Store;
use smallclaims::sqlite::work::SqliteWorkScope;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::{Binding, FINGERPRINT, GUARD, NativeSource, RAW_BYTES, ROWS, SOURCE_BYTES, bounded_row, decode, point};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_shared::required;
use crate::store::terminal_gate_evidence::*;
use std::collections::BTreeMap;

pub fn sqlite(code: i32) -> anyhow::Error {
    rusqlite::Error::SqliteFailure(
        ffi::Error::new(code),
        Some("private classifier input".into()),
    )
    .into()
}

#[derive(Debug)]
pub struct Pages {
    pub count: i64,
    pub max: i64,
    pub free: i64,
    pub size: i64,
    pub journal: String,
}

pub fn pages(c: &Connection) -> Pages {
    Pages {
        count: c.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap(),
        max: c
            .query_row("PRAGMA max_page_count", [], |r| r.get(0))
            .unwrap(),
        free: c
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap(),
        size: c.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap(),
        journal: c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap(),
    }
}
