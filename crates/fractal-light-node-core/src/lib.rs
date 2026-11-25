//! Graph-free durable light-node authority and authenticated sessions.
pub mod authority;
pub mod config;
pub mod custody;
pub mod journal;
pub mod session;

mod error;
use error::internal;
