#![allow(unused_imports)]
use crate::store::*;

pub const ROOT: &str = "agent/example/root";

pub const PERSON: &str = "person/avery";

pub const SOURCE: &str =
    "version 2\nagent \"example/root\" { accountable \"person/avery\"; harness \"codex\" {}; }";

pub fn intent() -> NormalizedIntent {
    crate::graph::parse_internal_intent(SOURCE, "alder").unwrap()
}
