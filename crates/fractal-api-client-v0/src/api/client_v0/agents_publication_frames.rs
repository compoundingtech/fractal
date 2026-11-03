//! Byte-budgeted frames of one immutable agents publication: a delta against the revision a
//! subscriber holds, or a complete roster split into whole-row snapshot chunks. Every frame is
//! sliced from the publication's one shared encoding; nothing here copies or re-encodes rows.

use std::ops::Range;
use std::sync::Arc;

use super::super::ClientSnapshot;

use std::collections::BTreeMap;
use sha2::{Digest, Sha256};
use crate::store::agents_publication::AgentsPublication;

pub(super) struct AgentsRowsSummary {
    epoch: uuid::Uuid,
    revision: u64,
    rows: BTreeMap<String, [u8; 32]>,
    row_ranges: Vec<Range<usize>>,
    id_ranges: Vec<Range<usize>>,
    metadata: Range<usize>,
}
impl AgentsRowsSummary {
    pub(super) fn epoch(&self) -> uuid::Uuid { self.epoch }
    pub(super) fn revision(&self) -> u64 { self.revision }
}
struct AgentsRowsDiff {
    base_revision: u64,
    upserts: Vec<usize>,
    upsert_bytes: usize,
    removes: Vec<u8>,
}
impl AgentsRowsDiff {
    fn base_revision(&self) -> u64 { self.base_revision }
    fn upsert_bytes(&self) -> usize { self.upsert_bytes }
    fn encoded_removes(&self) -> &[u8] { &self.removes[1..self.removes.len() - 1] }
    fn upserts(&self) -> impl Iterator<Item = usize> + '_ { self.upserts.iter().copied() }
}

// Weak summaries share indexing and cryptographic fingerprints across subscribers without
// retaining a publication, its encoded bytes, or even a summary after the last subscriber.
type SummaryKey = (uuid::Uuid, u64);
type Summaries = BTreeMap<SummaryKey, std::sync::Weak<AgentsRowsSummary>>;
static SUMMARIES: std::sync::LazyLock<parking_lot::Mutex<Summaries>> =
    std::sync::LazyLock::new(Default::default);
/// Index into the core publication's shared canonical encoding; no copied row JSON.
struct IndexedPublication {
    publication: Arc<AgentsPublication>,
    summary: Arc<AgentsRowsSummary>,
}
impl IndexedPublication {
    fn new(publication: Arc<AgentsPublication>) -> Self {
        let key = (publication.metadata().node_epoch, publication.metadata().revision);
        let mut summaries = SUMMARIES.lock();
        if let Some(summary) = summaries.get(&key).and_then(std::sync::Weak::upgrade) {
            return Self { publication, summary };
        }
        summaries.retain(|_, summary| summary.strong_count() > 0);
        let bytes = publication.encoded();
        let metadata_start = b"{\"publication\":".len();
        let metadata_end = value_end(bytes, metadata_start);
        let rows = array_ranges(bytes, metadata_end + b",\"items\":".len());
        let rows_end = rows.last().map_or(metadata_end + b",\"items\":[".len(), |range| range.end);
        let ids = array_ranges(bytes, rows_end + b"],\"order\":".len());
        let summary = Arc::new(AgentsRowsSummary {
            epoch: publication.metadata().node_epoch,
            revision: publication.metadata().revision,
            rows: publication.order().iter().zip(&rows).map(|(id, range)|
                (id.clone(), Sha256::digest(&bytes[range.clone()]).into())).collect(),
            row_ranges: rows,
            id_ranges: ids,
            metadata: metadata_start..metadata_end,
        });
        summaries.insert(key, Arc::downgrade(&summary));
        Self { publication, summary }
    }
    fn order(&self) -> &[String] { self.publication.order() }
    fn summary(&self) -> &Arc<AgentsRowsSummary> { &self.summary }
    fn encoded_metadata(&self) -> &[u8] { &self.publication.encoded()[self.summary.metadata.clone()] }
    fn span(&self, ranges: &[Range<usize>], rows: Range<usize>) -> &[u8] {
        if rows.is_empty() { return &[]; }
        &self.publication.encoded()[ranges[rows.start].start..ranges[rows.end - 1].end]
    }
    fn encoded_rows(&self, rows: Range<usize>) -> &[u8] { self.span(&self.summary.row_ranges, rows) }
    fn encoded_ids(&self, rows: Range<usize>) -> &[u8] { self.span(&self.summary.id_ranges, rows) }
    fn diff_from(&self, previous: &AgentsRowsSummary) -> Option<Arc<AgentsRowsDiff>> {
        if previous.epoch != self.summary.epoch || previous.revision >= self.summary.revision { return None; }
        let upserts: Vec<_> = self.order().iter().enumerate().filter_map(|(index, id)|
            (previous.rows.get(id) != self.summary.rows.get(id)).then_some(index)).collect();
        let upsert_bytes = upserts.iter().map(|index| self.summary.row_ranges[*index].len()).sum::<usize>()
            + upserts.len().saturating_sub(1);
        let removes: Vec<_> = previous.rows.keys().filter(|id| !self.summary.rows.contains_key(*id)).collect();
        let removes = serde_json::to_vec(&removes).expect("IDs serialize");
        Some(Arc::new(AgentsRowsDiff { base_revision: previous.revision, upserts, upsert_bytes, removes }))
    }
}

// Canonical serde JSON has no insignificant whitespace. Find complete values without decoding
// or allocating their strings; escaped quotes and nested arrays/objects remain opaque.
fn value_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0_usize;
    let mut string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        if string {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { string = false; if depth == 0 { return index + 1; } }
        } else {
            match byte {
                b'"' => string = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth > 0 => { depth -= 1; if depth == 0 { return index + 1; } }
                b',' | b']' | b'}' if depth == 0 => return index,
                _ => {}
            }
        }
    }
    bytes.len()
}
fn array_ranges(bytes: &[u8], start: usize) -> Vec<Range<usize>> {
    debug_assert_eq!(bytes[start], b'[');
    let mut position = start + 1;
    let mut ranges = Vec::new();
    while bytes[position] != b']' {
        let end = value_end(bytes, position);
        ranges.push(position..end);
        position = end;
        if bytes[position] == b',' { position += 1; }
    }
    ranges
}


/// What a subscriber was last sent completely. It retains no rows and no publication.
pub(super) type Delivered = Arc<AgentsRowsSummary>;

const SNAPSHOT_HEAD: &str = r#"{"kind":"snapshot","id":"#;
const CHANGES_HEAD: &str = r#"{"kind":"changes","id":"#;
const SNAPSHOT_FIELD: &str = r#","collection":"agents","snapshot":"#;
const PUBLICATION_FIELD: &str = r#","publication":"#;
const CHUNK_INDEX_FIELD: &str = r#","chunk_index":"#;
const CHUNK_COUNT_FIELD: &str = r#","chunk_count":"#;
const ITEMS_FIELD: &str = r#","items":["#;
const BASE_REVISION_FIELD: &str = r#","base_revision":"#;
const UPSERTS_FIELD: &str = r#","upserts":["#;
const REMOVES_FIELD: &str = r#"],"removes":["#;
const ORDER_FIELD: &str = r#"],"order":["#;
const TAIL: &str = r#"],"has_more":false}"#;

const SNAPSHOT_FIXED: usize = SNAPSHOT_HEAD.len() + SNAPSHOT_FIELD.len() + PUBLICATION_FIELD.len()
    + CHUNK_INDEX_FIELD.len() + CHUNK_COUNT_FIELD.len() + ITEMS_FIELD.len() + ORDER_FIELD.len() + TAIL.len();
const CHANGES_FIXED: usize = CHANGES_HEAD.len() + SNAPSHOT_FIELD.len() + PUBLICATION_FIELD.len()
    + BASE_REVISION_FIELD.len() + UPSERTS_FIELD.len() + REMOVES_FIELD.len() + ORDER_FIELD.len() + TAIL.len();

/// A complete row, or the envelope of an empty roster, cannot fit in one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowTooLarge {
    /// The first row that cannot fit; `None` when the envelope alone exceeds the budget.
    pub(super) row_id: Option<String>,
    /// The smallest frame that would carry it.
    pub(super) bytes: usize,
    pub(super) budget: usize,
}

#[cfg(test)]
impl RowTooLarge {
    /// The permanent error frame for subscription `id`.
    pub(super) fn frame(&self, id: &str) -> String {
        serde_json::json!({
            "kind": "error", "id": id, "collection": "agents",
            "code": "agents-publication-row-too-large",
            "message": "a complete agents row cannot fit in the frame budget",
            "retryable": false,
        }).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameError {
    OutOfRange { index: usize, count: usize },
    /// A different subscription ID than the one prepared for made the frame too large.
    OverBudget { bytes: usize, budget: usize },
}

/// One target revision's frames for one subscriber. It pins exactly the target publication,
/// never the base, and preflighted every frame's exact length before the first is encoded.
pub(super) struct Prepared {
    publication: Arc<IndexedPublication>,
    encoded_snapshot: Box<str>,
    /// Encoded length of the subscription ID the lengths below were computed with.
    id_bytes: usize,
    byte_budget: usize,
    plan: Plan,
}

enum Plan {
    Snapshot(Box<[Chunk]>),
    Changes { diff: Arc<AgentsRowsDiff>, len: usize },
}

struct Chunk {
    rows: Range<usize>,
    len: usize,
}

/// Plan `publication` for a subscriber holding `previous`: a delta when `previous` is an earlier
/// revision of the same epoch and the delta fits, otherwise complete snapshot chunks.
pub(super) fn prepare(
    publication: Arc<AgentsPublication>,
    snapshot: ClientSnapshot,
    id: &str,
    previous: Option<&Delivered>,
    byte_budget: usize,
) -> Result<Prepared, RowTooLarge> {
    let publication = Arc::new(IndexedPublication::new(publication));
    let encoded_snapshot: Box<str> = serde_json::to_string(&snapshot).expect("a snapshot serializes to JSON").into();
    let id_bytes = encoded_len(id);
    let shared = id_bytes + encoded_snapshot.len() + publication.encoded_metadata().len();
    let count = publication.order().len();
    let delta = previous.and_then(|previous| publication.diff_from(previous)).and_then(|diff| {
        let len = CHANGES_FIXED + shared + digits(diff.base_revision()) + diff.upsert_bytes()
            + diff.encoded_removes().len() + publication.encoded_ids(0..count).len();
        (len <= byte_budget).then_some(Plan::Changes { diff, len })
    });
    let plan = match delta {
        Some(plan) => plan,
        None => Plan::Snapshot(partition(&publication, SNAPSHOT_FIXED + shared, byte_budget)?),
    };
    Ok(Prepared { publication, encoded_snapshot, id_bytes, byte_budget, plan })
}

/// Greedy whole-row chunks, sized with conservative index/count widths and then checked exactly.
fn partition(publication: &IndexedPublication, fixed: usize, budget: usize) -> Result<Box<[Chunk]>, RowTooLarge> {
    let count = publication.order().len();
    let bound = fixed + digits_usize(count.saturating_sub(1)) + digits_usize(count.max(1));
    let span = |rows: Range<usize>| publication.encoded_rows(rows.clone()).len() + publication.encoded_ids(rows).len();
    if count == 0 {
        if bound > budget { return Err(RowTooLarge { row_id: None, bytes: bound, budget }); }
        return Ok(vec![Chunk { rows: 0..0, len: fixed + 2 }].into_boxed_slice());
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < count {
        let single = bound + span(start..start + 1);
        if single > budget {
            return Err(RowTooLarge { row_id: Some(publication.order()[start].clone()), bytes: single, budget });
        }
        let mut end = start + 1;
        while end < count && bound + span(start..end + 1) <= budget { end += 1; }
        ranges.push(start..end);
        start = end;
    }
    let chunk_count = ranges.len();
    ranges.into_iter().enumerate().map(|(index, rows)| {
        let len = fixed + digits_usize(index) + digits_usize(chunk_count) + span(rows.clone());
        if len > budget {
            return Err(RowTooLarge { row_id: Some(publication.order()[rows.start].clone()), bytes: len, budget });
        }
        Ok(Chunk { rows, len })
    }).collect()
}

impl Prepared {
    #[cfg(test)]
    pub(super) fn publication(&self) -> &Arc<AgentsPublication> { &self.publication.publication }

    /// The cursor to keep once every frame was sent.
    pub(super) fn delivered(&self) -> Delivered { Arc::clone(self.publication.summary()) }

    pub(super) fn frame_count(&self) -> usize {
        match &self.plan { Plan::Snapshot(chunks) => chunks.len(), Plan::Changes { .. } => 1 }
    }

    #[cfg(test)]
    /// `Some` when the single frame is a delta from this revision.
    pub(super) fn base_revision(&self) -> Option<u64> {
        match &self.plan { Plan::Snapshot(_) => None, Plan::Changes { diff, .. } => Some(diff.base_revision()) }
    }

    /// The exact encoded length of frame `index` for the prepared subscription ID.
    pub(super) fn frame_len(&self, index: usize) -> Result<usize, FrameError> {
        match &self.plan {
            Plan::Snapshot(chunks) => chunks.get(index).map(|chunk| chunk.len)
                .ok_or(FrameError::OutOfRange { index, count: chunks.len() }),
            Plan::Changes { len, .. } if index == 0 => Ok(*len),
            Plan::Changes { .. } => Err(FrameError::OutOfRange { index, count: 1 }),
        }
    }

    /// Encode frame `index` on demand; at most one frame buffer exists per call.
    pub(super) fn encode_frame(&self, index: usize, id: &str) -> Result<String, FrameError> {
        let encoded_id = serde_json::to_string(id).expect("an ID serializes to JSON");
        let len = self.frame_len(index)? - self.id_bytes + encoded_id.len();
        if len > self.byte_budget { return Err(FrameError::OverBudget { bytes: len, budget: self.byte_budget }); }
        let publication = &*self.publication;
        let mut frame = Vec::with_capacity(len);
        let all = 0..publication.order().len();
        match &self.plan {
            Plan::Snapshot(chunks) => {
                let chunk = &chunks[index];
                self.envelope(&mut frame, SNAPSHOT_HEAD, &encoded_id);
                frame.extend_from_slice(CHUNK_INDEX_FIELD.as_bytes());
                serde_json::to_writer(&mut frame, &index).expect("a chunk index serializes to JSON");
                frame.extend_from_slice(CHUNK_COUNT_FIELD.as_bytes());
                serde_json::to_writer(&mut frame, &chunks.len()).expect("a chunk count serializes to JSON");
                frame.extend_from_slice(ITEMS_FIELD.as_bytes());
                frame.extend_from_slice(publication.encoded_rows(chunk.rows.clone()));
                frame.extend_from_slice(ORDER_FIELD.as_bytes());
                frame.extend_from_slice(publication.encoded_ids(chunk.rows.clone()));
            }
            Plan::Changes { diff, .. } => {
                self.envelope(&mut frame, CHANGES_HEAD, &encoded_id);
                frame.extend_from_slice(BASE_REVISION_FIELD.as_bytes());
                serde_json::to_writer(&mut frame, &diff.base_revision()).expect("a base revision serializes to JSON");
                frame.extend_from_slice(UPSERTS_FIELD.as_bytes());
                for (position, row) in diff.upserts().enumerate() {
                    if position > 0 { frame.push(b','); }
                    frame.extend_from_slice(publication.encoded_rows(row..row + 1));
                }
                frame.extend_from_slice(REMOVES_FIELD.as_bytes());
                frame.extend_from_slice(diff.encoded_removes());
                frame.extend_from_slice(ORDER_FIELD.as_bytes());
                frame.extend_from_slice(publication.encoded_ids(all));
            }
        }
        frame.extend_from_slice(TAIL.as_bytes());
        debug_assert_eq!(frame.len(), len, "a frame's preflighted length is exact");
        Ok(String::from_utf8(frame).expect("canonical JSON is UTF-8"))
    }

    fn envelope(&self, frame: &mut Vec<u8>, head: &str, encoded_id: &str) {
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(encoded_id.as_bytes());
        frame.extend_from_slice(SNAPSHOT_FIELD.as_bytes());
        frame.extend_from_slice(self.encoded_snapshot.as_bytes());
        frame.extend_from_slice(PUBLICATION_FIELD.as_bytes());
        frame.extend_from_slice(self.publication.encoded_metadata());
    }
}

fn encoded_len(id: &str) -> usize {
    serde_json::to_string(id).expect("an ID serializes to JSON").len()
}

fn digits(mut value: u64) -> usize {
    let mut digits = 1;
    while value >= 10 { value /= 10; digits += 1; }
    digits
}

fn digits_usize(value: usize) -> usize { digits(value as u64) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::agents_publication::{AgentsStatusWatermark, PresentedAgents};
    use crate::store::published_list::{Publication, PublicationId};
    struct Owner { epoch: uuid::Uuid }
    impl Default for Owner { fn default() -> Self { Self { epoch: uuid::Uuid::new_v4() } } }
    use serde_json::{Value, json};

    const MIB: usize = 1_048_576;

    fn snapshot() -> ClientSnapshot {
        serde_json::from_value(json!({
            "id": "snapshot/host-a/1842/2fc9", "host_id": "host/host-a", "store_index": 1842,
            "projection_version": "client-projection.v0", "created_at": "2026-10-10T12:00:00.000Z",
            "published_at": "2026-10-10T12:00:00.000Z",
        })).unwrap()
    }

    fn publish(owner: &Owner, marker: u64, rows: Vec<Value>) -> Arc<AgentsPublication> {
        let order = rows.iter().map(|row| row["id"].as_str().unwrap().to_owned()).collect();
        Arc::new(Publication { cut: marker, published_at_unix_ms: u128::from(marker), valid_until_unix_ms: None,
            rows: PresentedAgents::new(owner.epoch, PublicationId { list: 1, generation: 0, sequence: marker },
                AgentsStatusWatermark { store_index: marker, local_frontier: marker }, marker, rows, order) })
    }

    fn row(id: &str, payload: &str) -> Value {
        json!({"id": id, "kind": "agent", "revision": "claim/x", "name": payload, "state": "stopped",
            "reachability": "local", "runtime_ids": []})
    }

    fn snapshot_frame(id: &str, publication: &AgentsPublication, index: usize, count: usize, rows: &[Value]) -> Value {
        json!({
            "kind": "snapshot", "id": id, "collection": "agents", "snapshot": snapshot(),
            "publication": publication.metadata(), "chunk_index": index, "chunk_count": count,
            "items": rows, "order": rows.iter().map(|row| row["id"].clone()).collect::<Vec<_>>(), "has_more": false,
        })
    }

    /// Every frame, checked against its preflighted length and the budget.
    fn frames(prepared: &Prepared, id: &str, budget: usize) -> Vec<(usize, Value)> {
        (0..prepared.frame_count()).map(|index| {
            let frame = prepared.encode_frame(index, id).unwrap();
            assert_eq!(frame.len(), prepared.frame_len(index).unwrap());
            assert!(frame.len() <= budget);
            (frame.len(), serde_json::from_str(&frame).unwrap())
        }).collect()
    }

    /// Chunks are consecutive, consistent, and concatenate to exactly the complete roster.
    fn assert_complete_snapshot(prepared: &Prepared, id: &str, budget: usize) -> usize {
        let publication = prepared.publication();
        let frames = frames(prepared, id, budget);
        let mut items = Vec::new();
        for (index, (_, frame)) in frames.iter().enumerate() {
            let chunk = frame["items"].as_array().unwrap();
            assert_eq!(frame["chunk_index"], index);
            assert_eq!(frame["chunk_count"], frames.len());
            assert_eq!(*frame, snapshot_frame(id, publication, index, frames.len(), chunk));
            items.extend(chunk.iter().cloned());
        }
        assert_eq!(items, publication.rows());
        frames.len()
    }

    #[test]
    fn agents_frames_empty_roster_is_one_chunk() {
        let owner = Owner::default();
        let publication = publish(&owner, 1, Vec::new());
        let prepared = prepare(Arc::clone(&publication), snapshot(), "agents", None, MIB).unwrap();
        assert_eq!(prepared.frame_count(), 1);
        assert_eq!(prepared.base_revision(), None);
        assert_eq!(frames(&prepared, "agents", MIB)[0].1, snapshot_frame("agents", &publication, 0, 1, &[]));
        assert_eq!(frames(&prepared, "agents", MIB)[0].1["snapshot"]["published_at"], "2026-10-10T12:00:00.000Z");
        let tight = RowTooLarge { row_id: None, bytes: prepared.frame_len(0).unwrap(), budget: 100 };
        assert_eq!(prepare(publication, snapshot(), "agents", None, 100).err(), Some(tight));
    }

    #[test]
    fn agents_frames_split_large_escaped_multibyte_roster_into_whole_rows() {
        let owner = Owner::default();
        let payload = "é\"\\\n\u{1}😀".repeat(700);
        let rows = (0..250).map(|index| row(&format!("agent/{index}/\"ü"), &payload)).collect::<Vec<_>>();
        let publication = publish(&owner, 1, rows);
        assert!(publication.encoded().len() > MIB);
        let id = "sub\"é/\u{2028}";
        let prepared = prepare(publication, snapshot(), id, None, MIB).unwrap();
        assert!(assert_complete_snapshot(&prepared, id, MIB) > 1);
        // Greedy: no chunk could also take the next row.
        let frames = frames(&prepared, id, MIB);
        let publication = &prepared.publication;
        let mut next = 0;
        // Packing used the conservative widths of 250 rows: index 249, count 250.
        let widths = |index: usize| digits_usize(249) + digits_usize(250) - digits_usize(index) - digits_usize(frames.len());
        for (index, (len, frame)) in frames[..frames.len() - 1].iter().enumerate() {
            next += frame["items"].as_array().unwrap().len();
            let more = publication.encoded_rows(next..next + 1).len() + publication.encoded_ids(next..next + 1).len() + 2;
            assert!(len + widths(index) + more > MIB);
        }
    }

    #[test]
    fn agents_frames_count_and_index_widths_at_the_exact_byte_limit() {
        let owner = Owner::default();
        let rows = (0..100).map(|index| row(&format!("agent/{index:03}"), "same-size")).collect::<Vec<_>>();
        let publication = publish(&owner, 1, rows.clone());
        // The widest one-row chunk: index 99 of 100.
        let budget = snapshot_frame("agents", &publication, 99, 100, &rows[99..]).to_string().len();
        let prepared = prepare(Arc::clone(&publication), snapshot(), "agents", None, budget).unwrap();
        assert_eq!(assert_complete_snapshot(&prepared, "agents", budget), 100);
        assert_eq!(prepared.frame_len(99).unwrap(), budget);
        assert_eq!(prepared.frame_len(9).unwrap(), budget - 1);
        let short = prepare(publication, snapshot(), "agents", None, budget - 1).err().unwrap();
        assert_eq!(short.row_id.as_deref(), Some("agent/000"));
        assert_eq!(short.bytes, budget);
        assert_eq!(prepared.encode_frame(100, "agents").err(), Some(FrameError::OutOfRange { index: 100, count: 100 }));
        assert_eq!(prepared.encode_frame(99, "agents-longer").err(),
            Some(FrameError::OverBudget { bytes: budget + 7, budget }));
    }

    #[test]
    fn agents_frames_reject_one_oversized_row_with_its_id() {
        let owner = Owner::default();
        let rows = vec![row("agent/a", "A"), row("agent/huge", &"x".repeat(2 * MIB)), row("agent/c", "C")];
        let publication = publish(&owner, 1, rows);
        let error = prepare(publication, snapshot(), "mine", None, MIB).err().unwrap();
        assert_eq!(error.row_id.as_deref(), Some("agent/huge"));
        assert!(error.bytes > MIB);
        let frame: Value = serde_json::from_str(&error.frame("mine")).unwrap();
        assert_eq!(frame, json!({"kind":"error","id":"mine","collection":"agents",
            "code":"agents-publication-row-too-large",
            "message":"a complete agents row cannot fit in the frame budget","retryable":false}));
    }

    #[test]
    fn agents_frames_delta_is_exact_and_skips_bases() {
        let owner = Owner::default();
        let first = publish(&owner, 1, vec![row("a", "1"), row("b", "1"), row("c", "1")]);
        let _second = publish(&owner, 2, vec![row("a", "2"), row("c", "1")]);
        let target = publish(&owner, 3, vec![row("d", "1"), row("a", "3"), row("c", "1")]);
        let prepared = prepare(Arc::clone(&target), snapshot(), "agents", Some(&IndexedPublication::new(first.clone()).summary), MIB).unwrap();
        assert_eq!(prepared.frame_count(), 1);
        assert_eq!(prepared.base_revision(), Some(first.metadata().revision));
        assert_eq!(frames(&prepared, "agents", MIB)[0].1, json!({
            "kind":"changes","id":"agents","collection":"agents","snapshot":snapshot(),
            "publication":target.metadata(),"base_revision":first.metadata().revision,
            "upserts":[row("d", "1"), row("a", "3")],"removes":["b"],"order":["d","a","c"],"has_more":false,
        }));
        assert_eq!(prepared.delivered().revision(), target.metadata().revision);
        // A delivered base at or past the target is not a delta base.
        let same = prepare(Arc::clone(&target), snapshot(), "agents", Some(&IndexedPublication::new(target.clone()).summary), MIB).unwrap();
        assert_eq!(same.base_revision(), None);
    }

    #[test]
    fn agents_frames_metadata_only_revision_is_an_empty_delta() {
        let owner = Owner::default();
        let rows = vec![row("a", "1"), row("b", "1")];
        let base = publish(&owner, 1, rows.clone());
        let target = publish(&owner, 2, rows);
        assert_eq!(target.metadata().revision, base.metadata().revision + 1);
        let prepared = prepare(Arc::clone(&target), snapshot(), "agents", Some(&IndexedPublication::new(base.clone()).summary), MIB).unwrap();
        let frame = &frames(&prepared, "agents", MIB)[0].1;
        assert_eq!(frame["upserts"], json!([]));
        assert_eq!(frame["removes"], json!([]));
        assert_eq!(frame["order"], json!(["a", "b"]));
        assert_eq!(frame["publication"]["status_watermark"]["store_index"], 2);
        assert!(frame.get("chunk_index").is_none());
    }

    #[test]
    fn agents_frames_new_epoch_and_oversized_delta_are_full_snapshots() {
        let owner = Owner::default();
        let base = publish(&owner, 1, vec![row("a", "1")]);
        let owner = Owner::default();
        let other_epoch = publish(&owner, 2, vec![row("a", "1")]);
        let prepared = prepare(Arc::clone(&other_epoch), snapshot(), "agents", Some(&IndexedPublication::new(base.clone()).summary), MIB).unwrap();
        assert_eq!(prepared.base_revision(), None);
        assert_eq!(assert_complete_snapshot(&prepared, "agents", MIB), 1);
        let target = publish(&owner, 3, (0..20).map(|index| row(&format!("r{index}"), &"y".repeat(1000))).collect());
        let budget = 4096;
        let prepared = prepare(target, snapshot(), "agents", Some(&IndexedPublication::new(other_epoch.clone()).summary), budget).unwrap();
        assert_eq!(prepared.base_revision(), None);
        assert!(assert_complete_snapshot(&prepared, "agents", budget) > 1);
    }

    #[test]
    fn agents_frames_delivered_summary_does_not_pin_the_publication() {
        let owner = Owner::default();
        let publication = publish(&owner, 1, vec![row("a", &"z".repeat(10_000))]);
        let weak = Arc::downgrade(&publication);
        let prepared = prepare(publication, snapshot(), "agents", None, MIB).unwrap();
        let delivered = prepared.delivered();
        drop(prepared);
        assert!(weak.upgrade().is_none());
        assert_eq!(delivered.revision(), 1);
        assert_eq!(delivered.rows.keys().map(String::as_str).collect::<Vec<_>>(), ["a"]);
    }

}

#[cfg(test)]
mod shared_index_tests {
    use super::*;
    use crate::store::agents_publication::{AgentsStatusWatermark, PresentedAgents};
    use crate::store::published_list::{Publication, PublicationId};

    #[test]
    fn agents_frames_share_index_without_retaining_publication_or_encoded_rows() {
        let publication = Arc::new(Publication {
            cut: 1, published_at_unix_ms: 1, valid_until_unix_ms: None,
            rows: PresentedAgents::new(uuid::Uuid::new_v4(),
                PublicationId { list: 1, generation: 0, sequence: 1 },
                AgentsStatusWatermark { store_index: 1, local_frontier: 1 }, 1,
                vec![serde_json::json!({"id":"agent/a","text":"nested [ \"escaped\" ] λ"})],
                vec!["agent/a".into()]),
        });
        let weak = Arc::downgrade(&publication);
        let encoded = Arc::downgrade(publication.encoded());
        let first = IndexedPublication::new(publication.clone());
        let second = IndexedPublication::new(publication.clone());
        assert!(Arc::ptr_eq(first.summary(), second.summary()));
        let summary = first.summary().clone();
        drop(first);
        drop(second);
        drop(publication);
        assert!(weak.upgrade().is_none());
        assert!(encoded.upgrade().is_none());
        assert_eq!(summary.revision(), 1);
    }

    #[test]
    fn agents_frames_empty_index_locates_order_after_the_empty_items_array() {
        let publication = Arc::new(Publication {
            cut: 1, published_at_unix_ms: 1, valid_until_unix_ms: None,
            rows: PresentedAgents::new(uuid::Uuid::new_v4(),
                PublicationId { list: 1, generation: 0, sequence: 1 },
                AgentsStatusWatermark { store_index: 1, local_frontier: 1 }, 1,
                Vec::new(), Vec::new()),
        });
        let expected = serde_json::to_value(publication.metadata()).unwrap();
        let indexed = IndexedPublication::new(publication);
        assert!(indexed.summary.row_ranges.is_empty());
        assert!(indexed.summary.id_ranges.is_empty());
        assert!(indexed.encoded_rows(0..0).is_empty());
        assert!(indexed.encoded_ids(0..0).is_empty());
        assert_eq!(serde_json::from_slice::<serde_json::Value>(indexed.encoded_metadata()).unwrap(), expected);
    }
}
