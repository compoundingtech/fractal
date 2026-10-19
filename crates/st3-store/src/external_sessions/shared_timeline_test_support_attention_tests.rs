#![allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::LazyLock;
use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
#[cfg(not(target_os = "linux"))]
use chrono::NaiveDateTime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use crate::native_views::{MAX_OMP_EVENT_IDENTITY_BYTES, MAX_TIMELINE_VALUE_BYTES, bounded_value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// A test's isolation scope: the temporary directory its transcripts live in. Paths
    /// outside the temporary directory share one scope.
    pub fn scope(path: &Path) -> PathBuf {
        let temp = std::env::temp_dir();
        path.strip_prefix(&temp)
            .ok()
            .and_then(|rest| rest.components().next())
            .map_or_else(PathBuf::new, |first| temp.join(first))
    }
