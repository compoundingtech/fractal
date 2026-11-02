//! The API as the daemon serves it: fractal-api-core's routes and the route families of the crates above it
//! (fractal-api-mailbox, fractal-api-client-v0). Every module keeps its `fractal_daemon::api::..` path.

pub use fractal_api_core::api::*;
pub use fractal_api_client_v0::api::client_v0;
pub use fractal_api_mailbox::api::mailbox;
use axum::Router;
mod worker_ingress;
pub use worker_ingress::{worker_authenticated, worker_observer};

/// Every route family the daemon serves beside fractal-api-core's own routes.
pub const ROUTE_FAMILIES: &[RouteFamily] = &[mailbox::routes, client_v0::routes];

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
