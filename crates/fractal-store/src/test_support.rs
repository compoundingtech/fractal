//! Fixture tools that need the store. The process tools (temporary directories, the fixture shell,
//! command wrappers) live in st3-secrets and are re-exported here.

pub use fractal_secrets::test_support::*;

use std::sync::OnceLock;

/// Only this fixture thread records; production exporter state is not changed.
pub fn roster_export_scope() -> impl Drop {
    crate::otel::fixture_export_scope()
}

#[cfg(feature = "test-support")]
type MailboxTransportControls = std::sync::Mutex<
    std::collections::BTreeMap<(usize, String), tokio::sync::watch::Sender<bool>>,
>;

#[cfg(feature = "test-support")]
fn mailbox_transports() -> &'static MailboxTransportControls {
    static CONTROLS: OnceLock<MailboxTransportControls> = OnceLock::new();
    CONTROLS.get_or_init(Default::default)
}

/// Process-local transport loss in an invented integration fixture. No API route,
/// CLI flag or environment variable can select a production seat or activate it.
#[cfg(feature = "test-support")]
pub fn hold_fixture_mailbox(store: &std::sync::Arc<crate::store::Store>, subject: &str, held: bool) {
    assert!(subject.starts_with("agent/eval."), "mailbox loss controls require an invented fixture seat");
    mailbox_transports().lock().unwrap()
        .entry((std::sync::Arc::as_ptr(store) as usize, subject.into()))
        .or_insert_with(|| tokio::sync::watch::channel(false).0).send_replace(held);
}

#[cfg(feature = "test-support")]
pub fn fixture_mailbox_transport(store: &std::sync::Arc<crate::store::Store>, subject: &str) -> tokio::sync::watch::Receiver<bool> {
    mailbox_transports().lock().unwrap()
        .entry((std::sync::Arc::as_ptr(store) as usize, subject.into()))
        .or_insert_with(|| tokio::sync::watch::channel(false).0).subscribe()
}
