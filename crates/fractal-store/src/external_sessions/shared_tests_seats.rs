#![allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
#[cfg(not(target_os = "linux"))]
use chrono::NaiveDateTime;
use chrono::{DateTime, Utc};
use kdl::{KdlDocument, KdlEntry, KdlNode};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use walkdir::WalkDir;
use crate::native_views::{MAX_OMP_EVENT_IDENTITY_BYTES, MAX_TIMELINE_VALUE_BYTES, bounded_value};
use crate::external_sessions::*;

#[cfg(target_os = "linux")]
    pub fn path_executable(name: &str) -> PathBuf {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("the test environment must provide `{name}` on PATH"))
    }

pub fn transcript_session(driver: ExternalDriver, path: &Path) -> ExternalSession {
        ExternalSession {
            id: "session/external-test".into(),
            revision: "revision".into(),
            driver,
            native_id: "native".into(),
            transcript: path.to_owned(),
            codex_home: None,
            cwd: None,
            title: None,
            started_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            process: None,
        }
    }

pub fn texts(timeline: &[Value]) -> Vec<&str> {
        timeline
            .iter()
            .filter(|item| item["type"] == "content")
            .filter_map(|item| item["body"]["text"].as_str())
            .collect()
    }

pub fn assert_unique_ids(timeline: &[Value]) {
        let ids = timeline
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), timeline.len(), "{timeline:#?}");
    }

pub fn claude_line(kind: &str, stamp: &str, text: &str) -> String {
        json!({"type":kind,"timestamp":stamp,"message":{"role":kind,"content":[{"type":"text","text":text}]}})
            .to_string()
    }

// Claude has no omitted record kinds: these fillers remain visible system entries.
    pub const CLAUDE_WINDOW_FILLER: &str = "{\"type\":\"file-history-snapshot\"}\n";

pub fn chat_texts(timeline: &[Value]) -> Vec<&str> {
        timeline
            .iter()
            .filter(|item| item["type"] == "content")
            .filter(|item| matches!(item["role"].as_str(), Some("user" | "assistant")))
            .filter_map(|item| item["body"]["text"].as_str())
            .collect()
    }

/// Independent bounded-reader oracle, retaining the upstream normalization inputs.
    pub fn unfurled_timeline(session: &ExternalSession) -> Vec<Value> {
        let bytes = fs::read(&session.transcript).unwrap();
        let start = bytes.len().saturating_sub(MAX_TIMELINE_BYTES as usize);
        let mut offset = start;
        if start != 0 {
            offset += bytes[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len() - start, |index| index + 1);
        }
        let mut lines = VecDeque::new();
        for line in bytes[offset..].split_inclusive(|byte| *byte == b'\n') {
            if lines.len() == MAX_TIMELINE_LINES {
                lines.pop_front();
            }
            lines.push_back((offset as u64, line));
            offset += line.len();
        }
        let mut items = Vec::new();
        if start != 0 || lines.len() == MAX_TIMELINE_LINES {
            items.push(timeline_item(
                0, &timestamp(session.updated_at_unix_ms), "system", "truncation",
                json!({
                    "reason": "the native transcript prefix is outside the bounded read window; not fetchable through this owner read",
                    "omitted_from_sequence": 0, "omitted_to_sequence": 0
                }),
            ));
        }
        let mut last_timestamp = timestamp(session.started_at_unix_ms);
        let mut next_free_sequence = 0;
        for (line_index, (offset, bytes)) in lines.into_iter().enumerate() {
            let sequence = offset
                .saturating_add(1)
                .saturating_mul(16)
                .max(next_free_sequence);
            let first = items.len();
            normalize_native_bytes(
                session.driver,
                bytes,
                sequence,
                &last_timestamp,
                line_index,
                true,
                &mut items,
            );
            let source = serde_json::to_value(NativeLocator::Jsonl {
                offset,
                length: bytes.len() as u64,
                digest: hex::encode(Sha256::digest(bytes)),
                sequence,
                timestamp: last_timestamp.clone(),
                line_index,
            })
            .unwrap();
            for item in &mut items[first..] {
                item["_source"] = source.clone();
            }
            if let Some(highest) = items[first..]
                .iter()
                .filter_map(|item| item["sequence"].as_u64())
                .max()
            {
                next_free_sequence = highest.saturating_add(1);
            }
            if let Some(stamp) = items[first..]
                .last()
                .and_then(|item| item["timestamp"].as_str())
            {
                last_timestamp = stamp.to_owned();
            }
        }
        items.sort_by_key(|item| item["sequence"].as_u64().unwrap_or(u64::MAX));
        items
    }

pub fn codex_record(id: &str, text: &str) -> String {
        format!(
            "{}\n",
            json!({"type":"response_item","timestamp":"2026-10-07T00:00:00Z",
                "payload":{"type":"message","role":"assistant","id":id,
                    "content":[{"type":"output_text","text":text}]}})
        )
    }
