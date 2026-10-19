//! Process-local fixture tools: st3-api's and st3-api-mailbox's, with the mailbox protocol routers
//! over the router the daemon serves.

pub use st3_api_mailbox::test_support::*;

use crate::api::{AppState, ROUTE_FAMILIES};

/// Build an explicit already-admitted protocol fixture. Callers own its temporary Store,
/// socket and synthetic runtime. This adapter preserves kernel peer subject checks and
/// durable Store fences, but cannot qualify native physical ownership or recovery.
pub fn admitted_mailbox_protocol_router(state: AppState) -> axum::Router {
    crate::api::mailbox::admitted_mailbox_protocol_router_with(state, ROUTE_FAMILIES)
}

/// An invented example seat in a caller-owned temporary Store. This supplies only a
/// synthetic transport identity, not a physical native lease or provider authority.
pub fn synthetic_mailbox_protocol_router(state: AppState, subject: &str) -> axum::Router {
    crate::api::mailbox::synthetic_mailbox_protocol_router_with(state, subject, ROUTE_FAMILIES)
}
