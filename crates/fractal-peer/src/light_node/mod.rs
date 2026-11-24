//! Opt-in home/seat execution sessions, separate from full-store replication.
//!
//! Physical custody remains executor-owned. The journal is authoritative local state;
//! a connected session is not evidence of runtime health or fleet freshness.
pub mod journal;
pub mod session;
