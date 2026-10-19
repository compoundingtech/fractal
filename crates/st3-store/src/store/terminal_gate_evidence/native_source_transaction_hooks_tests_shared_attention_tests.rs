#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::{Saved, binding};
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;
use crate::store::{ClaimInput, append_claim_record_tx};
use serde_json::json;
use std::sync::mpsc::Receiver;
use std::time::Duration;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;

use crate::store::terminal_gate_evidence::native_source_shared::*;
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
use crate::store::terminal_gate_evidence_shared_attention_tests::*;
use crate::store::terminal_gate_evidence::*;
use std::collections::BTreeMap;

pub fn update_exit(tx: &rusqlite::Connection, code: i64) {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h8f197e8b6bdb owner=st3_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.exit_code',?1) WHERE subject='exec/one/probe' AND kind='runtime.observed'", [code]).unwrap();
}

pub fn answer(rx: Receiver<Result<(), String>>) -> Result<(), String> {
    rx.recv_timeout(Duration::from_secs(10)).unwrap()
}

pub fn update_exit_raw(c: &rusqlite::Connection, code: i64) {
    c.execute(/* sqlite-large-value: allow-for-now lv-h6eaa34cb9552 owner=st3_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET body=json_set(body,'$.fields.exit_code',?1) WHERE subject='exec/one/probe' AND kind='runtime.observed'",[code]).unwrap();
}
