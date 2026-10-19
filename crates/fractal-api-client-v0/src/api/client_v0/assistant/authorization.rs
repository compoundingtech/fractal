//! The part of `api::client_v0::assistant::authorization` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use axum::extract::OriginalUri;
use axum::http::HeaderMap;
use smallclaims::hash::canonical_json_text;
use super::reads::{DEFAULT_RESPONSE_BYTES, MAX_RESPONSE_BYTES};
pub use fractal_api_core::api::client_v0::assistant::authorization::*;
pub use fractal_api_core::api::client_v0::assistant::authorization::{ReadReceipt, ReadTrust, ReadTrustRequest, finish, pin, read, route};
#[cfg(test)]
#[path = "authorization_tests.rs"]
mod tests;

