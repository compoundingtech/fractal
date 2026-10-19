#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence_shared::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::terminal_gate_evidence_shared::{INPUT_BYTES, NON_SELECTING, STRUCTURAL_MARKERS, id, preflight};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::Value;
use smallclaims::ivm::install::{Mutation, Namespace, Operator};

pub const MEMBERS: usize = 64;
