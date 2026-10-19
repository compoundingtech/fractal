//! The API as the daemon serves it: st3-api's routes and the route families of the crates above it
//! (st3-api-mailbox). Every module keeps its `st3::api::..` path.

pub use st3_api::api::*;
pub use st3_api_mailbox::api::mailbox;
use axum::Router;

/// Every route family the daemon serves beside st3-api's own routes.
pub const ROUTE_FAMILIES: &[RouteFamily] = &[mailbox::routes];

/// Build routes only. The owner starts background publications with
/// [`start_published_lists`]; a bare router refuses unpublished current runtime reads.
pub fn router(state: AppState) -> Router {
    router_with(state, ROUTE_FAMILIES)
}

/// Build the loopback-only client gateway. Unlike the local Unix boundary, every ordinary
/// client request on this router requires a paired bearer credential.
pub fn fabric_router(state: AppState) -> Router {
    fabric_router_with(state, ROUTE_FAMILIES)
}
