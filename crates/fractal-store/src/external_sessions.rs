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


pub const MAX_TIMELINE_LINES: usize = 4_096;
pub const MAX_TIMELINE_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(test)]
use crate::native_views::{MAX_OMP_EVENT_IDENTITY_BYTES, MAX_TIMELINE_VALUE_BYTES, bounded_value};
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalDriver {
    Codex,
    Claude,
    Pi,
    Omp,
    OpenCode,
}

impl ExternalDriver {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExternalProcess {
    pub pid: u32,
    pub parent_pid: u32,
    pub started_at_unix_ms: u128,
    pub fingerprint: String,
    pub cwd: Option<PathBuf>,
    pub command: String,
    pub exact_session: bool,
}

#[derive(Clone, Debug)]
pub struct ExternalSession {
    pub id: String,
    pub revision: String,
    pub driver: ExternalDriver,
    pub native_id: String,
    pub transcript: PathBuf,
    /// The provider home owning this rollout, independent of the daemon launch environment.
    pub codex_home: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub started_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub process: Option<ExternalProcess>,
}

pub fn timestamp(unix_ms: u128) -> String {
    let millis = i64::try_from(unix_ms).unwrap_or(i64::MAX);
    DateTime::<Utc>::from_timestamp_millis(millis)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Normalize a native transcript into timeline entries.
///
/// Volatile entry locator; removed before a timeline leaves the owner. It contains no
/// conversation bytes and is authenticated inside the owner content reference.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "format", rename_all = "kebab-case")]
pub enum NativeLocator {
    Jsonl {
        offset: u64,
        length: u64,
        digest: String,
        sequence: u64,
        timestamp: String,
        line_index: usize,
    },
    Sqlite {
        part_row: i64,
        message_row: i64,
        digest: String,
        message_digest: String,
        sequence: u64,
        timestamp: String,
        role: String,
    },
    SqliteMessage {
        message_row: i64,
        digest: String,
        sequence: u64,
        timestamp: String,
    },
}

/// Harnesses change their files between releases, crash mid-write, and occasionally tear a
/// record. The reader is liberal in what it accepts: one unreadable unit (a line that is not
/// UTF-8, is not JSON, or carries a kind this reader does not know) costs only that unit, never
/// the rest of the transcript. What it emits stays conservative: an unreadable line or an
/// unrecognized record becomes a clearly-labelled `system` entry, never an entry attributed to
/// the user or the agent. Only opening the file can fail the whole read.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineOrder {
    Sequence,
    TimestampSequence,
}

/// Native entries have rank 0; Small Talk messages have rank 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineKey {
    pub timestamp: String,
    pub sequence: u64,
    pub rank: u8,
    pub id: String,
}

impl TimelineKey {
    pub fn cmp_in(&self, other: &Self, order: TimelineOrder) -> std::cmp::Ordering {
        compare_timeline_positions(
            (&self.timestamp, self.sequence, self.rank, &self.id),
            (&other.timestamp, other.sequence, other.rank, &other.id),
            order,
        )
    }
}

pub fn compare_timeline_positions(
    left: (&str, u64, u8, &str),
    right: (&str, u64, u8, &str),
    order: TimelineOrder,
) -> std::cmp::Ordering {
    match order {
        TimelineOrder::Sequence => (left.1, left.2, left.3).cmp(&(right.1, right.2, right.3)),
        TimelineOrder::TimestampSequence => left.cmp(&right),
    }
}

pub fn native_timeline_position(item: &Value) -> (&str, u64, u8, &str) {
    (
        item["timestamp"].as_str().unwrap_or_default(),
        item["sequence"].as_u64().unwrap_or_default(),
        0,
        item["id"].as_str().unwrap_or_default(),
    )
}

pub fn timeline_key(item: &Value, rank: u8) -> TimelineKey {
    TimelineKey {
        timestamp: item["timestamp"].as_str().unwrap_or_default().to_owned(),
        sequence: item["sequence"].as_u64().unwrap_or_default(),
        rank,
        id: item["id"].as_str().unwrap_or_default().to_owned(),
    }
}

pub struct TimelineSlice {
    /// Newest-first entries strictly before the supplied key.
    pub items: Vec<Value>,
    pub has_more: bool,
    /// Source content generation: stable across appends, moved by every non-append rebuild.
    pub generation: u64,
}

pub fn collect_timeline_slice<'a>(
    committed: impl Iterator<Item = &'a Value>,
    transient: impl Iterator<Item = &'a Value>,
    order: TimelineOrder,
    before: Option<&TimelineKey>,
    limit: usize,
    clone_output: impl Fn(&Value) -> Value,
) -> TimelineSlice {
    let is_before = |item: &&Value| {
        before.is_none_or(|before| {
            compare_timeline_positions(
                native_timeline_position(item),
                (&before.timestamp, before.sequence, before.rank, &before.id),
                order,
            )
            .is_lt()
        })
    };
    let mut committed = committed.filter(is_before).peekable();
    let mut transient = transient.filter(is_before).peekable();
    let mut items = Vec::new();
    while items.len() < limit {
        let item = match (committed.peek(), transient.peek()) {
            (Some(item), Some(tail))
                if compare_timeline_positions(
                    native_timeline_position(item),
                    native_timeline_position(tail),
                    order,
                )
                .is_lt() =>
            {
                transient.next()
            }
            (Some(_), _) => committed.next(),
            (None, Some(_)) => transient.next(),
            (None, None) => break,
        };
        if let Some(item) = item {
            items.push(clone_output(item));
        }
    }
    TimelineSlice {
        items,
        has_more: committed.peek().is_some() || transient.peek().is_some(),
        generation: 0,
    }
}

/// Process-random epoch for timeline generations. A restart draws a fresh epoch, so page
/// cursors bound to a generation never carry into another process, matching the cursor MAC
/// key. Within one process, rebuild serials keep generations strictly increasing.
static TIMELINE_GENERATION_EPOCH: LazyLock<u64> = LazyLock::new(|| {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).expect("timeline generation epoch entropy");
    // Clear the top bit so adding rebuild serials cannot wrap in any relevant lifetime.
    u64::from_be_bytes(bytes) & (u64::MAX >> 1)
});
static TIMELINE_REBUILDS: AtomicU64 = AtomicU64::new(0);
fn next_timeline_generation() -> u64 {
    TIMELINE_GENERATION_EPOCH.wrapping_add(
        TIMELINE_REBUILDS
            .fetch_add(1, AtomicOrdering::Relaxed)
            .saturating_add(1),
    )
}

/// Test-only determinism hooks: park one transcript refresh mid-read and observe readers
/// queued on a contended fold. Nothing here is compiled into release builds.
#[cfg(any(test, feature = "test-support"))]
pub mod timeline_test_support {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, LazyLock, Mutex};

    /// Per-scope instances of a process-wide budget or cache.
    pub type Scopes<T> = LazyLock<Mutex<HashMap<PathBuf, Arc<T>>>>;

    /// A test's isolation scope: the temporary directory its transcripts live in. Paths
    /// outside the temporary directory share one scope.
    fn scope(path: &Path) -> PathBuf {
        let temp = std::env::temp_dir();
        path.strip_prefix(&temp)
            .ok()
            .and_then(|rest| rest.components().next())
            .map_or_else(PathBuf::new, |first| temp.join(first))
    }

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

    static FOLD_WAITERS: AtomicUsize = AtomicUsize::new(0);
    static REFRESH_GATES: LazyLock<Mutex<HashMap<PathBuf, RefreshGate>>> =
        LazyLock::new(Mutex::default);
    static REFRESH_ARRIVED: Condvar = Condvar::new();

    /// One gated transcript refresh: arrived once its reader parked, released to let it run.
    struct RefreshGate {
        arrived: bool,
        released: bool,
    }

    /// Test hooks never propagate lock poison: a gate recovering its current state is always
    /// sound, matching the fold's own poison recovery.
    fn gates() -> std::sync::MutexGuard<'static, HashMap<PathBuf, RefreshGate>> {
        REFRESH_GATES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Park the next refresh of `path` until `release_refresh(path)`, so a test controls
    /// exactly when the fold's I/O completes while holding the fold. Gates are per path, so
    /// parallel tests never release each other's readers.
    pub fn arm_refresh(path: &Path) {
        let mut gates = gates();
        gates.insert(
            path.to_owned(),
            RefreshGate {
                arrived: false,
                released: false,
            },
        );
        REFRESH_ARRIVED.notify_all();
    }

    pub fn wait_refresh_arrived(path: &Path) {
        let mut gates = gates();
        while !gates.get(path).is_some_and(|gate| gate.arrived) {
            gates = REFRESH_ARRIVED
                .wait(gates)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    pub fn release_refresh(path: &Path) {
        let mut gates = gates();
        if let Some(gate) = gates.get_mut(path) {
            gate.released = true;
        }
        REFRESH_ARRIVED.notify_all();
    }

    pub fn gate_refresh(path: &Path) {
        let mut gates = gates();
        loop {
            let Some(gate) = gates.get_mut(path) else {
                return;
            };
            if gate.released {
                return;
            }
            if !gate.arrived {
                gate.arrived = true;
                REFRESH_ARRIVED.notify_all();
                continue;
            }
            gates = REFRESH_ARRIVED
                .wait(gates)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Block until `count` readers are parked on a contended fold. They are guaranteed to
    /// arrive: the gated refresh holds that fold for as long as the test wants.
    pub fn wait_fold_waiters(count: usize) {
        while FOLD_WAITERS.load(Ordering::SeqCst) < count {
            std::thread::yield_now();
        }
    }

    pub fn enter_contended() {
        FOLD_WAITERS.fetch_add(1, Ordering::SeqCst);
    }

    pub fn leave_contended() {
        FOLD_WAITERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Volatile bounded fold of an append-only JSONL transcript. Replacement, shrinkage and a
/// changed last complete record rebuild it and draw a new generation, expiring page cursors
/// from the replaced content; pure appends keep the generation. Earlier edits are fenced by
/// normalized_record on pinned content reads, rather than by scanning the entire transcript
/// on every page.
#[derive(Default)]
pub struct LineTimelineFold {
    identity: Option<FileIdentity>,
    last_record: Option<(u64, usize, [u8; 32])>,
    generation: u64,
    consumed: u64,
    observed_len: u64,
    lines: VecDeque<FoldedLine>,
    entries: BTreeMap<(String, u64, u8, String), (usize, usize)>,
    sequence_entries: BTreeMap<(u64, u8, String), (usize, usize)>,
    emitting_lines: BTreeSet<usize>,
    next_line_number: usize,
    seed_timestamp: String,
    pub retained_source_bytes: usize,
}

pub struct FoldedLine {
    start: u64,
    /// Stable within this fold; translated to a window-relative index only on output.
    number: usize,
    bytes: Vec<u8>,
    digest: [u8; 32],
    locator: Option<NativeLocator>,
    next_free_sequence: u64,
    last_timestamp: String,
    items: Vec<Value>,
}

impl FoldedLine {
    fn new(start: u64, number: usize, bytes: Vec<u8>) -> Self {
        Self {
            start,
            number,
            digest: Sha256::digest(&bytes).into(),
            bytes,
            locator: None,
            next_free_sequence: 0,
            last_timestamp: String::new(),
            items: Vec::new(),
        }
    }

    fn normalize(
        &mut self,
        driver: ExternalDriver,
        sequence: u64,
        timestamp: &str,
        line_number: usize,
    ) -> Result<()> {
        // Cached locators and diagnostics carry a stable fold ordinal, not an index
        // that needs rewriting whenever the bounded window slides.
        self.items.clear();
        normalize_native_bytes(
            driver,
            &self.bytes,
            sequence,
            timestamp,
            line_number,
            true,
            &mut self.items,
        );
        let locator = NativeLocator::Jsonl {
            offset: self.start,
            length: self.bytes.len() as u64,
            digest: hex::encode(self.digest),
            sequence,
            timestamp: timestamp.to_owned(),
            line_index: line_number,
        };
        let source = serde_json::to_value(&locator)?;
        for item in &mut self.items {
            item["_source"] = source.clone();
        }
        self.locator = Some(locator);
        self.next_free_sequence = self
            .items
            .iter()
            .filter_map(|item| item["sequence"].as_u64())
            .max()
            .map_or(sequence, |highest| highest.saturating_add(1));
        self.last_timestamp = self
            .items
            .last()
            .and_then(|item| item["timestamp"].as_str())
            .unwrap_or(timestamp)
            .to_owned();
        Ok(())
    }
}

impl LineTimelineFold {
    pub fn read(&mut self, session: &ExternalSession) -> Result<Vec<Value>> {
        let mut transient = self.refresh(session)?;
        transient.sort_by_key(|item| item["sequence"].as_u64().unwrap_or(u64::MAX));
        let mut transient = transient.iter().peekable();
        let mut committed = self
            .sequence_entries
            .values()
            .map(|&position| self.indexed_item(position))
            .peekable();
        let mut items = Vec::new();
        // Native sequences are monotonic across records. Merge the small transient
        // set instead of sorting the complete returned window.
        loop {
            let item = match (committed.peek(), transient.peek()) {
                (Some(item), Some(tail))
                    if tail["sequence"].as_u64().unwrap_or(u64::MAX)
                        <= item["sequence"].as_u64().unwrap_or(u64::MAX) =>
                {
                    transient.next()
                }
                (Some(_), _) => committed.next(),
                (None, Some(_)) => transient.next(),
                (None, None) => break,
            };
            if let Some(item) = item {
                items.push(self.clone_output(item));
            }
        }
        bind_native_delivery_thread(session, &mut items);
        Ok(items)
    }

    pub fn slice(
        &mut self,
        session: &ExternalSession,
        order: TimelineOrder,
        before: Option<&TimelineKey>,
        limit: usize,
    ) -> Result<TimelineSlice> {
        use std::ops::Bound::{Excluded, Unbounded};
        let mut transient = self.refresh(session)?;
        transient.sort_by(|a, b| {
            compare_timeline_positions(
                native_timeline_position(a),
                native_timeline_position(b),
                order,
            )
        });
        let mut slice = match order {
            TimelineOrder::Sequence => {
                let upper = before.map(|key| (key.sequence, key.rank, key.id.clone()));
                let committed = self
                    .sequence_entries
                    .range::<(u64, u8, String), _>((
                        Unbounded,
                        upper.as_ref().map_or(Unbounded, Excluded),
                    ))
                    .rev()
                    .map(|(_, &position)| self.indexed_item(position));
                collect_timeline_slice(
                    committed,
                    transient.iter().rev(),
                    order,
                    before,
                    limit,
                    |item| self.clone_output(item),
                )
            }
            TimelineOrder::TimestampSequence => {
                let upper = before.map(|key| {
                    (
                        key.timestamp.clone(),
                        key.sequence,
                        key.rank,
                        key.id.clone(),
                    )
                });
                let committed = self
                    .entries
                    .range::<(String, u64, u8, String), _>((
                        Unbounded,
                        upper.as_ref().map_or(Unbounded, Excluded),
                    ))
                    .rev()
                    .map(|(_, &position)| self.indexed_item(position));
                collect_timeline_slice(
                    committed,
                    transient.iter().rev(),
                    order,
                    before,
                    limit,
                    |item| self.clone_output(item),
                )
            }
        };
        bind_native_delivery_thread(session, &mut slice.items);
        slice.generation = self.generation;
        Ok(slice)
    }

    fn indexed_item(&self, (number, index): (usize, usize)) -> &Value {
        let first = self
            .lines
            .front()
            .expect("indexed entry has a retained line")
            .number;
        &self.lines[number - first].items[index]
    }

    fn clone_output(&self, item: &Value) -> Value {
        let mut item = item.clone();
        if let Some(number) = item["_source"]["line_index"].as_u64() {
            let first = self
                .lines
                .front()
                .map_or(self.next_line_number, |line| line.number);
            let line_index = number - first as u64;
            item["_source"]["line_index"] = json!(line_index);
            if item["body"]["code"] == "native-line-unreadable" {
                item["body"]["details"]["line_in_window"] = json!(line_index.saturating_add(1));
            }
        }
        item
    }

    fn seed_before(&self, number: usize, fallback: &str) -> (u64, String) {
        self.emitting_lines.range(..number).next_back().map_or_else(
            || (0, fallback.to_owned()),
            |number| {
                let first = self
                    .lines
                    .front()
                    .expect("emitting line is retained")
                    .number;
                let line = &self.lines[*number - first];
                (line.next_free_sequence, line.last_timestamp.clone())
            },
        )
    }

    fn remove_entries(&mut self, items: &[Value]) {
        for item in items {
            let key = timeline_key(item, 0);
            self.sequence_entries
                .remove(&(key.sequence, key.rank, key.id.clone()));
            self.entries
                .remove(&(key.timestamp, key.sequence, key.rank, key.id));
        }
    }

    fn pop_front(&mut self) {
        if let Some(line) = self.lines.pop_front() {
            self.retained_source_bytes -= line.bytes.len();
            self.emitting_lines.remove(&line.number);
            self.remove_entries(&line.items);
        }
    }

    fn refresh(&mut self, session: &ExternalSession) -> Result<Vec<Value>> {
        let mut file = File::open(&session.transcript)
            .with_context(|| format!("read transcript {}", session.transcript.display()))?;
        let metadata = file.metadata()?;
        let len = metadata.len();
        let window_start = len.saturating_sub(MAX_TIMELINE_BYTES);
        let identity = file_identity(&metadata);
        if !self.resumable(&identity, len, window_start, &mut file) {
            // Any non-append change (replacement, shrink, edit, or a cold fold, including one
            // the fold cache evicted) draws a new generation, so outstanding page cursors are
            // refused as expired instead of applying their old boundary to a re-read fold.
            *self = Self {
                identity: Some(identity),
                consumed: window_start,
                generation: next_timeline_generation(),
                ..Self::default()
            };
        }
        let first_new_number = self.next_line_number;
        let previous_first = self.lines.front().map(|line| line.number);
        file.seek(SeekFrom::Start(self.consumed))?;
        // Appends beyond this high-water mark belong to the next refresh.
        let mut reader = BufReader::new(file.take(len - self.consumed));
        let mut read_error = None;
        if self.consumed == window_start && window_start != 0 && self.lines.is_empty() {
            let mut partial = Vec::new();
            match reader.read_until(b'\n', &mut partial) {
                Ok(count) => self.consumed += count as u64,
                Err(error) => read_error = Some(error),
            }
        }
        let mut tail = None;
        while read_error.is_none() {
            let mut buffer = Vec::new();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) => break,
                Ok(_) if buffer.last() == Some(&b'\n') => {
                    #[cfg(any(test, feature = "test-support"))]
                    timeline_test_support::gate_refresh(session.transcript.as_path());
                    let line = FoldedLine::new(self.consumed, self.next_line_number, buffer);
                    self.next_line_number += 1;
                    self.consumed += line.bytes.len() as u64;
                    self.last_record = Some((line.start, line.bytes.len(), line.digest));
                    self.retained_source_bytes += line.bytes.len();
                    self.lines.push_back(line);
                    while self.lines.len() > MAX_TIMELINE_LINES {
                        self.pop_front();
                    }
                }
                Ok(_) => {
                    tail = Some(FoldedLine::new(
                        self.consumed,
                        self.next_line_number,
                        buffer,
                    ));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => read_error = Some(error),
            }
        }
        while window_start != 0
            && self
                .lines
                .front()
                .is_some_and(|line| line.start <= window_start)
        {
            self.pop_front();
        }
        let output_cap = MAX_TIMELINE_LINES - usize::from(tail.is_some());
        while self.lines.len() > output_cap {
            self.pop_front();
        }
        let mut transient = Vec::new();
        if window_start != 0 || self.lines.len() + usize::from(tail.is_some()) == MAX_TIMELINE_LINES
        {
            transient.push(timeline_item(
                0, &timestamp(session.updated_at_unix_ms), "system", "truncation",
                json!({
                    "reason": "the native transcript prefix is outside the bounded read window; not fetchable through this owner read",
                    "omitted_from_sequence": 0,
                    "omitted_to_sequence": 0
                }),
            ));
        }
        let seed_timestamp = timestamp(session.started_at_unix_ms);
        let first = self.lines.front().map(|line| line.number);
        let new_index = first.map_or(0, |number| {
            first_new_number
                .saturating_sub(number)
                .min(self.lines.len())
        });
        // Append-only reads begin at the newly added suffix. Eviction or a changed
        // session seed can change inherited timestamps and sequence collisions, so
        // replay only the affected prefix until its cached inputs converge.
        let mut index = if first != previous_first || seed_timestamp != self.seed_timestamp {
            0
        } else {
            new_index
        };
        let (mut next_free_sequence, mut last_timestamp) = if index == 0 {
            (0, seed_timestamp.clone())
        } else {
            self.seed_before(first_new_number, &seed_timestamp)
        };
        while index < self.lines.len() {
            if index < new_index {
                // Omitted records carry neither an output nor a seed transition.
                // Do not replay long runs of them when the initial seed changes.
                index = self
                    .emitting_lines
                    .range(self.lines[index].number..first_new_number)
                    .next()
                    .map_or(new_index, |number| {
                        *number - first.expect("replay has a retained line")
                    });
                if index == self.lines.len() {
                    break;
                }
            }
            let line = &mut self.lines[index];
            let sequence = line
                .start
                .saturating_add(1)
                .saturating_mul(16)
                .max(next_free_sequence);
            let unchanged = matches!(&line.locator, Some(NativeLocator::Jsonl {
                sequence: old_sequence, timestamp: old_timestamp, ..
            }) if *old_sequence == sequence && old_timestamp == &last_timestamp);
            if unchanged && index < new_index {
                // The remaining old suffix has identical inputs and outputs; jump
                // directly to new records, including over runs of omitted records.
                index = new_index;
                (next_free_sequence, last_timestamp) =
                    self.seed_before(first_new_number, &seed_timestamp);
                continue;
            }
            for item in &line.items {
                let key = timeline_key(item, 0);
                self.sequence_entries
                    .remove(&(key.sequence, key.rank, key.id.clone()));
                self.entries
                    .remove(&(key.timestamp, key.sequence, key.rank, key.id));
            }
            line.normalize(session.driver, sequence, &last_timestamp, line.number)?;
            if line.items.is_empty() {
                // An omitted record must not advance the oracle's sequence seed.
                line.next_free_sequence = next_free_sequence;
            }
            if !line.items.is_empty() {
                self.emitting_lines.insert(line.number);
            }
            for (item_index, item) in line.items.iter().enumerate() {
                let key = timeline_key(item, 0);
                self.sequence_entries.insert(
                    (key.sequence, key.rank, key.id.clone()),
                    (line.number, item_index),
                );
                self.entries.insert(
                    (key.timestamp, key.sequence, key.rank, key.id),
                    (line.number, item_index),
                );
            }
            next_free_sequence = line.next_free_sequence;
            last_timestamp.clone_from(&line.last_timestamp);
            index += 1;
        }
        self.seed_timestamp = seed_timestamp;
        if let Some(mut tail) = tail {
            let sequence = tail
                .start
                .saturating_add(1)
                .saturating_mul(16)
                .max(next_free_sequence);
            tail.normalize(session.driver, sequence, &last_timestamp, tail.number)?;
            if !tail.items.is_empty() {
                next_free_sequence = tail.next_free_sequence;
            }
            last_timestamp = tail.last_timestamp;
            transient.extend(tail.items);
        }
        if let Some(error) = read_error {
            transient.push(timeline_item(
                next_free_sequence.max(16), &last_timestamp, "system", "error",
                json!({
                    "code": "native-transcript-read-failed",
                    "message": format!("st stopped reading the {} transcript early: {error}", session.driver.as_str()),
                    "retryable": true,
                    "details": {}
                }),
            ));
        }
        self.observed_len = len;
        Ok(transient)
    }

    fn resumable(
        &self,
        identity: &FileIdentity,
        len: u64,
        window_start: u64,
        file: &mut File,
    ) -> bool {
        if self.identity.as_ref() != Some(identity)
            || len < self.consumed
            || len < self.observed_len
            || self.consumed <= window_start
        {
            return false;
        }
        let Some((start, length, digest)) = self.last_record else {
            return self.consumed == 0;
        };
        let mut record = vec![0; length];
        file.seek(SeekFrom::Start(start)).is_ok()
            && file.read_exact(&mut record).is_ok()
            && <[u8; 32]>::from(Sha256::digest(&record)) == digest
    }
}

#[cfg(unix)]
pub type FileIdentity = (u64, u64);
#[cfg(not(unix))]
pub type FileIdentity = Option<SystemTime>;

pub fn file_identity(metadata: &fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        (metadata.dev(), metadata.ino())
    }
    #[cfg(not(unix))]
    {
        metadata.created().ok()
    }

}

fn bind_native_delivery_thread(session: &ExternalSession, items: &mut [Value]) {
    if session.driver == ExternalDriver::Codex {
        for item in items {
            if let Some(correlation) = item.get_mut("_delivery_correlation") {
                correlation["thread_id"] = json!(session.native_id);
            }
        }
    }
}

pub fn normalize_native_line(
    driver: ExternalDriver,
    value: &Value,
    sequence: u64,
    fallback_timestamp: &str,
    items: &mut Vec<Value>,
) {
    if omission_reason(driver, value).is_some() {
        return;
    }
    let first = items.len();
    match driver {
        ExternalDriver::Codex => normalize_codex(value, sequence, fallback_timestamp, items),
        ExternalDriver::Claude => normalize_claude(value, sequence, fallback_timestamp, items),
        ExternalDriver::Pi | ExternalDriver::Omp => {
            normalize_omp(driver, value, sequence, fallback_timestamp, items)
        }
        // OpenCode history is stored in SQLite and never read line by line.
        ExternalDriver::OpenCode => {}
    }
    if items.len() == first {
        // No parser branch may silently discard a record outside the omission table.
        push_unrecognized(
            items,
            sequence,
            fallback_timestamp,
            driver.as_str(),
            "record",
            value.get("type").and_then(Value::as_str),
            value,
        );
    }
    // Keep the complete native record, including fields a typed projection does not use.
    // This is a display hint, not an authorization or content filter.
    let body = &mut items[first]["body"];
    if body.get("blocks").is_none() {
        body["blocks"] = json!([]);
    }
    body["blocks"].as_array_mut().unwrap().push(json!({
        "id":format!("native-{sequence}/source"), "kind":"source_record",
        "source_type":driver.as_str(), "visibility":"internal", "payload":{"raw":value}
    }));
    if let Some(correlation) = native_delivery_correlation(driver, value) {
        for item in &mut items[first..] {
            item["_delivery_correlation"] = correlation.clone();
        }
    }
}

// Only typed native user envelopes can carry a delivery identity. This private hint is
// consumed by the graph/native merger, never a public conversation field.
fn native_delivery_correlation(driver: ExternalDriver, value: &Value) -> Option<Value> {
    let (kind, id) = match driver {
        ExternalDriver::Claude => {
            let content = match value["type"].as_str() {
                Some("user") => value.pointer("/message/content")?,
                Some("attachment") if value["attachment"]["type"] == "queued_command" => {
                    value.pointer("/attachment/prompt")?
                }
                _ => return None,
            };
            // The channel puts its marker at the start of the delivered prompt. Do not
            // search quoted text, later parts, or arbitrary substrings for delivery markers.
            let text = match content {
                Value::String(text) => text.as_str(),
                Value::Array(parts) => {
                    let first = parts.first()?;
                    if first["type"].as_str().is_some_and(|kind| kind != "text") {
                        return None;
                    }
                    first["text"].as_str()?
                }
                _ => return None,
            };
            let marker = text.strip_prefix("[st3-delivery:")?;
            let (id, suffix) = marker.split_once(']')?;
            if !suffix.is_empty() && !suffix.starts_with('\n') {
                return None;
            }
            ("message", id)
        }
        ExternalDriver::Codex
            if value["type"] == "response_item"
                && value["payload"]["type"] == "message"
                && value["payload"]["role"] == "user" =>
        {
            let id = value
                .pointer("/payload/clientId")
                .and_then(Value::as_str)
                .or_else(|| {
                    value.pointer("/payload/clientUserMessageId").and_then(Value::as_str)
                })?;
            return (!id.is_empty()).then(|| json!({"kind":"codex", "id":id}));
        }
        _ => return None,
    };
    // Graph deliveries use their subject as the filename. Legacy filesystem deliveries
    // and malformed marker strings cannot identify a graph message.
    let subject = id.strip_prefix("message/")?;
    if subject.is_empty() || id.chars().any(char::is_whitespace) || id.contains(['[', ']']) {
        return None;
    }
    Some(json!({"kind":kind, "id":id}))
}

/// Recover a whole record glued behind a torn one.
///
/// A harness killed mid-append leaves a partial record, and its next append lands on the same
/// line: `{"type":"assistant","message":{..."stop_sequence":n{"type":"user",...}`. In valid JSON an
/// object only opens after `:`, `,`, `[`, or at the start, so an object opening after anything
/// else is where the torn record ends. The first such position whose remainder parses as one
/// whole object is the recovered record. The search is bounded.
fn recover_trailing_record(line: &str, preview: bool) -> Option<Value> {
    let bytes = line.as_bytes();
    let mut attempts = 0;
    for (index, _) in line.match_indices('{') {
        if index == 0 {
            continue;
        }
        let previous = bytes[..index]
            .iter()
            .rev()
            .find(|byte| !byte.is_ascii_whitespace());
        if matches!(previous, Some(b':' | b',' | b'[')) {
            continue;
        }
        attempts += 1;
        if attempts > 16 {
            return None;
        }
        if let Ok(value @ Value::Object(_)) = parse_native_record(&bytes[index..], preview) {
            return Some(value);
        }
    }
    None
}

/// Native records above this size paint with bounded normalized strings. The fold still reads
/// and retains the source bytes; a continuation normalizes their complete strings on demand.
const PREVIEW_RECORD_BYTES: usize = 256 * 1024;
/// The longest head a preview keeps of a string. With its marker it stays within the client
/// display window (8 KiB), so the marker reports the size of the complete native string.
const PREVIEW_STRING_BYTES: usize = 8 * 1024 - 128;

/// Parse one native record. A preview of an oversized record retains all keys and elements,
/// but bounds each normalized string. This avoids full long-string copies in typed bodies and
/// source-record payloads; it does not bound total record memory or avoid scanning source bytes.
fn parse_native_record(bytes: &[u8], preview: bool) -> serde_json::Result<Value> {
    if preview && bytes.len() > PREVIEW_RECORD_BYTES {
        serde_json::from_slice::<PreviewValue>(bytes).map(|PreviewValue(value)| value)
    } else {
        serde_json::from_slice(bytes)
    }
}

fn preview_text(text: &str) -> String {
    if text.len() <= PREVIEW_STRING_BYTES {
        return text.to_owned();
    }
    let mut end = PREVIEW_STRING_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[st truncated this native timeline value: size limit; {} bytes]",
        &text[..end],
        text.len()
    )
}

struct PreviewValue(Value);

impl<'de> Deserialize<'de> for PreviewValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(PreviewVisitor).map(Self)
    }
}

struct PreviewVisitor;

impl<'de> serde::de::Visitor<'de> for PreviewVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(preview_text(value)))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(PreviewValue(value)) = seq.next_element()? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some((key, PreviewValue(value))) = map.next_entry::<String, PreviewValue>()? {
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

/// Unparseable records preserve exact bytes, including newline, controls and invalid UTF-8.
/// A `preview` of an oversized record bounds every retained string (see `parse_native_record`)
/// and marks its entries with `_preview_of_bytes`; the owner serves the rest on request.
pub fn normalize_native_bytes(
    driver: ExternalDriver,
    bytes: &[u8],
    sequence: u64,
    timestamp: &str,
    line_index: usize,
    preview: bool,
    items: &mut Vec<Value>,
) {
    let first = items.len();
    match parse_native_record(bytes, preview) {
        Ok(value) => normalize_native_line(driver, &value, sequence, timestamp, items),
        Err(error) => {
            let recovered = std::str::from_utf8(bytes)
                .ok()
                .and_then(|line| recover_trailing_record(line, preview));
            let mut item = timeline_item(
                sequence,
                timestamp,
                "system",
                "error",
                json!({
                    "code":"native-line-unreadable",
                    "message":format!("st preserved an unreadable {} transcript record{}", driver.as_str(), if recovered.is_some() { " and recovered the record after the torn prefix" } else { "" }),
                    "retryable":false, "details":{"line_in_window":line_index.saturating_add(1),"parse_error":error.to_string()}
                }),
            );
            let kept = if preview && bytes.len() > PREVIEW_RECORD_BYTES {
                &bytes[..PREVIEW_STRING_BYTES]
            } else {
                bytes
            };
            item["body"]["blocks"]
                .as_array_mut()
                .unwrap()
                .insert(0, raw_bytes_block(sequence, driver.as_str(), kept));
            items.push(item);
            if let Some(value) = recovered {
                normalize_native_line(driver, &value, sequence.saturating_add(1), timestamp, items);
            }
        }
    }
    if preview && bytes.len() > PREVIEW_RECORD_BYTES {
        for item in &mut items[first..] {
            item["_preview_of_bytes"] = json!(bytes.len());
        }
    }
}

pub fn raw_bytes_block(sequence: u64, source_type: &str, bytes: &[u8]) -> Value {
    json!({"id":format!("native-{sequence}/raw"),"kind":"raw_text","source_type":source_type,
        "payload":{"encoding":"base64","bytes":BASE64.encode(bytes),"text":String::from_utf8_lossy(bytes)}})
}

/// Observed OMP administrative records that are visible events, not conversation text.
const OMP_ADMIN_ENTRIES: &[&str] = &[
    "model_usage",
    "title_change",
    "session_init",
    "ttsr_injection",
];
/// Explicit omission table: only these records may be absent from the data.
/// See docs/st3/conversation-normalization-design.md, native record visibility audit.
pub fn omission_reason(driver: ExternalDriver, value: &Value) -> Option<&'static str> {
    match (driver, value.get("type").and_then(Value::as_str)) {
        (ExternalDriver::Codex, Some("session_meta")) => Some("setup header, not a chat turn"),
        (ExternalDriver::Codex, Some("response_item"))
            if value.pointer("/payload/type").and_then(Value::as_str) == Some("message")
                && matches!(
                    value.pointer("/payload/role").and_then(Value::as_str),
                    Some("system" | "developer")
                ) =>
        {
            Some("provider bootstrap instructions, not a chat turn")
        }
        (ExternalDriver::Omp | ExternalDriver::Pi, Some("session")) => {
            Some("setup header, not a chat turn")
        }
        (ExternalDriver::Omp | ExternalDriver::Pi, Some("custom_message"))
            if value.get("display") == Some(&Value::Bool(false)) =>
        {
            Some("extension explicitly marks it hidden in the harness")
        }
        _ => None,
    }
}
pub const REASONING_BLOCKS: &[&str] = &["thinking", "redacted_thinking", "reasoning"];

/// A native timestamp as the timeline carries it. RFC 3339 text is kept as written; a number is
/// Unix milliseconds (or seconds, when too small to be milliseconds). Anything else is absent,
/// so the caller's fallback applies instead of an unparseable timestamp reaching a client.
fn native_timestamp(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|_| text.clone()),
        Value::Number(number) => {
            let value = number.as_u64()?;
            let millis = if value < 100_000_000_000 {
                value.saturating_mul(1_000)
            } else {
                value
            };
            Some(timestamp(u128::from(millis)))
        }
        _ => None,
    }
}

/// Preserve unknown native JSON and label its origin. Transport bounds are applied on reads.
/// Records without an explicit native attribution use the system role.
pub fn push_unrecognized(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    driver: &str,
    what: &str,
    kind: Option<&str>,
    value: &Value,
) {
    let block_kind = match kind {
        Some("thinking" | "redacted_thinking" | "reasoning") => {
            push_reasoning(items, sequence, timestamp, "system", value);
            return;
        }
        Some("job" | "job_start" | "job_update" | "job_end") => "job",
        Some("subagent" | "subagent_start" | "subagent_update" | "subagent_end" | "agent") => {
            "subagent"
        }
        Some("ask" | "ask_start" | "ask_update" | "ask_end") => "ask",
        Some(
            "status"
            | "model_change"
            | "thinking_level_change"
            | "step-start"
            | "step-finish"
            | "retry"
            | "compaction"
            | "compacted"
            | "summary",
        ) => "status",
        _ => "unknown",
    };
    let label = match kind {
        Some(kind) => format!("[unrecognized {driver} {what} `{kind}`]"),
        None => format!("[unrecognized {driver} {what} without a type]"),
    };
    let role = value
        .get("role")
        .or_else(|| value.pointer("/message/role"))
        .and_then(Value::as_str)
        .map(|role| normalized_role(Some(role)))
        .unwrap_or("system");
    push_native_block(
        items,
        sequence,
        timestamp,
        role,
        (block_kind, kind.unwrap_or(what)),
        json!({"raw": value}),
        &label,
    );
    if matches!(driver, "omp" | "pi")
        && let Some((kind, view, visibility)) = crate::native_views::omp_record_view(value)
        && let Some(item) = items.last_mut()
    {
        let block = &mut item["body"]["blocks"][0];
        block["kind"] = json!(kind);
        block["view"] = view;
        if let Some(visibility) = visibility {
            block["visibility"] = json!(visibility);
        }
    }
}

/// Native bodies stay owner-local. HTTP negotiation adds size bounds and owner fetch refs;
/// the text is a legacy-client fallback, not a separate capture or storage policy.
pub fn push_native_block(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    role: &str,
    (kind, source_type): (&str, &str),
    payload: Value,
    label: &str,
) {
    let fallback = match kind {
        "image" => "[image · load from owner]".to_owned(),
        "reasoning" => payload
            .get("text")
            .and_then(Value::as_str)
            .map(|text| format!("[reasoning]\n{text}"))
            .unwrap_or_else(|| format!("[reasoning]\n{payload}")),
        _ => format!("{label}\n{payload}"),
    };
    let mut item = timeline_item(
        sequence,
        timestamp,
        role,
        "content",
        json!({"media_type":"text/plain", "text":fallback}),
    );
    item["body"]["blocks"] = json!([{
        "id": format!("native-{sequence}/0"), "kind":kind,
        "source_type":source_type, "payload":payload
    }]);
    items.push(item);
}

pub fn push_reasoning(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    role: &str,
    part: &Value,
) {
    let text = part
        .get("thinking")
        .or_else(|| part.get("text"))
        .or_else(|| part.get("summary"))
        .cloned()
        .unwrap_or(Value::Null);
    push_native_block(
        items,
        sequence,
        timestamp,
        role,
        (
            "reasoning",
            part.get("type")
                .and_then(Value::as_str)
                .unwrap_or("reasoning"),
        ),
        json!({"text":text,"raw":part}),
        "[reasoning]",
    );
}

pub fn push_image(items: &mut Vec<Value>, sequence: u64, timestamp: &str, role: &str, part: &Value) {
    push_native_block(
        items,
        sequence,
        timestamp,
        role,
        (
            "image",
            part.get("type").and_then(Value::as_str).unwrap_or("image"),
        ),
        part.clone(),
        "[image]",
    );
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn normalize_codex(value: &Value, sequence: u64, fallback_timestamp: &str, items: &mut Vec<Value>) {
    if omission_reason(ExternalDriver::Codex, value).is_some() {
        return;
    }
    let timestamp =
        native_timestamp(value.get("timestamp")).unwrap_or_else(|| fallback_timestamp.to_owned());
    match value.get("type").and_then(Value::as_str) {
        Some("response_item") => {}
        kind => {
            push_unrecognized(items, sequence, &timestamp, "codex", "record", kind, value);
            return;
        }
    }
    let payload = &value["payload"];
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            if !matches!(payload["role"].as_str(), Some("user" | "assistant")) {
                push_unrecognized(
                    items,
                    sequence,
                    &timestamp,
                    "codex",
                    "message",
                    payload["role"].as_str(),
                    value,
                );
                return;
            }
            let role = normalized_role(payload["role"].as_str());
            let message_id = payload["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("native/{sequence}"));
            push_message(items, sequence, &timestamp, role, &message_id);
            match &payload["content"] {
                Value::Array(content) => {
                    for (offset, part) in content.iter().enumerate() {
                        let part_sequence = sequence + 1 + offset as u64;
                        if let Some(text) = part
                            .get("text")
                            .or_else(|| part.get("input_text"))
                            .or_else(|| part.get("output_text"))
                            .or_else(|| part.get("refusal"))
                            .and_then(Value::as_str)
                        {
                            push_content(items, part_sequence, &timestamp, role, text);
                            continue;
                        }
                        match part.get("type").and_then(Value::as_str) {
                            Some("input_image" | "output_image" | "image") => {
                                push_image(items, part_sequence, &timestamp, role, part)
                            }
                            kind => push_unrecognized(
                                items,
                                part_sequence,
                                &timestamp,
                                "codex",
                                "message part",
                                kind,
                                part,
                            ),
                        }
                    }
                }
                Value::String(text) => push_content(items, sequence + 1, &timestamp, role, text),
                Value::Null => {}
                other => push_unrecognized(
                    items,
                    sequence + 1,
                    &timestamp,
                    "codex",
                    "message content",
                    Some(json_kind(other)),
                    other,
                ),
            }
        }
        Some("function_call" | "custom_tool_call") => push_tool_call(
            items,
            sequence,
            &timestamp,
            payload["call_id"].as_str().unwrap_or("native-call"),
            payload["name"].as_str().unwrap_or("tool"),
            payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .cloned()
                .unwrap_or_else(|| json!({})),
        ),
        Some("function_call_output" | "custom_tool_call_output") => push_tool_result(
            items,
            sequence,
            &timestamp,
            payload["call_id"].as_str().unwrap_or("native-call"),
            payload.get("output").cloned().unwrap_or(Value::Null),
        ),
        Some("reasoning") => push_reasoning(items, sequence, &timestamp, "assistant", payload),
        kind => push_unrecognized(
            items,
            sequence,
            &timestamp,
            "codex",
            "response item",
            kind,
            payload,
        ),
    }
}

fn normalize_claude(
    value: &Value,
    sequence: u64,
    fallback_timestamp: &str,
    items: &mut Vec<Value>,
) {
    let timestamp =
        native_timestamp(value.get("timestamp")).unwrap_or_else(|| fallback_timestamp.to_owned());
    let role = match value.get("type").and_then(Value::as_str) {
        Some("user") => "user",
        Some("assistant") => "assistant",
        Some("attachment") => {
            // A prompt that arrives while Claude is busy, including a Small Talk delivery, is
            // recorded as a queued command rather than as a user entry. The other attachments
            // are context Claude injects for itself.
            if value.pointer("/attachment/type").and_then(Value::as_str) == Some("queued_command") {
                let message_id = value
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("native/{sequence}"));
                push_message(items, sequence, &timestamp, "user", &message_id);
                push_claude_content(
                    items,
                    value.pointer("/attachment/prompt").unwrap_or(&Value::Null),
                    sequence + 1,
                    &timestamp,
                    "user",
                );
            } else {
                push_unrecognized(
                    items,
                    sequence,
                    &timestamp,
                    "claude",
                    "attachment",
                    value.pointer("/attachment/type").and_then(Value::as_str),
                    value,
                );
            }
            return;
        }
        Some("system") => {
            // Claude's own notices (compaction, local command output, API errors) carry text;
            // its timing and hook summaries do not.
            if let Some(text) = value.get("content").and_then(Value::as_str)
                && !text.trim().is_empty()
            {
                push_content(items, sequence, &timestamp, "system", text);
            } else {
                push_unrecognized(
                    items,
                    sequence,
                    &timestamp,
                    "claude",
                    "system",
                    value.get("subtype").and_then(Value::as_str),
                    value,
                );
            }
            return;
        }
        kind => {
            push_unrecognized(items, sequence, &timestamp, "claude", "entry", kind, value);
            return;
        }
    };
    let message_id = value
        .pointer("/message/id")
        .and_then(Value::as_str)
        .or_else(|| value.get("uuid").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("native/{sequence}"));
    push_message(items, sequence, &timestamp, role, &message_id);
    push_claude_content(
        items,
        value.pointer("/message/content").unwrap_or(&Value::Null),
        sequence + 1,
        &timestamp,
        role,
    );
}

fn push_claude_content(
    items: &mut Vec<Value>,
    content: &Value,
    first_sequence: u64,
    timestamp: &str,
    role: &str,
) {
    match content {
        Value::String(text) => push_content(items, first_sequence, timestamp, role, text),
        Value::Array(parts) => {
            for (offset, part) in parts.iter().enumerate() {
                let item_sequence = first_sequence + offset as u64;
                match part.get("type").and_then(Value::as_str) {
                    Some("text") | None if part.get("text").is_some_and(Value::is_string) => {
                        push_content(
                            items,
                            item_sequence,
                            timestamp,
                            role,
                            part["text"].as_str().unwrap_or_default(),
                        );
                    }
                    Some("tool_use") => push_tool_call(
                        items,
                        item_sequence,
                        timestamp,
                        part["id"].as_str().unwrap_or("native-call"),
                        part["name"].as_str().unwrap_or("tool"),
                        part.get("input").cloned().unwrap_or_else(|| json!({})),
                    ),
                    Some("tool_result") => push_tool_result_with_status(
                        items,
                        item_sequence,
                        timestamp,
                        part["tool_use_id"].as_str().unwrap_or("native-call"),
                        part.get("content").cloned().unwrap_or(Value::Null),
                        part.get("is_error") == Some(&Value::Bool(true)),
                    ),
                    Some(kind) if REASONING_BLOCKS.contains(&kind) => {
                        push_reasoning(items, item_sequence, timestamp, role, part);
                    }
                    Some("image") => push_image(items, item_sequence, timestamp, role, part),
                    Some("document") => push_native_block(
                        items,
                        item_sequence,
                        timestamp,
                        role,
                        ("document", "claude/document"),
                        part.clone(),
                        "[document]",
                    ),
                    kind => push_unrecognized(
                        items,
                        item_sequence,
                        timestamp,
                        "claude",
                        "content block",
                        kind,
                        part,
                    ),
                }
            }
        }
        Value::Null => {}
        other => push_unrecognized(
            items,
            first_sequence,
            timestamp,
            "claude",
            "message content",
            Some(json_kind(other)),
            other,
        ),
    }
}

/// OMP persists a failed assistant turn with `stopReason: "error"`, an `errorMessage`, and —
/// once an automatic retry settles — a `retryRecovery` marker recording how the attempt was
/// recovered or superseded. Project the presentation exactly as the harness resolves it:
/// superseded attempts are hidden, recovered ones render a compact note that still discloses
/// the original error, silent and user-interrupt aborts render nothing, and terminal failures
/// render in full. `label` is the resolved display text; `message` keeps the original error.
fn omp_assistant_error_view(message: &Value) -> Option<Value> {
    const SILENT_ABORT_MARKER: &str = "__omp.silent_abort__";
    const USER_INTERRUPT_LABEL: &str = "Interrupted by user";
    const GENERIC_ABORT_SENTINEL: &str = "Request was aborted";
    let retry = message
        .get("retryRecovery")
        .filter(|retry| retry.is_object());
    let stop_reason = message.get("stopReason").and_then(Value::as_str);
    let error_message = message.get("errorMessage").and_then(Value::as_str);
    let error_id = message.get("errorId").and_then(Value::as_i64);
    let flagged = |bit: i64| error_id.is_some_and(|id| id & bit != 0);
    let silent_abort = error_message == Some(SILENT_ABORT_MARKER) || flagged(0x0200_0000);
    let user_interrupt = error_message == Some(USER_INTERRUPT_LABEL) || flagged(0x0400_0000);
    let retry_note = || {
        retry
            .and_then(|retry| retry.get("note"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let (status, presentation, is_error, label) = match retry
        .and_then(|retry| retry.get("status"))
        .and_then(Value::as_str)
    {
        Some("recovered") => ("recovered", "compact-recovered", false, retry_note()),
        Some("superseded") => ("superseded", "none", false, ""),
        _ => match stop_reason {
            Some("error") => ("failed", "full", true, error_message.unwrap_or("Error")),
            Some("aborted") => {
                if silent_abort || user_interrupt {
                    return None;
                }
                let custom = error_message.is_some_and(|text| {
                    !text.is_empty()
                        && text != GENERIC_ABORT_SENTINEL
                        && text != SILENT_ABORT_MARKER
                });
                (
                    "failed",
                    "full",
                    true,
                    if custom {
                        error_message.unwrap_or_default()
                    } else {
                        "Operation aborted"
                    },
                )
            }
            Some("length") => (
                "failed",
                "full",
                true,
                error_message.unwrap_or(
                    "The native assistant reached its output limit; its response may be incomplete.",
                ),
            ),
            Some(stop) if !matches!(stop, "stop" | "toolUse") => (
                "failed",
                "full",
                true,
                error_message.unwrap_or("The native assistant stopped with an unsupported reason."),
            ),
            _ => {
                let text = error_message?;
                if text.is_empty() || silent_abort || user_interrupt {
                    return None;
                }
                ("failed", "full", true, text)
            }
        },
    };
    let mut view = json!({
        "type": "assistant_error",
        "status": status,
        "presentation": presentation,
        "is_error": is_error,
        "label": label,
        "message": error_message.unwrap_or(""),
    });
    // Keep the exact native diagnostics alongside the harness presentation projection.
    let mut native = serde_json::Map::new();
    for field in [
        "stopReason",
        "errorMessage",
        "errorStatus",
        "errorId",
        "retryRecovery",
    ] {
        if let Some(value) = message.get(field) {
            native.insert(field.to_owned(), value.clone());
        }
    }
    view["native"] = Value::Object(native);
    if let Some(stop_reason) = stop_reason {
        view["stop_reason"] = json!(stop_reason);
    }
    if let Some(error_id) = error_id {
        view["error_id"] = json!(error_id);
    }
    for field in ["api", "provider", "model"] {
        if let Some(value) = message.get(field).and_then(Value::as_str) {
            view[field] = json!(value);
        }
    }
    if let Some(retry) = retry {
        let mut projected = json!({});
        for field in ["kind", "status", "recovery", "note"] {
            if let Some(value) = retry.get(field).and_then(Value::as_str) {
                projected[field] = json!(value);
            }
        }
        if let Some(attempt) = retry.get("attempt").and_then(Value::as_u64) {
            projected["attempt"] = json!(attempt);
        }
        if let Some(recovered_at) = retry.get("recoveredAt").and_then(Value::as_str) {
            projected["recovered_at"] = json!(recovered_at);
        }
        if let Some(superseded) = retry.get("supersededBy").filter(|value| value.is_object()) {
            let mut by = json!({});
            if let Some(at) = superseded.get("timestamp").and_then(Value::as_i64) {
                by["timestamp"] = json!(at);
            }
            for (target, field) in [
                ("response_id", "responseId"),
                ("provider", "provider"),
                ("model", "model"),
            ] {
                if let Some(value) = superseded.get(field).and_then(Value::as_str) {
                    by[target] = json!(value);
                }
            }
            if by.as_object().is_some_and(|by| !by.is_empty()) {
                projected["superseded_by"] = by;
            }
        }
        if projected
            .as_object()
            .is_some_and(|projected| !projected.is_empty())
        {
            view["retry"] = projected;
        }
    }
    Some(view)
}

pub fn normalize_omp(
    driver: ExternalDriver,
    value: &Value,
    sequence: u64,
    fallback_timestamp: &str,
    items: &mut Vec<Value>,
) {
    if omission_reason(driver, value).is_some() {
        return;
    }
    let label = driver.as_str();
    let timestamp = native_timestamp(value.get("timestamp"))
        .or_else(|| native_timestamp(value.pointer("/message/timestamp")))
        .unwrap_or_else(|| fallback_timestamp.to_owned());
    let record_view = crate::native_views::omp_record_view(value);
    let first_item = items.len();
    match value.get("type").and_then(Value::as_str) {
        Some("message") => {}
        Some(kind @ ("compaction" | "branch_summary")) => {
            // A summary replaces the conversation before it in the model's context; show it as
            // the harness's own note.
            if let Some(summary) = value.get("summary").and_then(Value::as_str) {
                push_content(
                    items,
                    sequence,
                    &timestamp,
                    "system",
                    &format!("[{label} {kind}]\n{summary}"),
                );
            } else {
                push_unrecognized(
                    items,
                    sequence,
                    &timestamp,
                    label,
                    "entry",
                    Some(kind),
                    value,
                );
            }
            if let Some((kind, view, visibility)) = record_view
                && let Some(item) = items.last_mut()
            {
                let block = &mut item["body"]["blocks"][0];
                block["kind"] = json!(kind);
                block["payload"] = json!({"raw":value});
                block["view"] = view;
                if let Some(visibility) = visibility {
                    block["visibility"] = json!(visibility);
                }
            }
            return;
        }
        Some("custom_message") => {
            // An extension's message; `display: false` marks it hidden in the harness itself.
            if value.get("display") != Some(&Value::Bool(false)) {
                push_omp_content(
                    driver,
                    items,
                    value.get("content").unwrap_or(&Value::Null),
                    sequence,
                    &timestamp,
                    "system",
                );
                if let Some((kind, view, visibility)) = record_view {
                    // A message with no specific view keeps main's block shape (`text`); it
                    // gains the `omp_event` view and its complete native record.
                    let generic = view["type"] == "omp_event";
                    for item in &mut items[first_item..] {
                        if let Some(blocks) = item["body"]["blocks"].as_array_mut() {
                            for block in blocks {
                                if generic {
                                    if let Some(payload) = block["payload"].as_object_mut() {
                                        payload.insert("raw".into(), value.clone());
                                    } else {
                                        block["payload"] = json!({"raw":value});
                                    }
                                } else {
                                    block["kind"] = json!(kind);
                                    block["payload"] = json!({"raw":value});
                                }
                                block["view"] = view.clone();
                                if let Some(visibility) = visibility {
                                    block["visibility"] = json!(visibility);
                                }
                            }
                        }
                    }
                    if items.len() == first_item {
                        push_unrecognized(
                            items,
                            sequence,
                            &timestamp,
                            label,
                            "entry",
                            Some("custom_message"),
                            value,
                        );
                    }
                }
            }
            return;
        }
        // Observed OMP administrative records are typed status events; the complete record stays
        // in the raw payload. `display: false` is kept and marked, not dropped: main's omission
        // table lists the only records that may be absent.
        Some(kind) if OMP_ADMIN_ENTRIES.contains(&kind) => {
            if let Some((block_kind, view, visibility)) = record_view {
                push_native_block(
                    items,
                    sequence,
                    &timestamp,
                    "system",
                    (block_kind, kind),
                    json!({"raw": value}),
                    &format!("[{label} {kind}]"),
                );
                let block = &mut items.last_mut().unwrap()["body"]["blocks"][0];
                block["view"] = view;
                let hidden = value.get("display") == Some(&Value::Bool(false));
                if let Some(visibility) = visibility.or(hidden.then_some("hidden-by-harness")) {
                    block["visibility"] = json!(visibility);
                }
            } else {
                push_unrecognized(items, sequence, &timestamp, label, "entry", Some(kind), value);
            }
            return;
        }
        Some("custom")
            if driver == ExternalDriver::Omp
                && value.get("customType").and_then(Value::as_str) == Some("session_exit") =>
        {
            push_omp_exit(items, sequence, &timestamp, &value["data"]);
            let block = &mut items.last_mut().expect("exit item just pushed")["body"]["blocks"][0];
            block["payload"] = json!({"body_ref": true, "raw": value});
            if let Some((_, view, visibility)) = record_view
                && view["kind"].is_string()
                && view["reason"].is_string()
            {
                block["view"] = view;
                if let Some(visibility) = visibility {
                    block["visibility"] = json!(visibility);
                }
            }
            return;
        }
        Some("custom") => {
            if let Some((kind, view, visibility)) = record_view
                && view["type"] == "tool_start"
            {
                let tool = view["tool"].as_str().unwrap_or("tool");
                push_native_block(
                    items,
                    sequence,
                    &timestamp,
                    "system",
                    (kind, "custom"),
                    json!({"raw": value}),
                    &format!("[{label} tool start {tool}]"),
                );
                let block = &mut items.last_mut().unwrap()["body"]["blocks"][0];
                block["view"] = view;
                if let Some(visibility) = visibility {
                    block["visibility"] = json!(visibility);
                }
            } else {
                push_unrecognized(
                    items,
                    sequence,
                    &timestamp,
                    label,
                    "entry",
                    Some("custom"),
                    value,
                );
            }
            return;
        }
        Some(record_type @ ("reset_boundary" | "credential_pin" | "title")) => {
            let text = match record_type {
                "reset_boundary" => format!("[{label} session reset]"),
                "credential_pin" => format!(
                    "[{label} credential pin {}]",
                    value["provider"].as_str().unwrap_or_default()
                ),
                _ => format!(
                    "[{label} title] {}",
                    value["title"].as_str().unwrap_or_default()
                ),
            };
            if let Some((kind, view, visibility)) = record_view {
                push_native_block(
                    items,
                    sequence,
                    &timestamp,
                    "system",
                    (kind, record_type),
                    json!({"raw": value}),
                    &text,
                );
                let block = &mut items.last_mut().unwrap()["body"]["blocks"][0];
                block["view"] = view;
                if let Some(visibility) = visibility {
                    block["visibility"] = json!(visibility);
                }
            }
            return;
        }
        kind => {
            push_unrecognized(items, sequence, &timestamp, label, "entry", kind, value);
            return;
        }
    }
    let message = &value["message"];
    let native_role = message.get("role").and_then(Value::as_str);
    if let Some(native_role) = native_role
        && !matches!(
            native_role,
            "user"
                | "assistant"
                | "system"
                | "developer"
                | "tool"
                | "toolResult"
                | "tool_result"
                | "bashExecution"
                | "branchSummary"
                | "compactionSummary"
        )
    {
        push_unrecognized(
            items,
            sequence,
            &timestamp,
            label,
            "message role",
            Some(native_role),
            value,
        );
        return;
    }
    let role = normalized_role(native_role);
    let message_id = value["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("native/{sequence}"));
    push_message(items, sequence, &timestamp, role, &message_id);
    match native_role {
        Some("toolResult") if message.get("toolCallId").is_some_and(Value::is_string) => {
            push_tool_result_with_status(
                items,
                sequence + 1,
                &timestamp,
                message["toolCallId"].as_str().unwrap_or_default(),
                message.get("content").cloned().unwrap_or(Value::Null),
                message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
            preserve_omp_result_timing(items, message);
            if let Some(item) = items.last_mut() {
                item["body"]["blocks"][0]["view"] = crate::native_views::tool_output_view(
                    message
                        .get("toolName")
                        .and_then(Value::as_str)
                        .unwrap_or("tool"),
                    message["toolCallId"].as_str().unwrap_or_default(),
                    message
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    message.get("details"),
                );
                item["body"]["blocks"][0]["payload"] = json!({"body_ref":true,"raw":value});
            }
            return;
        }
        // A shell command the person ran with `!`: shown as the harness recorded it, without
        // attributing it to the agent.
        Some("bashExecution") => {
            let command = message
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = message
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default();
            push_content(
                items,
                sequence + 1,
                &timestamp,
                "system",
                &format!("$ {command}\n{output}"),
            );
            return;
        }
        Some("branchSummary" | "compactionSummary")
            if message.get("content").is_none_or(Value::is_null) =>
        {
            if let Some(summary) = message.get("summary").and_then(Value::as_str) {
                push_content(items, sequence + 1, &timestamp, "system", summary);
            }
            return;
        }
        _ => {}
    }
    let first_content = items.len();
    push_omp_content(
        driver,
        items,
        message.get("content").unwrap_or(&Value::Null),
        sequence + 1,
        &timestamp,
        role,
    );
    if native_role == Some("assistant")
        && let Some(view) = omp_assistant_error_view(message)
    {
        // The error presentation is its own block after the turn's content, so every
        // existing native block identity stays stable.
        let error_sequence = items
            .last()
            .and_then(|item| item["sequence"].as_u64())
            .map_or(sequence + 1, |last| last + 1);
        let label = view["label"].as_str().unwrap_or_default().to_owned();
        let fallback = match view["presentation"].as_str() {
            Some("full") => format!("[error] {label}"),
            Some("compact-recovered") => format!("[retry recovered] {label}"),
            _ => String::new(),
        };
        push_content(items, error_sequence, &timestamp, "assistant", &fallback);
        let block = &mut items.last_mut().expect("error item just pushed")["body"]["blocks"][0];
        block["kind"] = json!("error");
        block["source_type"] = json!(format!("{}/assistant_error", driver.as_str()));
        block["payload"] = json!({"body_ref": true});
        block["view"] = view;
    }
    // Message-level metadata has one owner: the last emitted block, including a
    // tool call or an error. Newest-first header windows retain that block even
    // when they split a message, without counting each part as another response.
    if let Some(metadata) = crate::native_views::assistant_metadata(message)
        && let Some(block) = items[first_content..]
            .iter_mut()
            .rev()
            .filter_map(|item| item["body"]["blocks"].as_array_mut())
            .find_map(|blocks| blocks.last_mut())
    {
        if let Some(existing) = block.get_mut("metadata").and_then(Value::as_object_mut) {
            if let Some(additions) = metadata.as_object() {
                existing.extend(
                    additions
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
        } else {
            block["metadata"] = metadata;
        }
    }
}

fn push_omp_exit(items: &mut Vec<Value>, sequence: u64, timestamp: &str, data: &Value) {
    // A stopped process says nothing about the turn or pending tools. Unknown exits
    // must not look successful; keep the complete checkpoint in either body shape.
    let kind = data.get("kind").and_then(Value::as_str).unwrap_or_default();
    let reason = data
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let exit_code = data
        .get("exitCode")
        .or_else(|| data.get("code"))
        .and_then(Value::as_i64);
    let fatal_reason = matches!(
        reason,
        "fatal"
            | "crash"
            | "uncaughtException"
            | "uncaught_exception"
            | "unhandledRejection"
            | "unhandled_rejection"
    );
    let code = match kind {
        "fatal"
        | "crash"
        | "uncaughtException"
        | "uncaught_exception"
        | "unhandledRejection"
        | "unhandled_rejection" => Some("native_session_exit_fatal"),
        "normal" | "signal" | "process_exit" | "exit"
            if fatal_reason || exit_code.is_some_and(|code| code != 0) =>
        {
            Some("native_session_exit_fatal")
        }
        "normal" => None,
        "signal" if matches!(reason, "sigterm" | "sigint" | "sighup") => None,
        "process_exit" | "exit" if exit_code == Some(0) => None,
        _ => Some("native_session_exit_unknown"),
    };
    let (entry_type, body) = if let Some(code) = code {
        (
            "error",
            json!({
                "code":code,
                "message":if code == "native_session_exit_fatal" {
                    "The native OMP process exited abnormally."
                } else {
                    "The native OMP process exit outcome is unknown."
                },
                "retryable":false,"details":data,
            }),
        )
    } else {
        (
            "status",
            json!({
                "status":"cancelled",
                "detail":serde_json::to_string(data).expect("native exit metadata serializes"),
            }),
        )
    };
    items.push(timeline_item(
        sequence, timestamp, "system", entry_type, body,
    ));
}

fn push_omp_content(
    driver: ExternalDriver,
    items: &mut Vec<Value>,
    content: &Value,
    first_sequence: u64,
    timestamp: &str,
    role: &str,
) {
    let label = driver.as_str();
    let timestamp = timestamp.to_owned();
    match content {
        Value::String(text) => push_content(items, first_sequence, &timestamp, role, text),
        Value::Array(parts) => {
            for (offset, part) in parts.iter().enumerate() {
                let item_sequence = first_sequence + offset as u64;
                match part.get("type").and_then(Value::as_str) {
                    Some("text") | None if part.get("text").is_some_and(Value::is_string) => {
                        push_content(
                            items,
                            item_sequence,
                            &timestamp,
                            role,
                            part["text"].as_str().unwrap_or_default(),
                        );
                    }
                    Some("toolCall" | "tool_call") => {
                        let tool = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                        push_tool_call(
                            items,
                            item_sequence,
                            &timestamp,
                            part.get("id")
                                .or_else(|| part.get("toolCallId"))
                                .and_then(Value::as_str)
                                .unwrap_or("native-call"),
                            tool,
                            part.get("arguments").cloned().unwrap_or_else(|| json!({})),
                        );
                        if let Some(item) = items.last_mut() {
                            item["body"]["blocks"][0]["view"] = crate::native_views::tool_call_view(
                                tool,
                                &item["body"]["arguments"],
                            );
                        }
                    }
                    Some("toolResult" | "tool_result") => {
                        let call_id = part
                            .get("toolCallId")
                            .or_else(|| part.get("call_id"))
                            .and_then(Value::as_str)
                            .unwrap_or("native-call");
                        push_tool_result_with_status(
                            items,
                            item_sequence,
                            &timestamp,
                            call_id,
                            part.get("content").cloned().unwrap_or(Value::Null),
                            part.get("isError") == Some(&Value::Bool(true)),
                        );
                        if let Some(item) = items.last_mut() {
                            item["body"]["blocks"][0]["view"] =
                                crate::native_views::tool_output_view(
                                    part.get("toolName")
                                        .or_else(|| part.get("name"))
                                        .and_then(Value::as_str)
                                        .unwrap_or("tool"),
                                    call_id,
                                    part.get("isError")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                    part.get("details"),
                                );
                            item["body"]["blocks"][0]["payload"] =
                                json!({"body_ref":true,"raw":part});
                        }
                        preserve_omp_result_timing(items, part);
                    }
                    Some(kind) if REASONING_BLOCKS.contains(&kind) => {
                        push_reasoning(items, item_sequence, &timestamp, role, part);
                    }
                    Some("image" | "input_image" | "image_url") => {
                        push_image(items, item_sequence, &timestamp, role, part);
                    }
                    kind => push_unrecognized(
                        items,
                        item_sequence,
                        &timestamp,
                        label,
                        "content block",
                        kind,
                        part,
                    ),
                }
            }
        }
        Value::Null => {}
        other => push_unrecognized(
            items,
            first_sequence,
            &timestamp,
            label,
            "message content",
            Some(json_kind(other)),
            other,
        ),
    }
}

pub fn normalized_role(role: Option<&str>) -> &'static str {
    match role {
        Some("user") => "user",
        Some("assistant") => "assistant",
        Some("tool" | "toolResult" | "tool_result") => "tool",
        _ => "system",
    }
}

pub fn push_message(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    role: &str,
    message_id: &str,
) {
    items.push(timeline_item(
        sequence,
        timestamp,
        role,
        "message",
        json!({"message_id": message_id}),
    ));
}

pub fn push_content(items: &mut Vec<Value>, sequence: u64, timestamp: &str, role: &str, text: &str) {
    items.push(timeline_item(
        sequence,
        timestamp,
        role,
        "content",
        json!({"media_type":"text/plain", "text":text}),
    ));
}

pub fn push_tool_call(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    call_id: &str,
    name: &str,
    arguments: Value,
) {
    let arguments = arguments
        .as_str()
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or(arguments);
    items.push(timeline_item(
        sequence,
        timestamp,
        "assistant",
        "tool_call",
        json!({"call_id":call_id, "name":name, "arguments":arguments}),
    ));
}

fn push_tool_result(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    call_id: &str,
    content: Value,
) {
    push_tool_result_with_status(items, sequence, timestamp, call_id, content, false);
}

pub fn push_tool_result_with_status(
    items: &mut Vec<Value>,
    sequence: u64,
    timestamp: &str,
    call_id: &str,
    content: Value,
    failed: bool,
) {
    let status = if failed { "error" } else { "success" };
    items.push(timeline_item(sequence, timestamp, "tool", "tool_result", json!({"call_id":call_id, "status":status, "media_type":"application/json", "content":content})));
}

fn preserve_omp_result_timing(items: &mut [Value], native_result: &Value) {
    let Some(details) = native_result.get("details").and_then(Value::as_object) else {
        return;
    };
    if details.is_empty() {
        return;
    }
    let blocks = items.last_mut().expect("result was just appended")["body"]["blocks"]
        .as_array_mut()
        .expect("tool result has normalized blocks");
    for block in blocks {
        if block["kind"] != "tool_output" {
            continue;
        }
        // Keep the native details object open: original names, units and future fields
        // survive beside any metadata already supplied by normalization.
        let metadata = block
            .as_object_mut()
            .expect("normalized block is an object")
            .entry("metadata")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("block metadata is an object");
        metadata.extend(
            details
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
}

pub fn timeline_item(
    sequence: u64,
    timestamp: &str,
    role: &str,
    entry_type: &str,
    mut body: Value,
) -> Value {
    let kind = match entry_type {
        "content" => Some("text"),
        "tool_call" => Some("tool_call"),
        "tool_result" => Some("tool_output"),
        "status" => Some("status"),
        "error" => Some("error"),
        _ => None,
    };
    if let Some(kind) = kind {
        body["blocks"] = json!([{
            "id":format!("native-{sequence}/0"), "kind":kind,
            "source_type":entry_type, "payload":{"body_ref":true}
        }]);
    }
    json!({
        "id": format!("timeline-entry/native-{sequence}"),
        "sequence": sequence,
        "revision": 1,
        "timestamp": timestamp,
        "role": role,
        "type": entry_type,
        "final": true,
        "body": body
    })
}

pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::external_sessions::shared_tests_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::external_sessions::shared_tests_seats::{CLAUDE_WINDOW_FILLER, assert_unique_ids, chat_texts, claude_line, codex_record, texts, transcript_session, unfurled_timeline};
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub use crate::external_sessions::shared_tests_seats::path_executable;
use super::*;

    fn assert_omp_message_usage_once(content: Value) {
        for driver in [ExternalDriver::Omp, ExternalDriver::Pi] {
            let mut items = Vec::new();
            for (sequence, usd) in [(0, 0.25), (1000, 0.5)] {
                let record = json!({
                    "type": "message",
                    "message": {
                        "role": "assistant", "content": content, "model": "test-model",
                        "usage": {"input": 12, "output": 5, "totalTokens": 17, "cost": {"total": usd}},
                        "contextSnapshot": {"promptTokens": 23},
                    },
                });
                let first = items.len();
                normalize_omp(
                    driver,
                    &record,
                    sequence,
                    "2026-10-06T00:00:00Z",
                    &mut items,
                );
                let blocks: Vec<_> = items[first..]
                    .iter()
                    .filter_map(|item| item["body"]["blocks"].as_array())
                    .flatten()
                    .collect();
                let owner = blocks.last().unwrap();
                assert_eq!(owner["metadata"]["usage"]["cost_usd"], usd);
                assert_eq!(owner["metadata"]["usage"]["total"], 17);
                assert_eq!(
                    blocks
                        .iter()
                        .filter(|block| block["metadata"]["usage"].is_object())
                        .count(),
                    1
                );
            }
            let header = crate::conversation_header::derive(&items, false);
            assert_eq!(header["cost"]["value"]["usd"], 0.75);
            assert_eq!(header["model"]["value"], "test-model");
            assert_eq!(header["context"]["value"]["tokens"], 23);
        }
    }

    #[test]
    fn omp_thinking_and_text_count_message_usage_once() {
        assert_omp_message_usage_once(json!([
            {"type": "thinking", "thinking": "Consider the request"},
            {"type": "text", "text": "Answer"},
        ]));
    }

    #[test]
    fn omp_tool_call_only_counts_message_usage_once() {
        assert_omp_message_usage_once(json!([
            {"type": "toolCall", "id": "first", "name": "read", "arguments": {"path": "a"}},
            {"type": "toolCall", "id": "second", "name": "read", "arguments": {"path": "b"}},
        ]));
    }

    #[test]
    fn omp_split_newest_header_window_keeps_latest_message_metadata() {
        let mut content: Vec<_> = (0..201)
            .map(|index| json!({"type": "text", "text": format!("part {index}")}))
            .collect();
        content.push(json!({
            "type": "toolCall", "id": "last-call", "name": "read", "arguments": {"path": "a"},
        }));
        assert_omp_message_usage_once(json!(content));
        for driver in [ExternalDriver::Omp, ExternalDriver::Pi] {
            let mut items = Vec::new();
            for (sequence, message, timestamp) in [
                (
                    0,
                    json!({
                        "role": "assistant", "content": "previous answer", "model": "old-model",
                        "usage": {"cost": {"total": 0.25}},
                        "contextSnapshot": {"promptTokens": 11},
                    }),
                    "2026-10-06T00:00:00Z",
                ),
                (
                    1000,
                    json!({
                        "role": "assistant", "content": content, "model": "latest-model",
                        "usage": {"input": 12, "output": 5, "totalTokens": 17, "cost": {"total": 0.5}},
                        "contextSnapshot": {"promptTokens": 23}, "ttft": 10, "duration": 30,
                    }),
                    "2026-10-06T00:00:01Z",
                ),
            ] {
                normalize_omp(
                    driver,
                    &json!({"type": "message", "message": message}),
                    sequence,
                    timestamp,
                    &mut items,
                );
            }
            let owner = &items.last().unwrap()["body"]["blocks"][0];
            assert_eq!(owner["kind"], "tool_call");
            assert_eq!(owner["metadata"]["usage"]["total"], 17);
            assert_eq!(owner["metadata"]["ttft_ms"], 10);
            assert_eq!(owner["metadata"]["duration_ms"], 30);
            // Match the native header's newest-first 200-item window: the latest
            // message spans 202 entries, so its first projected blocks are absent.
            items.reverse();
            let head = &items[..200];
            assert_eq!(head.len(), 200);
            assert!(
                head.iter()
                    .all(|item| item["sequence"].as_u64().unwrap() > 1001)
            );
            let header = crate::conversation_header::derive(head, true);
            assert_eq!(header["cost"]["value"], json!({"usd": 0.5, "window": true}));
            assert_eq!(header["model"]["value"], "latest-model");
            assert_eq!(
                header["context"]["value"],
                json!({"tokens": 23, "window": null})
            );
            for field in ["cost", "model", "context"] {
                assert_eq!(header[field]["as_of"], "2026-10-06T00:00:01Z");
            }
        }
    }

    #[test]
    fn omp_parity_fixture_projects_all_views_without_losing_native_payloads() {
        for driver in [ExternalDriver::Omp, ExternalDriver::Pi] {
            let mut items = Vec::new();
            for (index, line) in include_str!("../../st3-daemon/fixtures/native-records/omp-parity.jsonl")
                .lines()
                .enumerate()
            {
                let record: Value = serde_json::from_str(line).unwrap();
                let first = items.len();
                normalize_omp(
                    driver,
                    &record,
                    index as u64 * 100,
                    "2026-10-06T00:00:00Z",
                    &mut items,
                );
                if let Some((_, view, _)) = crate::native_views::omp_record_view(&record) {
                    let block = &items[first]["body"]["blocks"][0];
                    assert_eq!(block["payload"]["raw"], record);
                    assert_eq!(block["view"], view);
                }
                if record.pointer("/message/role").and_then(Value::as_str) == Some("toolResult") {
                    assert_eq!(
                        items.last().unwrap()["body"]["content"],
                        record["message"]["content"]
                    );
                    assert_eq!(
                        items.last().unwrap()["body"]["blocks"][0]["payload"]["raw"],
                        record
                    );
                }
                if let Some(parts) = record.pointer("/message/content").and_then(Value::as_array) {
                    for part in parts.iter().filter(|part| part["type"] == "toolCall") {
                        let call = items[first..]
                            .iter()
                            .find(|item| item["type"] == "tool_call")
                            .unwrap();
                        assert_eq!(call["body"]["arguments"], part["arguments"]);
                    }
                }
            }
            let pairs: std::collections::BTreeSet<_> = items
                .iter()
                .flat_map(|item| item["body"]["blocks"].as_array().into_iter().flatten())
                .filter_map(|block| {
                    Some((block["kind"].as_str()?, block["view"]["type"].as_str()?))
                })
                .collect();
            for expected in [
                ("tool_call", "bash"),
                ("tool_call", "edit"),
                ("tool_call", "write"),
                ("tool_call", "read"),
                ("tool_call", "search"),
                ("tool_call", "todo"),
                ("tool_call", "ask"),
                ("tool_call", "task"),
                ("tool_call", "hub"),
                ("tool_call", "eval"),
                ("tool_call", "generic"),
                ("tool_output", "bash"),
                ("tool_output", "edit"),
                ("tool_output", "search"),
                ("tool_output", "todo"),
                ("tool_output", "ask"),
                ("tool_output", "task"),
                ("tool_output", "hub"),
                ("tool_output", "generic"),
                ("irc", "irc"),
                ("job", "job"),
                ("status", "skill"),
                ("status", "compaction"),
                ("status", "model_change"),
                ("status", "thinking_level"),
                ("status", "title"),
                ("status", "session_exit"),
                ("status", "tool_start"),
                ("error", "assistant_error"),
            ] {
                assert!(
                    pairs.contains(&expected),
                    "missing {expected:?} for {driver:?}"
                );
            }
            let blocks: Vec<_> = items
                .iter()
                .flat_map(|item| item["body"]["blocks"].as_array().into_iter().flatten())
                .collect();
            assert!(
                blocks
                    .iter()
                    .any(|block| block["view"]["type"] == "tool_start"
                        && block["visibility"] == "internal")
            );
            assert!(
                blocks
                    .iter()
                    .any(|block| block["metadata"]["context_tokens"] == 23
                        && block["metadata"]["usage"]["cost_usd"] == 0.01)
            );
            for kind in ["text", "reasoning"] {
                assert!(blocks.iter().any(|block| block["kind"] == kind));
            }
            assert!(items.iter().any(|item| item["body"]["text"]
                == "Synthetic extension fallback"
                && item["body"]["blocks"][0]["kind"] == "text"));
            assert!(
                !items
                    .iter()
                    .any(|item| item["body"]["text"] == "Synthetic hidden extension")
            );
        }
    }

    #[test]
    fn timeline_send_identity_marks_every_native_user_entry_without_changing_projection() {
        let subject = "message/client-send";
        let marker = format!("[st3-delivery:{subject}]\nhello");
        let cases = [
            (
                ExternalDriver::Claude,
                json!({"type":"user","uuid":"claude-user","message":{"role":"user","content":[{"type":"text","text":marker},{"type":"image","source":{"type":"base64","data":"image"}}]}}),
                json!({"kind":"message","id":subject}),
            ),
            (
                ExternalDriver::Claude,
                json!({"type":"attachment","uuid":"queued-user","attachment":{"type":"queued_command","prompt":marker}}),
                json!({"kind":"message","id":subject}),
            ),
            (
                ExternalDriver::Claude,
                json!({"type":"user","message":{"content":marker}}),
                json!({"kind":"message","id":subject}),
            ),
            (
                ExternalDriver::Codex,
                json!({"type":"response_item","payload":{"type":"message","role":"user","clientId":"st:client","content":[{"type":"input_text","text":"hello"},{"type":"input_image","image_url":"image"}]}}),
                json!({"kind":"codex","id":"st:client"}),
            ),
            (
                ExternalDriver::Codex,
                json!({"type":"response_item","payload":{"type":"message","role":"user","clientUserMessageId":"st:client","content":"hello"}}),
                json!({"kind":"codex","id":"st:client"}),
            ),
        ];
        for (driver, value, expected) in cases {
            let mut entries = Vec::new();
            normalize_native_line(driver, &value, 16, &timestamp(0), &mut entries);
            assert!(entries.len() >= 2);
            for entry in &mut entries {
                assert_eq!(entry["_delivery_correlation"], expected);
                entry.as_object_mut().unwrap().remove("_delivery_correlation");
            }
            let mut projection = Vec::new();
            match driver {
                ExternalDriver::Pi | ExternalDriver::Omp => {
                    normalize_omp(driver, &value, 16, &timestamp(0), &mut projection);
                }
                ExternalDriver::Claude => {
                    normalize_claude(&value, 16, &timestamp(0), &mut projection);
                }
                ExternalDriver::Codex => {
                    normalize_codex(&value, 16, &timestamp(0), &mut projection);
                }
                ExternalDriver::OpenCode => unreachable!(),
            }
            // The wrapper's pre-existing source record is the only projection difference.
            entries[0]["body"]["blocks"].as_array_mut().unwrap().pop();
            if projection[0]["body"].get("blocks").is_none() {
                entries[0]["body"].as_object_mut().unwrap().remove("blocks");
            }
            assert_eq!(entries, projection);
        }
    }

    #[test]
    fn timeline_send_identity_ignores_absent_misleading_and_non_user_identities() {
        let cases = [
            (ExternalDriver::Pi, json!({"type":"message","meta":{"messageId":"message/client-send"},"message":{"role":"user","content":"hello"}})),
            (ExternalDriver::Omp, json!({"type":"custom","id":"delivery","customType":"st3.delivery","data":{"messageIds":["message/client-send"]}})),
            (ExternalDriver::Omp, json!({"type":"message","parentId":"delivery","message":{"role":"user","content":"hello"}})),
            (ExternalDriver::Pi, json!({"type":"message","message":{"role":"user","content":"message/client-send"}})),
            (ExternalDriver::Omp, json!({"type":"message","message":{"role":"user","meta":{"messageFilename":"legacy.md"},"content":"hello"}})),
            (ExternalDriver::Pi, json!({"type":"message","meta":{"messageId":"legacy.md"},"message":{"role":"user","content":"hello"}})),
            (ExternalDriver::Omp, json!({"type":"message","meta":{"messageFilename":"message/client-send"},"message":{"role":"assistant","content":"hello"}})),
            (ExternalDriver::Pi, json!({"type":"message","meta":{"messageFilename":"message/client-send"},"message":{"role":"toolResult","content":"hello"}})),
            (ExternalDriver::Claude, json!({"type":"assistant","message":{"content":"[st3-delivery:message/client-send]\nhello"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":"quoted [st3-delivery:message/client-send]\nhello"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":"[st3-delivery:message/client-send] incidental"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":"[st3-delivery:legacy.md]\nhello"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":"[st3-delivery:message/]\nhello"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":"[st3-delivery:message/has space]\nhello"}})),
            (ExternalDriver::Claude, json!({"type":"user","message":{"content":[{"type":"text","text":"hello"},{"type":"text","text":"[st3-delivery:message/client-send]\nquoted"}]}})),
            (ExternalDriver::Claude, json!({"type":"attachment","attachment":{"type":"other","prompt":"[st3-delivery:message/client-send]\nhello"}})),
            (ExternalDriver::Codex, json!({"type":"response_item","payload":{"type":"message","role":"user","id":"st:client","content":"hello"}})),
            (ExternalDriver::Codex, json!({"type":"response_item","clientId":"st:client","payload":{"type":"message","role":"user","content":"hello"}})),
            (ExternalDriver::Codex, json!({"type":"response_item","payload":{"type":"message","role":"assistant","clientId":"st:client","content":"hello"}})),
            (ExternalDriver::Codex, json!({"type":"response_item","payload":{"type":"message","role":"user","clientId":"","content":"hello"}})),
        ];
        for (driver, value) in cases {
            let mut entries = Vec::new();
            normalize_native_line(driver, &value, 16, &timestamp(0), &mut entries);
            assert!(!entries.is_empty());
            assert!(
                entries.iter().all(|entry| entry.get("_delivery_correlation").is_none()),
                "{driver:?}: {value}"
            );
        }
    }

    #[test]
    fn codex_claude_pi_and_omp_transcripts_normalize_to_one_timeline_shape() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        normalize_codex(
            &json!({"type":"response_item","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"message","role":"assistant","id":"m1","content":[{"type":"output_text","text":"codex"}]}}),
            0,
            &fallback,
            &mut items,
        );
        normalize_claude(
            &json!({"type":"assistant","timestamp":"2026-01-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"c1","name":"shell","input":{"command":"true"}}]}}),
            16,
            &fallback,
            &mut items,
        );
        normalize_omp(
            ExternalDriver::Omp,
            &json!({"type":"message","id":"m2","timestamp":"2026-01-01T00:00:02Z","message":{"role":"tool","content":[{"type":"toolResult","toolCallId":"c1","content":"ok"}]}}),
            32,
            &fallback,
            &mut items,
        );
        assert!(
            items
                .iter()
                .any(|item| item["type"] == "content" && item["body"]["text"] == "codex")
        );
        assert!(
            items
                .iter()
                .any(|item| item["type"] == "tool_call" && item["body"]["call_id"] == "c1")
        );
        assert!(
            items
                .iter()
                .any(|item| item["type"] == "tool_result" && item["body"]["call_id"] == "c1")
        );
    }

    // Native OMP call/result pairs captured on 2026-10-02, with tool input/output redacted.
    fn omp_tool_result_fixture() -> Vec<Value> {
        include_str!("../../st3-daemon/fixtures/omp-tool-results.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn native_images_keep_original_sources_for_owner_fetch() {
        let part = json!({"type":"image","data":"data:image/png;base64,AAAA","mimeType":"image/png","width":70000});
        for driver in [ExternalDriver::Omp, ExternalDriver::Pi] {
            let mut items = Vec::new();
            normalize_omp(
                driver,
                &json!({"type":"message","id":"image","message":{"role":"user","content":[part.clone()]}}),
                0,
                "",
                &mut items,
            );
            let block = &items[1]["body"]["blocks"][0];
            assert_eq!(block["kind"], "image");
            assert_eq!(block["payload"], part);
            assert_eq!(items[1]["body"]["text"], "[image · load from owner]");
        }
    }

    #[test]
    fn omp_managed_stop_fixture_preserves_the_complete_checkpoint() {
        let mut items = Vec::new();
        let mut checkpoint = Value::Null;
        for (offset, line) in include_str!("../../st3-daemon/fixtures/omp-resume/managed-stop-cancelled.jsonl")
            .lines()
            .enumerate()
        {
            let entry: Value = serde_json::from_str(line).unwrap();
            if entry["customType"] == "session_exit" {
                checkpoint = entry["data"].clone();
            }
            normalize_omp(
                ExternalDriver::Omp,
                &entry,
                offset as u64 * 16,
                "",
                &mut items,
            );
        }
        assert!(
            !items
                .iter()
                .any(|item| item["body"]["code"] == "native_process_exit")
        );
        let status = items.iter().find(|item| item["type"] == "status").unwrap();
        assert_eq!(status["body"]["status"], "cancelled");
        let details: Value =
            serde_json::from_str(status["body"]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(details, checkpoint);
        assert_eq!(
            details["pendingToolCalls"][0]["args"]["questions"][0]["question"],
            "Heal canary: which color?"
        );
        assert!(
            items
                .windows(2)
                .all(|pair| pair[0]["sequence"].as_u64() < pair[1]["sequence"].as_u64())
        );
    }

    #[test]
    fn omp_exit_checkpoint_preserves_open_native_values_without_normalizer_bounds() {
        let data = json!({
            "kind":"future-kind","reason":"Bearer planted-secret\n\u{1b}",
            "pendingToolCalls": (0..20).map(|_| json!({
                "toolCallId":99, "toolName":null, "args":{"intent":"x".repeat(32_768)}
            })).collect::<Vec<_>>(),
            "future":{"native":true},
        });
        let mut items = Vec::new();
        normalize_omp(
            ExternalDriver::Omp,
            &json!({
                "type":"custom","customType":"session_exit","data":data
            }),
            0,
            "",
            &mut items,
        );
        assert_eq!(items[0]["type"], "error");
        assert_eq!(items[0]["body"]["code"], "native_session_exit_unknown");
        assert_eq!(items[0]["body"]["details"], data);
        assert_eq!(items[0]["body"]["blocks"][0]["kind"], "error");
        assert_eq!(items[0]["body"]["blocks"][0]["payload"]["body_ref"], true);
    }

    #[test]
    fn omp_session_exits_classify_native_kinds_without_losing_details() {
        for (data, expected_code) in [
            (json!({"kind":"signal","reason":"sigterm"}), None),
            (json!({"kind":"signal","reason":"sigint"}), None),
            (json!({"kind":"signal","reason":"sighup"}), None),
            (json!({"kind":"normal","reason":"dispose"}), None),
            (json!({"kind":"normal","reason":"manual"}), None),
            (
                json!({"kind":"process_exit","reason":"exit","exitCode":0}),
                None,
            ),
            (json!({"kind":"exit","reason":"exit","code":0}), None),
            (
                json!({"kind":"fatal","reason":"uncaught_exception"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"fatal","reason":"unhandled_rejection"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"uncaughtException","reason":"exception"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"unhandledRejection","reason":"rejection"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"crash","reason":"crash"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"normal","reason":"uncaughtException"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"signal","reason":"unhandledRejection"}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"process_exit","reason":"exit","exitCode":1}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"exit","reason":"exit","code":2}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"normal","reason":"exit","exitCode":1}),
                Some("native_session_exit_fatal"),
            ),
            (
                json!({"kind":"process_exit","reason":"exit"}),
                Some("native_session_exit_unknown"),
            ),
            (
                json!({"kind":"signal","reason":"future-signal"}),
                Some("native_session_exit_unknown"),
            ),
            (
                json!({"kind":"future-kind","reason":"sigterm"}),
                Some("native_session_exit_unknown"),
            ),
            (
                json!({"reason":"dispose"}),
                Some("native_session_exit_unknown"),
            ),
            (json!({"kind":"normal"}), None),
            (
                json!({"kind":null,"reason":{"future":true}}),
                Some("native_session_exit_unknown"),
            ),
        ] {
            let mut data = data;
            data["pendingToolCalls"] =
                json!([{"toolName":"ask","args":{"future":[false,null,42]}}]);
            data["future"] = json!({"native":"unchanged"});
            let mut items = Vec::new();
            normalize_omp(
                ExternalDriver::Omp,
                &json!({"type":"custom","customType":"session_exit","data":data}),
                0,
                "",
                &mut items,
            );
            let entry = &items[0];
            assert_eq!(entry["role"], "system", "{data}");
            let block = &entry["body"]["blocks"][0];
            assert_eq!(block["payload"]["body_ref"], true, "{data}");
            assert_eq!(block["payload"]["raw"]["data"], data);
            if data["kind"].is_string() && data["reason"].is_string() {
                assert_eq!(block["view"]["type"], "session_exit");
                assert_eq!(block["view"]["kind"], data["kind"]);
                assert_eq!(block["view"]["reason"], data["reason"]);
            } else {
                assert!(block.get("view").is_none(), "{data}");
            }
            if let Some(code) = expected_code {
                assert_eq!(entry["type"], "error", "{data}");
                assert_eq!(entry["body"]["code"], code, "{data}");
                assert_eq!(entry["body"]["retryable"], false, "{data}");
                assert_eq!(entry["body"]["details"], data);
                assert_eq!(block["kind"], "error", "{data}");
            } else {
                assert_eq!(entry["type"], "status", "{data}");
                assert_eq!(entry["body"]["status"], "cancelled", "{data}");
                let detail: Value =
                    serde_json::from_str(entry["body"]["detail"].as_str().unwrap()).unwrap();
                assert_eq!(detail, data);
                assert_eq!(block["kind"], "status", "{data}");
            }
        }
    }

    #[test]
    fn omp_assistant_outcomes_preserve_exact_diagnostics_and_retry_recovery() {
        for stop in ["error", "aborted", "length", "future native stop\n\u{1b}"] {
            let message = json!({
                "role":"assistant","content":[],
                "stopReason":stop,"errorMessage":format!("provider\n\u{1b}{}", "x".repeat(32_768)),
                "errorStatus":"future-status","errorId":{"provider":99},
                "retryRecovery":{"attempt":2,"future":[false,null,"unchanged"]},
            });
            let mut items = Vec::new();
            normalize_omp(
                ExternalDriver::Omp,
                &json!({
                    "type":"message","message":message
                }),
                0,
                "",
                &mut items,
            );
            let errors = items
                .iter()
                .flat_map(|entry| entry["body"]["blocks"].as_array().into_iter().flatten())
                .filter(|block| block["source_type"] == "omp/assistant_error")
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 1, "{stop}");
            assert!(
                items.iter().all(|entry| entry["type"] != "error"),
                "assistant stop {stop} must not also produce a system error entry"
            );
            assert_eq!(
                items.iter()
                    .flat_map(|entry| entry["body"]["blocks"].as_array().into_iter().flatten())
                    .filter(|block| block["kind"] == "error")
                    .count(),
                1,
                "exactly one error block for assistant stop {stop}"
            );
            let error = errors[0];
            let view = &error["view"];
            assert_eq!(view["type"], "assistant_error");
            assert_eq!(view["presentation"], "full");
            assert_eq!(view["stop_reason"], message["stopReason"]);
            assert_eq!(view["message"], message["errorMessage"]);
            for key in [
                "stopReason",
                "errorMessage",
                "errorStatus",
                "errorId",
                "retryRecovery",
            ] {
                assert_eq!(view["native"][key], message[key], "{stop}/{key}");
            }
            assert_eq!(error["kind"], "error");
            assert_eq!(error["payload"], json!({"body_ref":true}));
        }
    }

    #[test]
    fn omp_length_and_unknown_stops_render_without_error_messages() {
        for stop in ["length", "future-stop"] {
            let view = omp_assistant_error_view(&json!({"stopReason": stop})).unwrap();
            assert_eq!(view["presentation"], "full");
            assert_eq!(view["is_error"], true);
            assert_eq!(view["native"]["stopReason"], stop);
            assert!(!view["label"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn omp_message_tool_result_success_correlates_with_call() {
        let fixture = omp_tool_result_fixture();
        let mut items = Vec::new();
        for (offset, entry) in fixture[..2].iter().enumerate() {
            normalize_omp(
                ExternalDriver::Omp,
                entry,
                offset as u64 * 16,
                "",
                &mut items,
            );
        }
        let call = items
            .iter()
            .find(|item| item["type"] == "tool_call")
            .unwrap();
        let result = items
            .iter()
            .find(|item| item["type"] == "tool_result")
            .unwrap();
        assert_eq!(result["body"]["call_id"], call["body"]["call_id"]);
        assert_eq!(result["body"]["status"], "success");
        assert_eq!(result["body"]["content"], fixture[1]["message"]["content"]);
        assert_eq!(result["timestamp"], fixture[1]["timestamp"]);
    }

    #[test]
    fn omp_message_tool_result_error_retains_status_and_text() {
        let fixture = omp_tool_result_fixture();
        let mut items = Vec::new();
        for (offset, entry) in fixture[2..].iter().enumerate() {
            normalize_omp(
                ExternalDriver::Omp,
                entry,
                offset as u64 * 16,
                "",
                &mut items,
            );
        }
        let call = items
            .iter()
            .find(|item| item["type"] == "tool_call")
            .unwrap();
        let result = items
            .iter()
            .find(|item| item["type"] == "tool_result")
            .unwrap();
        assert_eq!(result["body"]["call_id"], call["body"]["call_id"]);
        assert_eq!(result["body"]["status"], "error");
        assert_eq!(result["body"]["content"], fixture[3]["message"]["content"]);
    }

    #[test]
    fn pi_and_omp_tool_result_blocks_preserve_native_details_and_source_records() {
        for driver in [ExternalDriver::Omp, ExternalDriver::Pi] {
            for details in [
                json!({"wallTimeMs": 12.75, "timeoutSeconds": 0, "futureTimeNs": 0.125}),
                json!({"wallTimeMs": 0, "timeoutSeconds": 0.125, "future": {"unit": "ticks", "value": 4.5}}),
            ] {
                for nested_kind in [None, Some("toolResult"), Some("tool_result")] {
                    let mut entry = omp_tool_result_fixture()[1].clone();
                    entry["message"]["arguments"] = json!({"timeoutSeconds": 999});
                    if let Some(kind) = nested_kind {
                        entry["message"]
                            .as_object_mut()
                            .unwrap()
                            .remove("toolCallId");
                        entry["message"]["content"] = json!([{
                            "type": kind, "call_id": "nested",
                            "content": "finished", "details": details
                        }]);
                    } else {
                        entry["message"]["details"] = details.clone();
                    }
                    let mut items = Vec::new();
                    normalize_native_line(driver, &entry, 0, "", &mut items);
                    let result = items
                        .iter()
                        .find(|item| item["type"] == "tool_result")
                        .unwrap();
                    assert_eq!(result["body"]["blocks"][0]["kind"], "tool_output");
                    assert_eq!(result["body"]["blocks"][0]["metadata"], details);
                    assert!(result["body"].get("metadata").is_none());
                    let source = items
                        .iter()
                        .flat_map(|item| item["body"]["blocks"].as_array().unwrap())
                        .find(|block| block["kind"] == "source_record")
                        .unwrap();
                    assert_eq!(source["payload"]["raw"], entry);
                    assert!(source.get("metadata").is_none());
                }
            }
        }
    }

    #[test]
    fn native_tool_result_details_merge_without_inventing_absent_timing() {
        let mut entry = omp_tool_result_fixture()[1].clone();
        entry["message"].as_object_mut().unwrap().remove("details");
        entry["message"]["arguments"] = json!({"timeoutSeconds": 999});
        let mut items = Vec::new();
        normalize_native_line(ExternalDriver::Omp, &entry, 0, "", &mut items);
        assert!(
            items.last().unwrap()["body"]["blocks"][0]
                .get("metadata")
                .is_none()
        );
        items.last_mut().unwrap()["body"]["blocks"][0]["metadata"] =
            json!({"existing": true, "wallTimeMs": 999});
        entry["message"]["details"] =
            json!({"wallTimeMs": 0, "timeoutSeconds": 0.125, "future": "unchanged"});
        let source_before = items[0]["body"]["blocks"].clone();
        preserve_omp_result_timing(&mut items, &entry["message"]);
        assert_eq!(
            items.last().unwrap()["body"]["blocks"][0]["metadata"],
            json!({"existing": true, "wallTimeMs": 0, "timeoutSeconds": 0.125, "future": "unchanged"})
        );
        assert_eq!(items[0]["body"]["blocks"], source_before);
    }

    #[test]
    fn omp_failed_assistant_record_appends_typed_error_block() {
        let record = json!({
            "type": "message",
            "id": "err-one",
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {
                "role": "assistant",
                "content": [{"type": "thinking", "thinking": "synthetic planning note"}],
                "api": "openai-codex-responses",
                "provider": "openai-codex",
                "model": "gpt-5.6-sol",
                "stopReason": "error",
                "errorId": 135168,
                "errorMessage": "synthetic socket closed",
                "retryRecovery": {
                    "kind": "auto-retry",
                    "status": "recovered",
                    "attempt": 1,
                    "recoveredAt": "2026-01-01T00:00:20Z",
                    "recovery": "plain",
                    "note": "error; retried",
                    "supersededBy": {
                        "timestamp": 1789031579362i64,
                        "responseId": "resp_synthetic",
                        "provider": "openai-codex",
                        "model": "gpt-5.6-sol"
                    }
                }
            }
        });
        for (driver, source_type) in [
            (ExternalDriver::Omp, "omp/assistant_error"),
            (ExternalDriver::Pi, "pi/assistant_error"),
        ] {
            let mut items = Vec::new();
            normalize_omp(driver, &record, 16, "", &mut items);
            assert_eq!(items.len(), 3);
            // Prior content and its native block identity are untouched.
            assert_eq!(items[1]["type"], "content");
            assert_eq!(items[1]["body"]["blocks"][0]["id"], "native-17/0");
            let error = items.last().unwrap();
            assert_eq!(error["type"], "content");
            assert_eq!(error["role"], "assistant");
            assert_eq!(error["sequence"], 18);
            let block = &error["body"]["blocks"][0];
            assert_eq!(block["id"], "native-18/0");
            assert_eq!(block["kind"], "error");
            assert_eq!(block["source_type"], source_type);
            assert_eq!(block["payload"], json!({"body_ref": true}));
            assert_eq!(block["view"]["type"], "assistant_error");
            assert_eq!(block["view"]["status"], "recovered");
            assert_eq!(block["view"]["presentation"], "compact-recovered");
            assert_eq!(block["view"]["is_error"], false);
            assert_eq!(block["view"]["label"], "error; retried");
            assert_eq!(block["view"]["message"], "synthetic socket closed");
            assert_eq!(block["view"]["stop_reason"], "error");
            assert_eq!(block["view"]["error_id"], 135168);
            assert_eq!(block["view"]["api"], "openai-codex-responses");
            assert_eq!(block["view"]["provider"], "openai-codex");
            assert_eq!(block["view"]["model"], "gpt-5.6-sol");
            assert_eq!(block["view"]["retry"]["kind"], "auto-retry");
            assert_eq!(block["view"]["retry"]["status"], "recovered");
            assert_eq!(block["view"]["retry"]["attempt"], 1);
            assert_eq!(block["view"]["retry"]["recovery"], "plain");
            assert_eq!(block["view"]["retry"]["note"], "error; retried");
            assert_eq!(
                block["view"]["retry"]["recovered_at"],
                "2026-01-01T00:00:20Z"
            );
            assert_eq!(
                block["view"]["retry"]["superseded_by"],
                json!({
                    "timestamp": 1789031579362i64,
                    "response_id": "resp_synthetic",
                    "provider": "openai-codex",
                    "model": "gpt-5.6-sol"
                })
            );
            assert!(
                block["view"]["retry"]["superseded_by"]
                    .get("responseId")
                    .is_none()
            );
            assert_eq!(error["body"]["text"], "[retry recovered] error; retried");
        }
    }

    #[test]
    fn omp_terminal_assistant_error_renders_in_full() {
        let mut record = json!({
            "type": "message",
            "id": "err-two",
            "message": {
                "role": "assistant",
                "content": [],
                "stopReason": "error",
                "errorMessage": "synthetic boom"
            }
        });
        let mut items = Vec::new();
        normalize_omp(ExternalDriver::Omp, &record, 0, "", &mut items);
        let block = &items.last().unwrap()["body"]["blocks"][0];
        assert_eq!(block["view"]["status"], "failed");
        assert_eq!(block["view"]["presentation"], "full");
        assert_eq!(block["view"]["is_error"], true);
        assert_eq!(block["view"]["label"], "synthetic boom");
        assert_eq!(block["view"]["message"], "synthetic boom");
        assert!(block["view"]["retry"].is_null());
        assert!(block["view"]["provider"].is_null());
        assert_eq!(
            items.last().unwrap()["body"]["text"],
            "[error] synthetic boom"
        );

        record["message"]
            .as_object_mut()
            .unwrap()
            .remove("errorMessage");
        let mut items = Vec::new();
        normalize_omp(ExternalDriver::Omp, &record, 0, "", &mut items);
        assert_eq!(
            items.last().unwrap()["body"]["blocks"][0]["view"]["label"],
            "Error"
        );
    }

    #[test]
    fn omp_superseded_retry_error_is_typed_but_not_an_error() {
        let record = json!({
            "type": "message",
            "id": "err-three",
            "message": {
                "role": "assistant",
                "content": [],
                "stopReason": "error",
                "errorMessage": "synthetic rate limited",
                "retryRecovery": {
                    "kind": "auto-retry",
                    "status": "superseded",
                    "attempt": 2,
                    "recovery": "model",
                    "note": "switched model; retried"
                }
            }
        });
        let mut items = Vec::new();
        normalize_omp(ExternalDriver::Omp, &record, 0, "", &mut items);
        let block = &items.last().unwrap()["body"]["blocks"][0];
        assert_eq!(block["kind"], "error");
        assert_eq!(block["view"]["status"], "superseded");
        assert_eq!(block["view"]["presentation"], "none");
        assert_eq!(block["view"]["is_error"], false);
        assert_eq!(block["view"]["label"], "");
        assert_eq!(block["view"]["message"], "synthetic rate limited");
        assert_eq!(block["view"]["retry"]["status"], "superseded");
        assert_eq!(block["view"]["retry"]["attempt"], 2);
        assert!(block["view"]["retry"]["superseded_by"].is_null());
        assert_eq!(items.last().unwrap()["body"]["text"], "");
    }

    #[test]
    fn omp_aborted_assistant_errors_match_native_presentation_rules() {
        let cases = [
            (
                json!({"stopReason":"aborted","errorMessage":"__omp.silent_abort__"}),
                None,
            ),
            (
                json!({"stopReason":"aborted","errorMessage":"Interrupted by user"}),
                None,
            ),
            (
                json!({"stopReason":"aborted","errorId": 0x0400_0000i64, "errorMessage":"synthetic"}),
                None,
            ),
            (
                json!({"stopReason":"aborted","errorId": 0x0200_0000i64}),
                None,
            ),
            (json!({"stopReason":"aborted"}), Some("Operation aborted")),
            (
                json!({"stopReason":"aborted","errorMessage":"Request was aborted"}),
                Some("Operation aborted"),
            ),
            (
                json!({"stopReason":"aborted","errorMessage":"synthetic context limit"}),
                Some("synthetic context limit"),
            ),
            (
                json!({"errorMessage":"synthetic late failure"}),
                Some("synthetic late failure"),
            ),
            (json!({"stopReason":"stop"}), None),
            (json!({}), None),
        ];
        for (fields, expected) in cases {
            let record = json!({
                "type": "message",
                "id": "abort-one",
                "message": {"role": "assistant", "content": []}
            });
            let mut record = record;
            let message = record["message"].as_object_mut().unwrap();
            for (key, value) in fields.as_object().unwrap() {
                message.insert(key.clone(), value.clone());
            }
            let mut items = Vec::new();
            normalize_omp(ExternalDriver::Omp, &record, 0, "", &mut items);
            assert!(
                items.iter().all(|entry| entry["type"] != "error"),
                "no separate system error entry for {fields}"
            );
            assert_eq!(
                items.iter()
                    .flat_map(|entry| entry["body"]["blocks"].as_array().into_iter().flatten())
                    .filter(|block| block["kind"] == "error")
                    .count(),
                usize::from(expected.is_some()),
                "silent aborts and user interrupts emit no error block: {fields}"
            );
            match expected {
                None => assert_eq!(items.len(), 1, "no error block for {fields}"),
                Some(label) => {
                    let block = &items.last().unwrap()["body"]["blocks"][0];
                    assert_eq!(block["view"]["status"], "failed", "for {fields}");
                    assert_eq!(block["view"]["presentation"], "full", "for {fields}");
                    assert_eq!(block["view"]["is_error"], true, "for {fields}");
                    assert_eq!(block["view"]["label"], label, "for {fields}");
                }
            }
        }
    }

    #[test]
    fn omp_assistant_error_block_keeps_source_record_and_native_identity() {
        let record = json!({
            "type": "message",
            "id": "err-four",
            "message": {
                "role": "assistant",
                "content": "synthetic partial answer",
                "stopReason": "error",
                "errorMessage": "synthetic socket closed"
            }
        });
        let mut items = Vec::new();
        normalize_native_line(ExternalDriver::Omp, &record, 16, "", &mut items);
        // Source record stays on the first item; the error block is separate.
        assert_eq!(items[0]["body"]["blocks"][0]["kind"], "source_record");
        assert_eq!(items[0]["body"]["blocks"][0]["payload"]["raw"], record);
        assert_eq!(items[0]["body"]["blocks"][0]["visibility"], "internal");
        assert_eq!(items[1]["body"]["blocks"][0]["id"], "native-17/0");
        assert_eq!(items[1]["body"]["blocks"][0]["kind"], "text");
        let error = items.last().unwrap();
        assert_eq!(error["sequence"], 18);
        assert_eq!(error["body"]["blocks"][0]["kind"], "error");
        assert_eq!(
            error["body"]["blocks"][0]["source_type"],
            "omp/assistant_error"
        );
        assert_eq!(error["body"]["text"], "[error] synthetic socket closed");
    }

    #[test]
    fn omp_message_tool_result_legacy_block_remains_correlated() {
        let mut entry = omp_tool_result_fixture()[1].clone();
        let message = entry["message"].as_object_mut().unwrap();
        let call_id = message.remove("toolCallId").unwrap();
        let content = message.remove("content").unwrap();
        message.insert(
            "content".to_owned(),
            json!([{
                "type": "toolResult", "toolCallId": call_id, "content": content
            }]),
        );
        let mut items = Vec::new();
        normalize_omp(ExternalDriver::Omp, &entry, 0, "", &mut items);
        let result = items
            .iter()
            .find(|item| item["type"] == "tool_result")
            .unwrap();
        assert_eq!(result["body"]["call_id"], call_id);
        assert_eq!(result["body"]["status"], "success");
        assert_eq!(result["body"]["content"], content);
    }

    #[test]
    fn omp_native_capture_pairs_authoritative_result_and_preserves_irc_sender() {
        // Captured 2026-10-01; input, output and sender text redacted, native IDs retained.
        let fixture = include_str!("../../st3-daemon/fixtures/omp-native-events.jsonl")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let mut items = Vec::new();
        for (offset, entry) in fixture.iter().enumerate() {
            normalize_native_line(ExternalDriver::Omp, entry, offset as u64 * 16, "", &mut items);
        }
        let call = items.iter().find(|item| item["type"] == "tool_call").unwrap();
        let result = items.iter().find(|item| item["type"] == "tool_result").unwrap();
        assert_eq!(
            call["body"]["call_id"],
            fixture[0]["message"]["content"][0]["id"]
        );
        assert_eq!(result["body"]["call_id"], call["body"]["call_id"]);
        assert_eq!(result["body"]["status"], "error");
        assert_eq!(result["body"]["content"], fixture[1]["message"]["content"]);
        assert_eq!(result["body"]["blocks"][0]["view"]["is_error"], true);
        assert!(!items.iter().any(|item| item["role"] == "tool" && item["type"] == "content"));
        // The incoming IRC message keeps its sender in the typed view and its attribution in the
        // complete native record.
        let event = items
            .iter()
            .find(|item| item["body"]["blocks"][0]["view"]["type"] == "irc")
            .unwrap();
        let block = &event["body"]["blocks"][0];
        assert_eq!(block["kind"], "irc");
        assert_eq!(block["view"]["from"], fixture[2]["details"]["from"]);
        assert_eq!(block["view"]["message_id"], fixture[2]["details"]["id"]);
        assert_eq!(block["payload"]["raw"]["attribution"], fixture[2]["attribution"]);
        assert_eq!(block["payload"]["raw"], fixture[2]);
        assert_unique_ids(&items);
    }

    fn omp_event_blocks(entry: &Value) -> Vec<Value> {
        let mut items = Vec::new();
        normalize_native_line(ExternalDriver::Omp, entry, 0, "", &mut items);
        items
            .iter()
            .flat_map(|item| item["body"]["blocks"].as_array().cloned().unwrap_or_default())
            .collect()
    }

    #[test]
    fn omp_custom_events_preserve_native_provenance_and_hidden_visibility() {
        for entry in [
            json!({"type":"custom_message","customType":"resource-observed","display":true,"attribution":"daemon",
                "content":{"resource":"resource/example","status":"ready"},"details":{"revision":3}}),
            json!({"type":"custom_message","customType":"notice","attribution":"human","content":"Steer"}),
            json!({"type":"custom_message","customType":"notice","attribution":"system","content":"Note"}),
            json!({"type":"custom_message","customType":"notice","content":"Unattributed"}),
        ] {
            let blocks = omp_event_blocks(&entry);
            assert!(!blocks.is_empty(), "{entry}");
            let block = &blocks[0];
            // Main's block shape is kept: text content stays a `text` block.
            assert_ne!(block["kind"], "status", "{entry}");
            assert_eq!(block["payload"]["raw"], entry);
            let view = &block["view"];
            assert_eq!(view["type"], "omp_event");
            assert_eq!(view["kind"], entry["customType"]);
            // Attribution, content and details are the native values; an absent one stays
            // absent rather than being inferred (human, daemon and system are not agents).
            for field in ["attribution", "content", "details"] {
                assert_eq!(view.get(field), entry.get(field), "{field} of {entry}");
            }
            let mut hidden = entry.clone();
            hidden["display"] = json!(false);
            let mut hidden_items = Vec::new();
            normalize_native_line(ExternalDriver::Omp, &hidden, 0, "", &mut hidden_items);
            assert!(hidden_items.is_empty());
        }
    }

    #[test]
    fn omp_typed_custom_events_keep_attribution_and_job_identity_in_the_raw_record() {
        let entry = json!({"type":"custom_message","customType":"async-result","display":true,
            "attribution":"agent","content":"Task completed",
            "details":{"jobs":[{"type":"task","id":"task-17","label":"worker"}]}});
        let blocks = omp_event_blocks(&entry);
        assert_eq!(blocks[0]["view"]["type"], "job");
        assert_eq!(blocks[0]["view"]["jobs"][0]["id"], "task-17");
        assert_eq!(blocks[0]["payload"]["raw"], entry);
    }

    #[test]
    fn omp_oversized_custom_content_remains_decodable_with_sender_details() {
        let content = "x".repeat(MAX_TIMELINE_VALUE_BYTES + 1);
        let entry = json!({"type":"custom_message","customType":"notice","attribution":"agent",
            "content":content,"details":{"from":"worker","id":"message-1"}});
        let blocks = omp_event_blocks(&entry);
        let view = &blocks[0]["view"];
        assert_eq!(view["details"]["from"], "worker");
        assert_eq!(view["attribution"], "agent");
        assert!(
            view["content"]
                .as_str()
                .unwrap()
                .ends_with("[st truncated this native timeline value]")
        );
        // The bound applies to the view; the complete native record stays in the raw payload.
        assert_eq!(blocks[0]["payload"]["raw"]["content"], content);
        // Oversized details become marked text, not a corrupted view.
        let large = json!({"type":"custom_message","customType":"notice",
            "content":"short","details":{"rows":"y".repeat(MAX_TIMELINE_VALUE_BYTES)}});
        let view = &omp_event_blocks(&large)[0]["view"];
        assert!(view["details"].as_str().unwrap().ends_with("[st truncated this native timeline value]"));
        assert_eq!(view["content"], "short");
    }

    #[test]
    fn omp_event_field_bounds_preserve_inclusive_utf8_and_structured_values() {
        let limit = MAX_TIMELINE_VALUE_BYTES;
        let exact = json!("λ".repeat(limit / 2));
        assert_eq!(bounded_value(&exact, limit), exact);
        let wider = json!(format!("{}λ", exact.as_str().unwrap()));
        assert_eq!(
            bounded_value(&wider, limit).as_str().unwrap(),
            format!("{}\n[st truncated this native timeline value]", exact.as_str().unwrap())
        );
        let small = json!({"native":[false, 7, null]});
        assert_eq!(bounded_value(&small, limit), small);
        assert!(bounded_value(&json!({"native":"\"".repeat(limit)}), limit).is_string());
    }

    #[test]
    fn omp_wide_identities_are_bounded_and_do_not_disturb_other_events() {
        let wide = json!({"type":"custom_message","customType":"λ".repeat(MAX_TIMELINE_VALUE_BYTES),
            "attribution":"a".repeat(MAX_TIMELINE_VALUE_BYTES),"content":"wide identity"});
        let normal = json!({"type":"custom_message","customType":"notice","content":"normal"});
        let blocks = omp_event_blocks(&wide);
        let view = &blocks[0]["view"];
        for key in ["kind", "attribution"] {
            let value = view[key].as_str().unwrap();
            assert!(value.ends_with("[st truncated this native timeline value]"), "{key}");
            assert!(value.len() <= MAX_OMP_EVENT_IDENTITY_BYTES + 48, "{key}");
        }
        assert_eq!(view["content"], "wide identity");
        assert_eq!(blocks[0]["payload"]["raw"], wide);
        assert_eq!(omp_event_blocks(&normal)[0]["view"]["kind"], "notice");
    }

    #[test]
    fn omp_nested_tool_result_retains_error_status() {
        let mut items = Vec::new();
        normalize_native_line(
            ExternalDriver::Omp,
            &json!({"type":"message","message":{"role":"tool","content":[
                {"type":"toolResult","toolCallId":"failed-call","content":"failure","isError":true}
            ]}}),
            0,
            "",
            &mut items,
        );
        let result = items.iter().find(|item| item["type"] == "tool_result").unwrap();
        assert_eq!(result["body"]["call_id"], "failed-call");
        assert_eq!(result["body"]["status"], "error");
        assert_eq!(result["body"]["blocks"][0]["view"]["is_error"], true);
    }

    #[test]
    fn omp_observed_admin_records_are_typed_events_with_native_timestamps() {
        // Native 2026-10-04 shapes; all payload strings and numeric usage are redacted.
        for line in include_str!("../../st3-daemon/fixtures/omp-admin-events.jsonl").lines() {
            let entry: Value = serde_json::from_str(line).unwrap();
            let mut items = Vec::new();
            normalize_native_line(ExternalDriver::Omp, &entry, 16, "fallback", &mut items);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0]["role"], "system");
            assert_eq!(items[0]["timestamp"], entry["timestamp"]);
            let block = &items[0]["body"]["blocks"][0];
            assert_eq!(block["kind"], "status");
            assert_eq!(block["source_type"], entry["type"]);
            assert_eq!(block["payload"]["raw"], entry);
            let view = &block["view"];
            if entry["type"] == "title_change" {
                // Main's typed title view already covers this record.
                assert_eq!(view["type"], "title");
                assert_eq!(view["title"], entry["title"]);
            } else {
                assert_eq!(view["type"], "omp_event");
                assert_eq!(view["kind"], entry["type"]);
                assert!(view.get("attribution").is_none());
                assert!(view.get("content").is_none());
                assert_eq!(view["details"], entry);
            }
            assert!(block.get("visibility").is_none());

            // An explicit `display: false` is retained and marked, never dropped.
            let mut hidden = entry.clone();
            hidden["display"] = json!(false);
            let mut hidden_items = Vec::new();
            normalize_native_line(ExternalDriver::Omp, &hidden, 16, "", &mut hidden_items);
            assert_eq!(hidden_items.len(), 1);
            assert_eq!(
                hidden_items[0]["body"]["blocks"][0]["visibility"],
                "hidden-by-harness"
            );

            // Pi shares the format, and gets the same typed projection.
            let mut pi_items = Vec::new();
            normalize_native_line(ExternalDriver::Pi, &entry, 16, "", &mut pi_items);
            assert_eq!(pi_items[0]["body"]["blocks"][0]["view"], *view);
        }
    }

    #[test]
    fn omp_admin_events_do_not_make_hidden_bookkeeping_visible() {
        // Only the session header is omitted; the other bookkeeping is retained as itself and
        // never relabelled as an administrative event.
        let mut items = Vec::new();
        normalize_native_line(
            ExternalDriver::Omp,
            &json!({"type":"session","timestamp":"2026-10-01T00:00:00Z"}),
            0,
            "",
            &mut items,
        );
        assert!(items.is_empty());
        for kind in ["model_change", "custom", "credential_pin", "title"] {
            let blocks = omp_event_blocks(&json!({"type":kind,"timestamp":"2026-10-01T00:00:00Z"}));
            assert!(!blocks.is_empty(), "{kind}");
            assert!(blocks.iter().all(|block| block["view"]["type"] != "omp_event"), "{kind}");
        }
    }

    #[test]
    fn omp_admin_event_text_carries_its_bounded_record_for_clients_without_the_view() {
        for kind in ["model_usage", "session_init", "ttsr_injection"] {
            let mut items = Vec::new();
            normalize_native_line(
                ExternalDriver::Omp,
                &json!({"type":kind,"timestamp":"2026-10-01T00:00:00Z","task":"visible-detail"}),
                0,
                "2026-10-01T00:00:00Z",
                &mut items,
            );
            let text = items[0]["body"]["text"].as_str().unwrap();
            assert!(text.starts_with(&format!("[omp {kind}]")), "{kind}: {text}");
            assert!(text.contains("visible-detail"), "{kind}: {text}");
        }
    }

    #[test]
    fn omp_admin_event_missing_timestamp_uses_predecessor_timestamp() {
        let mut items = Vec::new();
        normalize_native_line(
            ExternalDriver::Omp,
            &json!({"type":"session_init","task":"fixture"}),
            0,
            "2026-10-01T00:00:00Z",
            &mut items,
        );
        assert_eq!(items[0]["timestamp"], "2026-10-01T00:00:00Z");
    }

    #[test]
    fn current_codex_custom_tools_are_visible_but_bootstrap_prompts_are_not() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        for (sequence, payload) in [
            (
                16,
                json!({"type":"message","role":"developer","id":"hidden","content":[{"type":"input_text","text":"private bootstrap"}]}),
            ),
            (
                32,
                json!({"type":"custom_tool_call","call_id":"call-one","name":"exec","input":"const task = 1;"}),
            ),
            (
                48,
                json!({"type":"custom_tool_call_output","call_id":"call-one","output":[{"type":"input_text","text":"done"}]}),
            ),
        ] {
            normalize_codex(
                &json!({"type":"response_item","timestamp":"2026-09-24T12:00:00Z","payload":payload}),
                sequence,
                &fallback,
                &mut items,
            );
        }
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "tool_call");
        assert_eq!(items[0]["body"]["call_id"], "call-one");
        assert_eq!(items[1]["type"], "tool_result");
        assert_eq!(items[1]["body"]["content"][0]["text"], "done");
    }

    #[test]
    fn unknown_claude_records_and_blocks_are_labelled_not_dropped_or_attributed() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        for (sequence, value) in [
            (
                16,
                json!({"type":"brand-new-kind","timestamp":"2026-09-30T10:00:00Z","detail":"x"}),
            ),
            (32, json!({"type":"file-history-snapshot","snapshot":{}})),
            (
                48,
                json!({"timestamp":"2026-09-30T10:00:00Z","note":"no type at all"}),
            ),
            (
                64,
                json!({"type":"assistant","timestamp":"2026-09-30T10:00:01Z","message":{"content":[
                    {"type":"thinking","thinking":"private"},
                    {"type":"hologram","payload":"new"},
                    {"type":"image","source":{"data":"AAAA"}},
                    {"type":"text","text":"visible"}
                ]}}),
            ),
            (
                80,
                json!({"type":"user","timestamp":"2026-09-30T10:00:02Z","message":{"content":[
                    {"type":"tool_result","tool_use_id":"call-1","is_error":true,"content":"boom"}
                ]}}),
            ),
        ] {
            normalize_claude(&value, sequence, &fallback, &mut items);
        }
        let labels = texts(&items);
        assert!(labels[0].starts_with("[unrecognized claude entry `brand-new-kind`]"));
        assert!(labels[2].starts_with("[unrecognized claude entry without a type]"));
        assert_eq!(labels[3], "[reasoning]\nprivate");
        assert!(labels[4].starts_with("[unrecognized claude content block `hologram`]"));
        assert_eq!(labels[5], "[image · load from owner]");
        assert_eq!(labels[6], "visible");
        assert_eq!(labels.len(), 7);
        assert!(labels[1].starts_with("[unrecognized claude entry `file-history-snapshot`]"));
        assert!(labels.iter().any(|text| text.contains("private")));
        for item in &items {
            if item["body"]["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("[unrecognized"))
            {
                assert_eq!(item["role"], "system");
            }
        }
        let result = items
            .iter()
            .find(|item| item["type"] == "tool_result")
            .unwrap();
        assert_eq!(result["body"]["status"], "error");
        assert_unique_ids(&items);
    }

    #[test]
    fn omp_developer_reminders_are_system_text_without_unknown_blocks() {
        let text = "<system-reminder>\nSynthetic incomplete todo reminder.\n</system-reminder>";
        let mut record = json!({
            "type": "message",
            "message": {
                "role": "developer",
                "attribution": {"source": "harness"},
                "timestamp": "2026-10-06T12:00:00Z",
                "content": [{"type": "text", "text": text}]
            }
        });
        let mut items = Vec::new();
        normalize_omp(ExternalDriver::Omp, &record, 16, &timestamp(0), &mut items);
        assert!(items.iter().all(|item| item["role"] == "system"));
        let blocks: Vec<_> = items
            .iter()
            .flat_map(|item| item["body"]["blocks"].as_array().into_iter().flatten())
            .collect();
        assert!(blocks.iter().all(|block| block["kind"] != "unknown"));
        assert!(blocks.iter().any(|block| block["kind"] == "text"));
        assert!(items.iter().any(|item| item["body"]["text"] == text));

        // Native attribution is handled exactly as it is for other OMP system messages.
        record["message"]["role"] = json!("system");
        let mut system_items = Vec::new();
        normalize_omp(
            ExternalDriver::Omp,
            &record,
            16,
            &timestamp(0),
            &mut system_items,
        );
        assert_eq!(items, system_items);
    }

    #[test]
    fn unknown_json_is_lossless_before_transport_bounding() {
        let value = json!({"type":"brand-new-kind","blob":"é".repeat(8192),"nested":{"token":"invented-token"}});
        let mut items = Vec::new();
        normalize_claude(&value, 16, &timestamp(0), &mut items);
        assert_eq!(items[0]["body"]["blocks"][0]["kind"], "unknown");
        assert_eq!(items[0]["body"]["blocks"][0]["payload"]["raw"], value);
    }

    #[test]
    fn claude_queued_prompts_and_system_notes_are_conversation() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        normalize_claude(
            &json!({"type":"attachment","uuid":"q1","timestamp":"2026-09-30T10:00:00Z","attachment":{"type":"queued_command","prompt":"a message delivered while busy"}}),
            16,
            &fallback,
            &mut items,
        );
        normalize_claude(
            &json!({"type":"attachment","timestamp":"2026-09-30T10:00:00Z","attachment":{"type":"total_tokens_reminder"}}),
            32,
            &fallback,
            &mut items,
        );
        normalize_claude(
            &json!({"type":"system","subtype":"compact_boundary","content":"Conversation compacted","timestamp":"2026-09-30T10:00:01Z"}),
            48,
            &fallback,
            &mut items,
        );
        normalize_claude(
            &json!({"type":"system","subtype":"turn_duration","durationMs":5,"timestamp":"2026-09-30T10:00:01Z"}),
            64,
            &fallback,
            &mut items,
        );
        let labels = texts(&items);
        assert_eq!(labels[0], "a message delivered while busy");
        assert!(labels[1].contains("total_tokens_reminder"));
        assert_eq!(labels[2], "Conversation compacted");
        assert!(labels[3].contains("turn_duration"));
        assert_eq!(items[1]["role"], "user");
        assert_eq!(items[2]["role"], "system");
    }

    #[test]
    fn codex_event_mirrors_and_unknown_records_are_retained_raw() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        for (sequence, value) in [
            (
                16,
                json!({"type":"event_msg","payload":{"type":"agent_message","message":"dup"}}),
            ),
            (
                32,
                json!({"type":"response_item","payload":{"type":"reasoning","summary":[]}}),
            ),
            (48, json!({"type":"new_record_kind","payload":{}})),
            (
                64,
                json!({"type":"response_item","payload":{"type":"web_search_call","action":{"query":"q"}}}),
            ),
            (
                80,
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[
                    {"type":"input_image","image_url":"data:"},
                    {"type":"input_hologram"},
                    {"type":"input_text","text":"typed"}
                ]}}),
            ),
        ] {
            normalize_codex(&value, sequence, &fallback, &mut items);
        }
        let labels = texts(&items);
        assert!(labels[1].starts_with("[reasoning]"));
        assert!(labels[2].starts_with("[unrecognized codex record `new_record_kind`]"));
        assert!(labels[3].starts_with("[unrecognized codex response item `web_search_call`]"));
        assert_eq!(labels[4], "[image · load from owner]");
        assert!(labels[5].starts_with("[unrecognized codex message part `input_hologram`]"));
        assert_eq!(labels[6], "typed");
        assert_eq!(labels.len(), 7);
        assert!(labels[0].contains("event_msg"));
        assert!(labels[0].contains("dup"));
        assert_unique_ids(&items);
    }

    #[test]
    fn omp_summaries_shell_commands_and_unknown_records_are_visible() {
        let fallback = timestamp(0);
        let mut items = Vec::new();
        for (sequence, value) in [
            (
                16,
                json!({"type":"model_change","id":"a","timestamp":"2026-09-30T10:00:00Z"}),
            ),
            (
                32,
                json!({"type":"compaction","id":"b","timestamp":"2026-09-30T10:00:00Z","summary":"earlier work"}),
            ),
            (
                48,
                json!({"type":"message","id":"c","timestamp":"2026-09-30T10:00:01Z","message":{"role":"bashExecution","command":"ls","output":"file"}}),
            ),
            (64, json!({"type":"future_kind","id":"d"})),
            (
                80,
                json!({"type":"message","id":"e","timestamp":"2026-09-30T10:00:02Z","message":{"role":"assistant","content":[
                    {"type":"thinking","thinking":"private"},
                    {"type":"sparkle"},
                    {"type":"text","text":"answer"}
                ]}}),
            ),
        ] {
            normalize_omp(ExternalDriver::Omp, &value, sequence, &fallback, &mut items);
        }
        let labels = texts(&items);
        assert!(labels[0].contains("model_change"));
        assert_eq!(labels[1], "[omp compaction]\nearlier work");
        assert_eq!(labels[2], "$ ls\nfile");
        assert!(labels[3].starts_with("[unrecognized omp entry `future_kind`]"));
        assert_eq!(labels[4], "[reasoning]\nprivate");
        assert!(labels[5].starts_with("[unrecognized omp content block `sparkle`]"));
        assert_eq!(labels[6], "answer");
        assert_eq!(labels.len(), 7);
        let shell = items
            .iter()
            .find(|item| item["body"]["text"] == "$ ls\nfile")
            .unwrap();
        assert_eq!(shell["role"], "system");
    }

    #[test]
    fn error_blocks_and_source_supplied_timing_fit_the_open_contract() {
        let mut entry = timeline_item(
            16,
            &timestamp(0),
            "system",
            "error",
            json!({"code":"native-stop","message":"provider stopped","retryable":true,"details":{"stopReason":"future","errorMessage":"invented-token"}}),
        );
        assert_eq!(entry["body"]["blocks"][0]["kind"], "error");
        assert_eq!(
            entry["body"]["blocks"][0]["payload"],
            json!({"body_ref":true})
        );
        let metadata =
            json!({"wallTimeMs":12.75,"timeoutSeconds":0,"future":{"value":"untouched"}});
        entry["body"]["blocks"][0]["metadata"] = metadata.clone();
        let decoded: st3_client::TimelineEntry = serde_json::from_value(entry).unwrap();
        let st3_client::TimelineBody::Error(body) = decoded.body else {
            panic!("known error envelope")
        };
        assert_eq!(body.blocks[0].metadata.as_ref(), Some(&metadata));
    }

}

#[cfg(any(test, feature = "test-support"))]
pub mod shared_tests_seats;
