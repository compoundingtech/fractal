//! The part of `external_sessions` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::LazyLock;
use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
#[cfg(not(target_os = "linux"))]
use chrono::NaiveDateTime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
#[cfg(test)]
use crate::native_views::{MAX_OMP_EVENT_IDENTITY_BYTES, MAX_TIMELINE_VALUE_BYTES, bounded_value};
pub use st3_store::external_sessions::*;
pub use st3_store::external_sessions::{ExternalDriver, ExternalProcess, ExternalSession, FoldedLine, LineTimelineFold, MAX_TIMELINE_BYTES, MAX_TIMELINE_LINES, NativeLocator, REASONING_BLOCKS, TimelineKey, TimelineOrder, TimelineSlice};
pub use st3_store::external_sessions::{collect_timeline_slice, compare_timeline_positions, digest, file_identity, native_timeline_position, normalize_native_bytes, normalize_native_line, normalize_omp, normalized_role, omission_reason, push_content, push_image};
pub use st3_store::external_sessions::{push_message, push_native_block, push_reasoning, push_tool_call, push_tool_result_with_status, push_unrecognized, raw_bytes_block, timeline_item, timeline_key, timestamp};

#[cfg(any(test, feature = "test-support"))]
pub mod timeline_test_support {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, LazyLock, Mutex};
    pub use st3_store::external_sessions::timeline_test_support::*;

    /// The instance of a process-wide budget or cache for `path`'s scope, so each test owns
    /// its own and parallel tests cannot affect each other through global state.
    pub fn scoped<T>(scopes: &Scopes<T>, path: &Path, create: impl FnOnce() -> T) -> Arc<T> {
        let mut scopes = scopes.lock().unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            scopes
                .entry(scope(path))
                .or_insert_with(|| Arc::new(create())),
        )
    }
}
