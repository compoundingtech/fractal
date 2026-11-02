#![allow(unused_imports)]
use std::path::Path;
use std::sync::OnceLock;
use rusqlite::OptionalExtension as _;
use crate::sekrets::identity::{self, STATEMENT_VERSION, Statement};
use crate::sekrets::protocol::Attestation;
use crate::model::St3Error;
use crate::store::Store;
use crate::sekrets_daemon::*;
use crate::sekrets::store::LogEntry;
use serde_json::json;

pub fn entry(event: &str, caller_person: &str, detail: serde_json::Value) -> LogEntry {
        LogEntry {
            seq: 7,
            at_unix_ms: 1_000,
            caller_person: Some(caller_person.into()),
            owner_person: Some("person/ada".into()),
            event: event.into(),
            actor: Some("agent/fleet/fixture-web/builder".into()),
            profile: Some("ada/agent-gh".into()),
            detail,
        }
    }
