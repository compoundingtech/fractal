#![allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
use anyhow::Result;
use rusqlite::{Connection, Statement, StatementStatus, params};
use serde_json::Value;
use crate::store::card_fold::health_capture::*;
use serde_json::json;

pub fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hb48646e7123d owner=st3_store::store::card_fold::health_capture_tests_shared_attention_tests TODO=chunk-or-reference-variable-columns */ "CREATE TABLE claims(id TEXT NOT NULL UNIQUE,subject TEXT,kind TEXT,origin TEXT,
                actor TEXT,store_index INTEGER PRIMARY KEY,accepted_at_unix_ms TEXT,body TEXT);
             CREATE INDEX claims_subject_kind_index ON claims(subject,kind,store_index);
             CREATE INDEX claims_message_to_order_index ON claims(json_extract(body,'$.fields.to'),store_index)
                WHERE kind='message.sent';",
        ).unwrap();
        insert(
            &connection,
            "runtime-a",
            "agent/a",
            "runtime.observed",
            1,
            100,
            json!({
                "status":"running", "incarnation_id":"incarnation-a"
            }),
        );
        connection
    }

pub fn insert(
        connection: &Connection,
        id: &str,
        subject: &str,
        kind: &str,
        index: u64,
        at: u64,
        fields: Value,
    ) {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-hdd1510c56a11 owner=st3_store::store::card_fold::health_capture_tests_shared_attention_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims VALUES(?1,?2,?3,'host/a','agent/sender',?4,?5,?6)",
                params![
                    id,
                    subject,
                    kind,
                    index,
                    at.to_string(),
                    json!({"fields":fields}).to_string()
                ],
            )
            .unwrap();
    }
