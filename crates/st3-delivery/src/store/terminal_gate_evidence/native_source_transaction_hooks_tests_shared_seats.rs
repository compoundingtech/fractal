#![allow(unused_imports)]
pub use st3_store::store::terminal_gate_evidence::native_source_transaction_hooks_tests_shared_seats::*;
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


pub struct Fixture {
    pub adapter: Arc<Adapter>,
    pub store: Store,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new(seed: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("claims.sqlite3"), "node").unwrap();
        let adapter = Adapter::install(&store).unwrap();
        adapter.bootstrap(&store, binding(), seed).unwrap();
        let fixture = Self {
            adapter,
            store,
            dir,
        };
        fixture.reset_counts();
        fixture
    }
    pub fn source(&self) -> &NativeSource {
        self.adapter.source.get().unwrap()
    }
    pub fn read<T>(&self, read: impl FnOnce(&Transaction<'_>) -> T) -> T {
        // A pinned read, never a managed writer transaction or a hook invocation.
        let mut c = rusqlite::Connection::open_with_flags(
            self.dir.path().join("claims.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let tx = c.transaction().unwrap();
        read(&tx)
    }
    pub fn line(&self) -> Value {
        self.read(|tx| self.source().doctor_line(tx))
    }
    pub fn saved(&self) -> Saved {
        self.read(|tx| (self.source().installer.root(tx, VIEW).unwrap(), self.source().scoped_state(tx).unwrap(),
            tx.query_row("SELECT body FROM claims WHERE subject='exec/one/probe' AND kind='runtime.observed' ORDER BY store_index DESC LIMIT 1", [], |r|r.get(0)).unwrap(),
            tx.query_row("SELECT body FROM test_terminal_members WHERE key='gate/native'", [], |r|r.get(0)).unwrap()))
    }
    pub fn reset_counts(&self) {
        self.adapter.prepare_calls.store(0, Ordering::Relaxed);
        self.adapter.finalize_calls.store(0, Ordering::Relaxed);
        self.adapter.work.lock().unwrap().clear();
    }
    pub fn counts(&self) -> (usize, usize) {
        (
            self.adapter.prepare_calls.load(Ordering::Relaxed),
            self.adapter.finalize_calls.load(Ordering::Relaxed),
        )
    }
    pub fn raw(&self, job: impl FnOnce(&mut rusqlite::Connection)) {
        let mut writer = self.store.connection.write();
        job(writer.connection.as_mut().unwrap());
    }
    pub fn commit(&self, job: impl FnOnce(&Transaction<'_>)) {
        let mut writer = self.store.connection.write();
        let tx = writer.transaction().unwrap();
        job(&tx);
        tx.commit().unwrap();
    }
    pub fn enqueue(
        &self,
        job: impl FnOnce(&Transaction<'_>) -> bool + Send + 'static,
    ) -> Receiver<Result<(), String>> {
        let (done, answer) = sync_channel(1);
        self.store.connection.send(WriterJob::Batched {
            run: Box::new(job),
            profile: None,
            wait: None,
            holder: smallclaims::windows::holds::Holder::current(),
            enqueued: Instant::now(),
            done,
        });
        answer
    }
}

pub fn observation(f: &Fixture, code: i64) -> ClaimInput {
    let desired: String = f.read(|tx| {
        tx.query_row(
            "SELECT claim_id FROM desired WHERE subject='exec/one/probe'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    });
    ClaimInput {
        subject: "exec/one/probe".into(),
        kind: "runtime.observed".into(),
        actor: None,
        fields: BTreeMap::from([
            ("status".into(), json!("exited")),
            ("exit_code".into(), json!(code)),
            ("incarnation_id".into(), json!(format!("managed-{code}"))),
        ]),
        evidence: vec![desired],
        expected_subject: None,
        idempotency_key: None,
    }
}
