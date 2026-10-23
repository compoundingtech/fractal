//! The part of `store::assistant_write` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_store::store::assistant_write::*;
pub use fractal_store::store::assistant_write::{AssistantWriteAuthorization, authorize_grant, refuse_unverified, validate_replicated};

/// The connection must be the writer transaction that commits the protected mutation.
pub fn authorize(connection: &Connection, origin: &str, authorization: &AssistantWriteAuthorization) -> Result<(), St3Error> {
    authorized_grant(connection, origin, authorization).map(|_| ())
}
