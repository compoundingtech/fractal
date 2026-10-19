#![allow(unused_imports)]
use crate::store::*;
use crate::store::runtime::*;
use std::fs;

pub fn crash_during_schema_install_child() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let path = std::env::var_os("ST3_SCHEMA_CRASH_PATH").expect("private child database path");
        let marker =
            std::env::var_os("ST3_SCHEMA_CRASH_MARKER").expect("private child marker path");
        let phase = std::env::var("ST3_SCHEMA_CRASH_PHASE").expect("private child phase");
        let target = match phase.as_str() {
            "core" => "claims",
            "runtime" => "mission_runs",
            "late-runtime" => "coordination_sends",
            _ => panic!("unknown private crash phase"),
        };
        let connection = Connection::open(path).unwrap();
        let marked_phase = phase.clone();
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::CreateTable { table_name } if table_name == target) {
                std::fs::write(&marker, &marked_phase).unwrap();
                // SAFETY: this private child must exit without SQLite or Rust cleanup.
                unsafe { libc::_exit(86) };
            }
            Authorization::Allow
        }));
        if phase == "core" {
            connection
                .execute_batch(/* sqlite-large-value: allow-for-now lv-h7105126c9419 owner=st3_store::store::runtime_schema_install_tests_shared_attention_tests TODO=trace-statement-and-value-representation */ smallclaims::store::FRESH_SCHEMA)
                .unwrap();
        } else {
            SmalltalkRuntime::default()
                .create_schema_for_fresh_store(&connection)
                .unwrap();
        }
        panic!("the crash point was not reached");
    }
