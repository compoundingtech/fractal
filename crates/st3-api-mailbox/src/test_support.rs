//! Process-local fixture tools: st3-api's, re-exported, and the mailbox protocol fixtures.

pub use st3_api_core::test_support::*;

/// Build an explicit already-admitted protocol fixture. Callers own its temporary Store,
/// socket and synthetic runtime. This adapter preserves kernel peer subject checks and
/// durable Store fences, but cannot qualify native physical ownership or recovery.
#[cfg(feature = "test-support")]
pub fn admitted_mailbox_protocol_router(state: crate::api::AppState) -> axum::Router {
    crate::api::mailbox::admitted_mailbox_protocol_router(state)
}

/// An invented example seat in a caller-owned temporary Store. This supplies only a
/// synthetic transport identity, not a physical native lease or provider authority.
#[cfg(feature = "test-support")]
pub fn synthetic_mailbox_protocol_router(state: crate::api::AppState, subject: &str) -> axum::Router {
    crate::api::mailbox::synthetic_mailbox_protocol_router(state, subject)
}
