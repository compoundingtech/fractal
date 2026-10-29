#![allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
use crate::model::{DesiredSubject, MissionInputKind, MissionSpec, NormalizedIntent, St3Error, UsedMissionSpec, WorkSelector};
use crate::graph::parse_intent;
use crate::store::Store;

pub const UNSTORED: &str = "5f0c3a9e1d2b4c6a8e0f1a3b5c7d9e1f2a4b6c8d0e2f4a6b8c0d2e4f6a8b0c2d";

/// A mission whose runs declare `declarations`.
    pub fn watch(declarations: &str) -> String {
        format!(
            r#"version 2
mission "watch" state="ready" {{
  goal "Watch."
  step "watch" {{ agentless }}
{declarations}
}}"#
        )
    }

pub fn schedule(revision: &str) -> String {
        watch(&format!(
            r#"schedule "cycle" {{
  every "6h"
  anchor "2026-01-01T00:00:00Z"
  work {{ mission "cycle@{revision}"; workspace "/tmp/cycles" }}
}}"#
        ))
    }

pub const CYCLE: &str = r#"mission "cycle" state="ready" {
  goal "Complete one cycle."
  step "work" { agentless }
}"#;

pub fn cycle_revision() -> String {
        parse_intent(&format!("version 2\n{CYCLE}"), "node")
            .unwrap()
            .missions["cycle"]
            .revision
            .clone()
    }

pub const OBSERVER: &str = r#"observer "status" {
  resource "resource/status"
  provider "local.file"
  locator "/tmp/st3-reference-status"
  field "status"
}"#;

pub const SUBSCRIPTION: &str = r#"subscription "changed" {
  observer "observer/status"
  on "status"
  to "agent/example/watcher"
  delivery "message"
}"#;

pub const WATCHER: &str = r#"agent "example/watcher" { workspace "/tmp"; command "true" }"#;
