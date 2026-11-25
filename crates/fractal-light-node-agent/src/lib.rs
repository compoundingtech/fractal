//! A physical light-node process: durable session custody and native execution,
//! without the graph store, replication workers or fleet reductions.
#![deny(unsafe_code)]
#[cfg(unix)]
pub mod agent;
pub mod config;
pub mod native;
pub mod performance;
pub mod service;
