#![allow(unused_imports)]
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::Value;
use smallclaims::ivm::install::{Mutation, Namespace, Operator};

pub const INPUT_BYTES: usize = 16 * 1024;

pub const STRUCTURAL_MARKERS: usize = 72;

// These kinds participate in actual-state folding but do not select the runtime observation.
// No wildcard is accepted: a new runtime.action kind requires a source-policy review.
pub const NON_SELECTING: &[&str] = &[
    "runtime.action.requested",
    "runtime.action.succeeded",
    "runtime.action.failed",
    "runtime.action.deadline-reached",
    "runtime.reconcile-decision",
    "runtime.restart-window-reset",
];

// Byte, depth and allocation-marker limits precede serde's allocation. Count only syntax,
// including object/array openings and value separators, outside escaped string contents.
pub fn preflight(raw: &str) -> Result<()> {
    ensure!(raw.len() <= INPUT_BYTES, "terminal input byte cap");
    let (mut quoted, mut escaped, mut depth, mut markers) = (false, false, 0usize, 0usize);
    for byte in raw.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth += 1;
                markers += 1;
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1).context("terminal input syntax")?;
            }
            b',' => markers += 1,
            _ => (),
        }
        ensure!(
            depth <= 8 && markers <= STRUCTURAL_MARKERS,
            "terminal input structure cap"
        );
    }
    ensure!(!quoted && depth == 0, "terminal input syntax");
    Ok(())
}

pub fn id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control),
        "terminal identifier refused"
    );
    Ok(())
}
