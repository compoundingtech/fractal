#![allow(unused_imports)]
use crate::store::terminal_gate_evidence_shared_attention_tests::*;
use crate::store::terminal_gate_evidence::*;
use serde_json::json;
use smallclaims::ivm::install::{Installer, Limits, Outcome, ScanPage};

pub fn encoded(value: Value) -> Value {
    Value::String(value.to_string())
}
