#![allow(unused_imports)]
use crate::store::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

pub const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

pub const CUT: u128 = 150;

pub const ROUNDS: usize = 4;

pub fn epoch(store: &Store) -> i64 {
    smallclaims::store::checkpoint_capture_epoch(&store.readers.get()).unwrap()
}

pub fn publish_desired(store: &Store, name: &str, version: usize) -> String {
    let source = format!(
        "version 2\nagent \"capture/{name}\" {{ name \"Capture version {version}\"; workspace \"/tmp\"; harness \"claude\" {{ }} }}\n"
    );
    let intent = crate::parse_intent(&source, store.origin()).unwrap();
    assert_eq!(intent.subjects.len(), 1);
    let subject = intent.subjects.keys().next().unwrap().clone();
    let preview = store
        .mission(
            &intent,
            IntentInput {
                kdl: source,
                source_name: None,
            },
        )
        .unwrap();
    store
        .apply(
            &intent,
            &preview.subject_tokens,
            &format!("capture-{name}-desired-{version}"),
        )
        .unwrap();
    subject
}

pub fn publish_document(store: &Store, name: &str, version: usize) -> String {
    let name = format!("doc/capture/{name}");
    store
        .put_document(
            &name,
            format!("Invented capture document version {version}.").as_bytes(),
            &store.latest_document_token(&name).unwrap(),
            &format!("capture-{name}-document-{version}"),
        )
        .unwrap()
        .hash
}

pub fn exchange(source: &Store, target: &Store) -> ReplicationExchange {
    source
        .export_replication_exchange_answering(
            FLEET,
            &target.replication_inventory().unwrap(),
            &target.replication_signature_requests().unwrap(),
        )
        .unwrap()
}

pub fn admit_and_project(source: &Store, target: &Store, exchange: &ReplicationExchange) {
    let receipt = target
        .receive_replication_exchange(source.origin(), FLEET, exchange)
        .unwrap();
    assert_eq!(receipt.received, exchange.envelopes.len());
    assert_eq!(receipt.duplicate, 0);
    let admitted = target.validate_replication_backlog().unwrap();
    assert!(admitted.changed);
    assert!(admitted.valid > 0);
    assert_eq!((admitted.invalid, admitted.unknown, admitted.held), (0, 0, 0));
    assert!(target.project_replication_backlog().unwrap());
}

pub struct PagePause {
    pub envelope_page: AtomicBool,
    pub pauses: AtomicUsize,
    pub reached: mpsc::SyncSender<usize>,
    pub resume: Mutex<mpsc::Receiver<()>>,
}

#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn pause_after_envelope_page(
    event: std::ffi::c_uint,
    context: *mut std::ffi::c_void,
    statement: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    let pause = unsafe { &*context.cast::<PagePause>() };
    let sql = unsafe { rusqlite::ffi::sqlite3_sql(statement.cast()) };
    if sql.is_null() {
        return 0;
    }
    let sql = unsafe { std::ffi::CStr::from_ptr(sql) }.to_bytes();
    const ENVELOPE_PAGE: &[u8] = b"FROM replica_envelopes envelopes";
    if event == rusqlite::ffi::SQLITE_TRACE_STMT as u32
        && sql.windows(ENVELOPE_PAGE.len()).any(|part| part == ENVELOPE_PAGE)
    {
        pause.envelope_page.store(true, Ordering::Release);
    }
    if event == rusqlite::ffi::SQLITE_TRACE_PROFILE as u32
        && sql == b"COMMIT"
        && pause.envelope_page.swap(false, Ordering::AcqRel)
        && let Ok(round) = pause.pauses.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |count| (count < ROUNDS).then_some(count + 1),
        )
        && pause.reached.send(round).is_ok()
        && let Ok(resume) = pause.resume.lock()
    {
        let _ = resume.recv_timeout(Duration::from_secs(30));
    }
    0
}

/// Uninstall even when an assertion unwinds; the callback context outlives this guard.
pub struct TraceGuard<'a>(pub &'a Connection);

impl Drop for TraceGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(self.0.handle(), 0, None, std::ptr::null_mut());
        }
    }
}
