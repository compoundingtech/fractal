#![allow(unused_imports)]
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;
use crate::store::{ClaimInput, append_claim_record_tx};
use serde_json::json;
use smallclaims::ivm::install::Root;
use smallclaims::sqlite::WriterJob;
use std::sync::mpsc::{Receiver, sync_channel};
use std::time::{Duration, Instant};
use crate::store::Store;
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;
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
use smallclaims::ivm::install::Installer;
use std::collections::BTreeMap;
use crate::store::attention_ivm::tests_shared_seats::*;
use crate::store::terminal_gate_evidence::native_source_transaction_hooks_shared_seats::*;

pub type Saved = (Root, (i64, bool, Option<String>, bool), String, String);

pub fn binding() -> Binding {
    Binding {
        origin: "node".into(),
        mission: "one".into(),
        run: "mission-run/one".into(),
        generation: "run-generation/one".into(),
        step: "step-run/one/prepare".into(),
        path: "prepare".into(),
        gate: "prepared".into(),
        exec: "exec/one/probe".into(),
    }
}

