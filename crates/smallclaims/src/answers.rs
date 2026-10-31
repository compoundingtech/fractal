//! What readers answer from each claim kind, summarized so two members can compare.
//!
//! Trimming can remove superseded claims, including a writer's newest claim when a slot spans
//! writers. Claim inventories are diagnostics, not proof of reader disagreement. The runtime
//! supplies folded reader values for retention slots; when both members supply such a reader,
//! its values replace the corresponding kind's inventory comparison.
//!
//! The summary is taken at a cut, so members compare the same history while new claims arrive.
//! A key counts only once its newest claim is older than the cut: a key with a newer claim is
//! still moving, and the same key is left out by every member that holds that claim. The cut
//! never selects an older claim of a key, since a trimmed member may no longer hold it.
//!
//! A key that gains a claim after the cut leaves the summary on each member when that claim
//! arrives, so two members that compute at different times can differ only in such keys. The
//! keys a narrowing lists therefore carry them too, marked `moving`, and a key moving on either
//! member is no difference. A kept fold reader's answers learn of those claims as they are
//! served: each subject with a claim at or after the cut by then moves, and the reader's digests
//! are counted again without it, so two members that computed hours apart still agree.
//!
//! `keys` counts the keys a kind's digest covers and `claims` the kind's claims older than the
//! cut. A member that trims holds fewer claims than one that does not, so callers compare
//! `claims` only between members that trim the same way and have finished the same trims; see
//! [`claims_compare`].
//!
//! Each key also falls in one of 256 buckets by a hash of the key, and each bucket has its own
//! digest, so a difference narrows to a bucket and then to the keys in it without listing a kind.
//!
//! A summary reads every claim of every kind on a reader connection, a page of claims per
//! statement, so no statement holds a read snapshot for long. On a store of three million claims
//! that is ten minutes or more, so it is taken once a cut and kept, never per request:
//! [`AnswersCache`] computes on a thread of its own, answers `pending` until it has the answer,
//! and keeps the answer beside the store, so a restart serves it rather than computing again.
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::store::{Store, canonical};

pub const ANSWERS_VERSION: u32 = 1;
pub const BUCKETS: usize = 256;

/// The answers one kind gives at a cut.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindAnswers {
    /// Keys whose newest claim is older than the cut.
    pub keys: u64,
    /// Claims older than the cut.
    pub claims: u64,
    /// SHA-256 over every counted key and its newest claim, in key order.
    pub digest: String,
    /// SHA-256 over every key and its newest claim older than the cut, in key order, the moving
    /// keys too. Members that both still hold those claims agree on it whenever they computed;
    /// empty from a member that does not compute it, and for fold readers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before_cut: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswersSummary {
    pub version: u32,
    pub cut_unix_ms: u128,
    /// Whether the member trims on its own (`[trim] local`), so a peer knows whether its
    /// `claims` counts compare.
    #[serde(default)]
    pub trims_locally: bool,
    /// The cut of the newest checkpoint the member finished trimming, and whether a trim of a
    /// later one is unfinished; see [`claims_compare`].
    #[serde(default, skip_serializing_if = "is_zero")]
    pub trimmed_cut_unix_ms: u128,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trim_unfinished: bool,
    pub kinds: BTreeMap<String, KindAnswers>,
}

fn is_zero(value: &u128) -> bool {
    *value == 0
}

/// Whether two summaries' `claims` counts compare: the members trim the same way, finished
/// trimming the same checkpoint, and have no later trim unfinished. A trim deletes claims older
/// than its cut, so a member that finished the day's trim before it computed counts fewer claims
/// than one that finished it after, though both answer the same.
pub fn claims_compare(ours: &AnswersSummary, theirs: &AnswersSummary) -> bool {
    ours.trims_locally == theirs.trims_locally
        && ours.trimmed_cut_unix_ms == theirs.trimmed_cut_unix_ms
        && !ours.trim_unfinished
        && !theirs.trim_unfinished
}

/// One key's answer: the newest claim of `kind` that `origin` wrote about `subject`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnswerKey {
    pub subject: String,
    pub origin: String,
    pub claim: String,
    /// The key's newest claim is at or after the cut: no digest counts it, and a member that has
    /// not yet received that claim still counts the older one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moving: bool,
}

/// One kind's bucket digests at a cut, with the keys of the buckets asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindBuckets {
    pub kind: String,
    pub cut_unix_ms: u128,
    /// One digest per bucket; an empty bucket's is empty.
    pub buckets: Vec<String>,
    /// The keys of the buckets asked for, in key order, with the moving ones.
    pub keys: Vec<AnswerKey>,
}

/// A kind whose answers differ between two summaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindDifference {
    pub kind: String,
    pub ours: Option<KindAnswers>,
    pub theirs: Option<KindAnswers>,
}

/// A key whose answer differs between two members; `None` where a member counts no answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyDifference {
    pub subject: String,
    pub origin: String,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

fn domain(hash: &mut Sha256, label: &str) {
    hash.update(b"st3-answers-v1\0");
    hash.update(label.as_bytes());
    hash.update([0]);
}

fn field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

/// The bucket a key falls in.
pub fn bucket(subject: &str, origin: &str) -> usize {
    let mut hash = Sha256::new();
    domain(&mut hash, "bucket");
    field(&mut hash, subject);
    field(&mut hash, origin);
    usize::from(hash.finalize()[0])
}

/// The claims that share one key's newest accepted time, and that time.
type Newest = (u128, Vec<String>);
/// A key's newest claims, and its newest claims older than the cut.
type KeyClaims = (Option<Newest>, Option<Newest>);

fn newer(newest: &mut Option<Newest>, accepted: u128, id: &str) {
    match newest {
        Some((at, ids)) if accepted <= *at => {
            if accepted == *at {
                ids.push(id.to_owned());
            }
        }
        _ => *newest = Some((accepted, vec![id.to_owned()])),
    }
}

/// The newest of claims that share one accepted time: claims accepted in the same millisecond go
/// in canonical order.
fn pick(connection: &Connection, mut ids: Vec<String>) -> Result<String> {
    if ids.len() == 1 {
        return Ok(ids.pop().expect("one claim"));
    }
    let mut keyed = ids
        .into_iter()
        .map(|id| canonical::claim_key(connection, &id))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort();
    Ok(keyed.pop().expect("claims share the time").5)
}

/// The claims a `record.repaired` replaced. A member that admitted one before the repair arrived,
/// or wrote it, keeps its row, and a member that received the repair first never admits it, so
/// no member counts them; see #4282.
fn repaired_claims(connection: &Connection) -> Result<BTreeSet<String>> {
    let mut statement = connection.prepare_cached("SELECT id FROM projection_digest_repaired_claims")?;
    let mut rows = statement.query([])?;
    let mut repaired = BTreeSet::new();
    while let Some(row) = rows.next()? {
        repaired.insert(row.get(0)?);
    }
    Ok(repaired)
}

/// Call `visit` with each key of `kind` and its newest claim, in key order, marked `moving` when
/// that claim is not older than `cut_unix_ms`, and with the key's newest claim older than the cut
/// if it holds one. Return how many claims of the kind are older than the cut. Claims a repair
/// replaced count nowhere.
fn each_answer(
    connection: &Connection,
    kind: &str,
    cut_unix_ms: u128,
    visit: impl FnMut(AnswerKey, Option<String>) -> Result<()>,
) -> Result<u64> {
    each_answer_in_pages(connection, kind, cut_unix_ms, PAGE, visit)
}

/// Claims read per statement. A statement holds its read snapshot until it finishes, and while
/// any snapshot is open the WAL cannot restart, so a statement reads one page in index order.
const PAGE: i64 = 10_000;

fn each_answer_in_pages(
    connection: &Connection,
    kind: &str,
    cut_unix_ms: u128,
    page: i64,
    mut visit: impl FnMut(AnswerKey, Option<String>) -> Result<()>,
) -> Result<u64> {
    let mut statement = connection.prepare_cached(
        "SELECT store_index, subject, origin, id, accepted_at_unix_ms FROM claims INDEXED BY claims_kind_index
         WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT ?3",
    )?;
    let repaired = repaired_claims(connection)?;
    let mut claims = 0;
    // Each key's claims. A key's claims can fall in different pages, so every key is finished
    // after the last page.
    let mut keys: BTreeMap<(String, String), KeyClaims> = BTreeMap::new();
    let mut after = 0_i64;
    loop {
        let mut rows = statement.query(rusqlite::params![kind, after, page])?;
        let mut read = 0;
        while let Some(row) = rows.next()? {
            read += 1;
            after = row.get(0)?;
            let id: String = row.get(3)?;
            if repaired.contains(&id) {
                continue;
            }
            let accepted: u128 = row
                .get::<_, String>(4)?
                .parse()
                .with_context(|| format!("claim {id} has an unreadable acceptance time"))?;
            let (newest, before) = keys.entry((row.get(1)?, row.get(2)?)).or_default();
            newer(newest, accepted, &id);
            if accepted < cut_unix_ms {
                claims += 1;
                newer(before, accepted, &id);
            }
        }
        if read < page {
            break;
        }
    }
    for ((subject, origin), (newest, before)) in keys {
        let Some((accepted, newest)) = newest else { continue };
        let before = before.map(|(_, ids)| pick(connection, ids)).transpose()?;
        let claim = match &before {
            Some(before) if accepted < cut_unix_ms => before.clone(),
            _ => pick(connection, newest)?,
        };
        visit(AnswerKey { subject, origin, claim, moving: accepted >= cut_unix_ms }, before)?;
    }
    Ok(claims)
}

/// Every kind this store holds a claim of, in order.
fn kinds(connection: &Connection) -> Result<Vec<String>> {
    // A skip scan of the kind index, one probe per kind, rather than a pass over every claim.
    let mut kinds = Vec::new();
    let mut next = connection.prepare_cached(
        "SELECT MIN(kind) FROM claims INDEXED BY claims_kind_index WHERE kind>?1",
    )?;
    loop {
        let after = kinds.last().cloned().unwrap_or_default();
        let mut rows = next.query([after])?;
        let Some(kind) = rows.next()?.map(|row| row.get::<_, Option<String>>(0)).transpose()?.flatten() else {
            return Ok(kinds);
        };
        drop(rows);
        kinds.push(kind);
    }
}

/// One kind's answers at a cut.
pub fn kind_answers(connection: &Connection, kind: &str, cut_unix_ms: u128) -> Result<KindAnswers> {
    let mut hash = Sha256::new();
    domain(&mut hash, kind);
    let mut before_hash = Sha256::new();
    domain(&mut before_hash, kind);
    let mut keys = 0;
    let claims = each_answer(connection, kind, cut_unix_ms, |key, before| {
        if let Some(before) = before {
            field(&mut before_hash, &key.subject);
            field(&mut before_hash, &key.origin);
            field(&mut before_hash, &before);
        }
        if key.moving {
            return Ok(());
        }
        keys += 1;
        field(&mut hash, &key.subject);
        field(&mut hash, &key.origin);
        field(&mut hash, &key.claim);
        Ok(())
    })?;
    Ok(KindAnswers {
        keys,
        claims,
        digest: hex::encode(hash.finalize()),
        before_cut: hex::encode(before_hash.finalize()),
    })
}

/// Every kind's answers at a cut. Each kind is read in its own statement, so no read holds one
/// snapshot for the whole pass.
pub fn summary(connection: &Connection, cut_unix_ms: u128) -> Result<AnswersSummary> {
    let mut kinds_answers = BTreeMap::new();
    for kind in kinds(connection)? {
        let answers = kind_answers(connection, &kind, cut_unix_ms)?;
        kinds_answers.insert(kind, answers);
    }
    Ok(AnswersSummary { version: ANSWERS_VERSION, cut_unix_ms, kinds: kinds_answers, ..AnswersSummary::default() })
}

/// Kinds named with this prefix are fold readers' answers, not claim kinds; see [`FoldReaders`].
pub const READER_PREFIX: &str = "reader/";

/// Readers that fold claims themselves rather than answer with one claim, such as a seat's
/// current harness state or its status history. A trim can keep every key's newest claim and
/// still change their answers by dropping an older claim they fold, so the summary covers them
/// too. Given a reader connection and the cut, they return each reader's answers by name: one
/// key per subject they answer for, `origin` empty and `claim` a digest of the answer, in key
/// order, and a `moving` key for each subject with a claim at or after the cut. The runtime that owns the readers sets them; see [`AnswersCache::set_readers`].
pub type FoldReaders = Arc<dyn Fn(&Connection, u128) -> Result<BTreeMap<String, Vec<AnswerKey>>> + Send + Sync>;

/// Explicit (reader kind, subject, slot) keys that moved after a kept summary's cut.
pub type ReaderMoves = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;
pub type ReaderMovement = Arc<dyn Fn(&Connection, &BTreeSet<String>, u128) -> Result<ReaderMoves> + Send + Sync>;

/// A fold reader's answers as a kind: its summary entry, and every key with its bucket digests.
pub fn reader_answers(name: &str, cut_unix_ms: u128, keys: Vec<AnswerKey>) -> (String, KindAnswers, KindBuckets) {
    let kind = format!("{READER_PREFIX}{name}");
    let mut hash = Sha256::new();
    domain(&mut hash, &kind);
    let mut hashes: Vec<Option<Sha256>> = (0..BUCKETS).map(|_| None).collect();
    for key in keys.iter().filter(|key| !key.moving) {
        let bucket_hash = hashes[bucket(&key.subject, &key.origin)].get_or_insert_with(|| {
            let mut hash = Sha256::new();
            domain(&mut hash, &kind);
            hash
        });
        for hash in [&mut hash, bucket_hash] {
            field(hash, &key.subject);
            field(hash, &key.origin);
            field(hash, &key.claim);
        }
    }
    let counted = keys.iter().filter(|key| !key.moving).count();
    let answers =
        KindAnswers { keys: counted as u64, claims: 0, digest: hex::encode(hash.finalize()), before_cut: String::new() };
    let buckets = KindBuckets {
        kind: kind.clone(),
        cut_unix_ms,
        buckets: hashes
            .into_iter()
            .map(|hash| hash.map(|hash| hex::encode(hash.finalize())).unwrap_or_default())
            .collect(),
        keys,
    };
    (kind, answers, buckets)
}

/// One kind's bucket digests at a cut, with the keys of `wanted` buckets.
pub fn kind_buckets(
    connection: &Connection,
    kind: &str,
    cut_unix_ms: u128,
    wanted: &BTreeSet<usize>,
) -> Result<KindBuckets> {
    let mut hashes: Vec<Option<Sha256>> = (0..BUCKETS).map(|_| None).collect();
    let mut keys = Vec::new();
    each_answer(connection, kind, cut_unix_ms, |key, _| {
        let index = bucket(&key.subject, &key.origin);
        if !key.moving {
            let hash = hashes[index].get_or_insert_with(|| {
                let mut hash = Sha256::new();
                domain(&mut hash, kind);
                hash
            });
            field(hash, &key.subject);
            field(hash, &key.origin);
            field(hash, &key.claim);
        }
        if wanted.contains(&index) {
            keys.push(key);
        }
        Ok(())
    })?;
    Ok(KindBuckets {
        kind: kind.to_owned(),
        cut_unix_ms,
        buckets: hashes
            .into_iter()
            .map(|hash| hash.map(|hash| hex::encode(hash.finalize())).unwrap_or_default())
            .collect(),
        keys,
    })
}

/// The kinds whose answers differ. `claims` counts differ also when `compare_claims` is set,
/// for two members that trim the same way.
pub fn differing_kinds(
    ours: &AnswersSummary,
    theirs: &AnswersSummary,
    compare_claims: bool,
) -> Vec<KindDifference> {
    // Fold readers compare only between members that both run them: a member on a build without
    // them has none, which is no disagreement about any answer.
    let runs_readers = |summary: &AnswersSummary| summary.kinds.keys().any(|kind| kind.starts_with(READER_PREFIX));
    let readers = runs_readers(ours) && runs_readers(theirs);
    let kinds = ours.kinds.keys().chain(theirs.kinds.keys()).collect::<BTreeSet<_>>();
    kinds
        .into_iter()
        .filter(|kind| readers || !kind.starts_with(READER_PREFIX))
        .filter(|kind| {
            let replacement = format!("{READER_PREFIX}slots/{kind}");
            !(ours.kinds.contains_key(&replacement) && theirs.kinds.contains_key(&replacement))
        })
        .filter(|kind| {
            !kind.starts_with("reader/slots/")
                || (ours.kinds.contains_key(*kind) && theirs.kinds.contains_key(*kind))
        })
        .filter_map(|kind| {
            let (a, b) = (ours.kinds.get(kind), theirs.kinds.get(kind));
            // A member that holds no claim of a kind answers as one that counts none of its keys.
            let answers = |side: Option<&KindAnswers>| {
                let side = side.cloned().unwrap_or_default();
                let digest = if side.keys == 0 { String::new() } else { side.digest };
                (side.keys, digest, compare_claims.then_some(side.claims))
            };
            // Members that computed at different times agree on the claims older than the cut.
            let (before_a, before_b) = (a.map(|a| a.before_cut.as_str()), b.map(|b| b.before_cut.as_str()));
            let before_cut_agrees =
                before_a.is_some_and(|before| !before.is_empty()) && before_a == before_b && answers(a).2 == answers(b).2;
            (answers(a) != answers(b) && !before_cut_agrees).then(|| KindDifference {
                kind: kind.clone(),
                ours: a.cloned(),
                theirs: b.cloned(),
            })
        })
        .collect()
}

/// The buckets whose digests differ.
pub fn differing_buckets(ours: &[String], theirs: &[String]) -> BTreeSet<usize> {
    (0..BUCKETS)
        .filter(|&index| ours.get(index) != theirs.get(index))
        .collect()
}

/// The keys whose answers differ, from both members' keys of the same buckets. A key moving on
/// either member is left out: the members differ only in when they computed.
pub fn differing_keys(ours: &[AnswerKey], theirs: &[AnswerKey]) -> Vec<KeyDifference> {
    let moving = ours
        .iter()
        .chain(theirs)
        .filter(|key| key.moving)
        .map(|key| (key.subject.clone(), key.origin.clone()))
        .collect::<BTreeSet<_>>();
    let side = |keys: &[AnswerKey]| {
        keys.iter()
            .filter(|key| !key.moving)
            .map(|key| ((key.subject.clone(), key.origin.clone()), key.claim.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let (ours, theirs) = (side(ours), side(theirs));
    ours.keys()
        .chain(theirs.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| !moving.contains(*key) && ours.get(*key) != theirs.get(*key))
        .map(|key| KeyDifference {
            subject: key.0.clone(),
            origin: key.1.clone(),
            ours: ours.get(key).cloned(),
            theirs: theirs.get(key).cloned(),
        })
        .collect()
}

impl Store {
    /// Every kind's answers at a cut; see the module documentation for its cost.
    pub fn answers_summary(&self, cut_unix_ms: u128) -> Result<AnswersSummary> {
        let connection = self.readers.get();
        // The newest trimmed cut and the newest unfinished one, before and after the pass, so a
        // trim that starts or finishes during it counts as unfinished.
        let trims = || -> Result<(i64, i64)> {
            Ok(connection.query_row(
                "SELECT COALESCE(MAX(CASE WHEN state='trimmed' THEN cut_unix_ms END), 0),
                        COALESCE(MAX(CASE WHEN state IN ('trimming', 'set-aside') THEN cut_unix_ms END), 0)
                 FROM checkpoints",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        };
        let before = trims()?;
        let mut summary = summary(&connection, cut_unix_ms)?;
        let (trimmed, unfinished) = trims()?;
        summary.trims_locally = self.trims_locally();
        summary.trimmed_cut_unix_ms = u128::try_from(trimmed)?;
        summary.trim_unfinished = unfinished > trimmed || before != (trimmed, unfinished);
        Ok(summary)
    }

    /// Answer `request` at once, computing what it asks. Only a small store can afford this;
    /// a daemon answers through an [`AnswersCache`].
    pub fn answer(&self, request: &AnswersRequest) -> Result<AnswersReply> {
        Ok(match &request.kind {
            None => AnswersReply { summary: Some(self.answers_summary(request.cut_unix_ms)?), ..AnswersReply::default() },
            Some(kind) => AnswersReply {
                buckets: Some(self.answer_buckets(kind, request.cut_unix_ms, &request.buckets)?),
                ..AnswersReply::default()
            },
        })
    }

    /// One kind's bucket digests at a cut, with the keys of `wanted` buckets.
    pub fn answer_buckets(
        &self,
        kind: &str,
        cut_unix_ms: u128,
        wanted: &BTreeSet<usize>,
    ) -> Result<KindBuckets> {
        kind_buckets(&self.readers.get(), kind, cut_unix_ms, wanted)
    }
}

/// What one member found when it compared its answers at a cut with a peer's: every kind that
/// differs, or none when they agree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswersCheck {
    pub peer: String,
    pub cut_unix_ms: u128,
    pub differences: Vec<KindReport>,
}

/// One kind that differs, with the keys that differ. `complete` is false when the check named
/// fewer keys than differ, or none because it narrowed only the first few kinds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindReport {
    pub kind: String,
    pub ours: Option<KindAnswers>,
    pub theirs: Option<KindAnswers>,
    pub keys: Vec<KeyDifference>,
    pub complete: bool,
}

/// What a member asks another: the summary at a cut, or one kind's bucket digests with the keys
/// of `buckets`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswersRequest {
    pub cut_unix_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub buckets: BTreeSet<usize>,
}

/// The answer to an [`AnswersRequest`]. `pending` says the member is still computing it: ask
/// again later.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswersReply {
    #[serde(default)]
    pub pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<AnswersSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buckets: Option<KindBuckets>,
}

impl AnswersReply {
    fn pending() -> Self {
        Self { pending: true, ..Self::default() }
    }
}

/// How long a claim kind's every key serves the requests that narrow it. A check that found a
/// difference narrows again after longer than this, so both members read the kind again.
pub const ANSWERS_FRESH: Duration = Duration::from_secs(20 * 60);
/// The kinds whose every key is kept at once, for the bucket requests that narrow a difference.
const KEPT_KINDS: usize = 8;

/// Answers computed off the request path: the summary at one cut, kept until the cut changes and
/// beside the store across restarts, and every key of the few kinds a check is narrowing, kept for
/// [`ANSWERS_FRESH`].
#[derive(Default)]
pub struct AnswersCache {
    state: Mutex<CacheState>,
    readers: Mutex<Option<FoldReaders>>,
    movement: Mutex<Option<ReaderMovement>>,
}

#[derive(Default)]
struct CacheState {
    summary: Option<AnswersSummary>,
    /// The newest claim the summary has seen admitted; see [`still_current`].
    summary_mark: i64,
    kinds: Vec<(Instant, KindBuckets)>,
    /// Every fold reader's keys at the summary's cut, kept apart so narrowing a claim kind never
    /// evicts them.
    readers: Vec<KindBuckets>,
    computing: BTreeSet<(u128, Option<String>)>,
}

enum Computed {
    Summary(AnswersSummary, Vec<KindBuckets>, i64),
    Kept(AnswersSummary, Vec<KindBuckets>, i64),
    Kind(KindBuckets),
}

/// The summary a store keeps beside it, with every fold reader's keys, the build that computed
/// them and the newest claim admitted when it started. A restart serves it while it is current;
/// a new build computes again, in case it answers differently.
#[derive(Serialize, Deserialize)]
struct Kept {
    build: String,
    summary: AnswersSummary,
    readers: Vec<KindBuckets>,
    mark: i64,
}

/// The store index of the newest claim admitted.
fn newest_claim(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row("SELECT COALESCE(MAX(store_index), 0) FROM claims", [], |row| row.get(0))?)
}

/// The newest claim admitted, unless a claim older than the cut was admitted after `mark`, so a
/// summary computed when `mark` was newest lacks it. A claim keeps the time its origin accepted
/// it, so a member that was offline replays claims older than the cut long after it; a claim
/// accepted after the cut only moves its key. The claims after `mark` are a short range of the
/// primary key, and the mark moves up each time nothing older arrived.
fn still_current(connection: &Connection, mark: i64, cut_unix_ms: u128) -> Result<Option<i64>> {
    let newest = newest_claim(connection)?;
    let late: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE store_index>?1 AND store_index<=?2
             AND CAST(accepted_at_unix_ms AS INTEGER)<?3)",
        rusqlite::params![mark, newest, i64::try_from(cut_unix_ms)?],
        |row| row.get(0),
    )?;
    Ok((!late).then_some(newest))
}

/// The subjects of `subjects` with a claim accepted at or after the cut by now. A fold reader's
/// key moves when any claim of its subject does, so a reader kept since before such a claim
/// arrived still counts the subject a member computing now leaves out. One index read a subject.
fn moved_since(connection: &Connection, subjects: &BTreeSet<String>, cut_unix_ms: u128) -> Result<BTreeSet<String>> {
    let mut newest = connection.prepare_cached(
        "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_accepted_index WHERE subject=?1
         ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
    )?;
    let mut moved = BTreeSet::new();
    for subject in subjects {
        let accepted: Option<String> = newest.query([subject])?.next()?.map(|row| row.get(0)).transpose()?;
        if accepted.is_some_and(|accepted| accepted.parse::<u128>().map_or(true, |accepted| accepted >= cut_unix_ms)) {
            moved.insert(subject.clone());
        }
    }
    Ok(moved)
}

/// Where `store` keeps its summary: beside its database, and nowhere for a store in memory.
fn kept_path(store: &Store) -> Option<std::path::PathBuf> {
    let mut path = store.path.clone().into_os_string();
    path.push(".answers.json");
    (!store.shared_memory).then(|| path.into())
}

/// The summary `store` kept at `cut`, if this build computed it as the store trims now.
fn kept(store: &Store, cut_unix_ms: u128) -> Option<Kept> {
    let kept: Kept = serde_json::from_slice(&std::fs::read(kept_path(store)?).ok()?).ok()?;
    (kept.build == crate::store::checkpoint_agreement::checkpoint_build()
        && kept.summary.cut_unix_ms == cut_unix_ms
        && kept.summary.trims_locally == store.trims_locally())
    .then_some(kept)
}

/// Keep `summary` beside `store`, replacing the summary it kept at an earlier cut.
fn keep(store: &Store, summary: &AnswersSummary, readers: &[KindBuckets], mark: i64) -> Result<()> {
    use std::io::Write as _;
    let Some(path) = kept_path(store) else { return Ok(()) };
    let kept = Kept {
        build: crate::store::checkpoint_agreement::checkpoint_build(),
        summary: summary.clone(),
        readers: readers.to_vec(),
        mark,
    };
    let mut written = path.clone().into_os_string();
    written.push(".new");
    // Readable by the store's owner only, as the store is; a mode applies only to a new file.
    let _ = std::fs::remove_file(&written);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&written).with_context(|| format!("writing {}", path.display()))?;
    file.write_all(&serde_json::to_vec(&kept)?).with_context(|| format!("writing {}", path.display()))?;
    std::fs::rename(&written, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

impl CacheState {
    /// The subjects the kept fold readers still count.
    fn counted_subjects(&self) -> BTreeSet<String> {
        self.readers
            .iter()
            .flat_map(|every| every.keys.iter().filter(|key| !key.moving).map(|key| key.subject.clone()))
            .collect()
    }

    /// Mark every key of a `moved` subject moving in the kept fold readers, and count each reader
    /// that had one again, in the summary as in its buckets.
    fn remark_moving(&mut self, moved: &BTreeSet<String>, slots: &ReaderMoves) {
        let Some(summary) = self.summary.as_mut() else { return };
        for every in &mut self.readers {
            let moves = |key: &AnswerKey| {
                if every.kind.starts_with("reader/slots/") {
                    slots.get(&every.kind).and_then(|keys| keys.get(&key.subject))
                        .is_some_and(|slots| slots.contains(&key.origin))
                } else {
                    moved.contains(&key.subject)
                }
            };
            if !every.keys.iter().any(|key| !key.moving && moves(key)) {
                continue;
            }
            for key in every.keys.iter_mut().filter(|key| moves(key)) {
                key.moving = true;
                key.claim.clear();
            }
            let name = every.kind.strip_prefix(READER_PREFIX).unwrap_or(&every.kind).to_owned();
            let (kind, answers, buckets) = reader_answers(&name, every.cut_unix_ms, std::mem::take(&mut every.keys));
            summary.kinds.insert(kind, answers);
            *every = buckets;
        }
    }
}

impl AnswersCache {
    /// Cover these fold readers' answers in every summary from now on.
    pub fn set_readers(&self, readers: FoldReaders) {
        *self.readers.lock().unwrap_or_else(PoisonError::into_inner) = Some(readers);
    }

    /// Track runtime-defined retention slots without moving unrelated slots of the same subject.
    pub fn set_reader_movement(&self, movement: ReaderMovement) {
        *self.movement.lock().unwrap_or_else(PoisonError::into_inner) = Some(movement);
    }

    /// Answer `request` from what is kept, or answer `pending` and start computing it on a thread
    /// of its own, unless one already does.
    pub fn answer<S>(self: &Arc<Self>, store: &Arc<S>, request: &AnswersRequest) -> AnswersReply
    where
        S: Borrow<Store> + Send + Sync + 'static,
    {
        let fresh = |at: &Instant| at.elapsed() < ANSWERS_FRESH;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.summary.as_ref().is_some_and(|summary| summary.cut_unix_ms == request.cut_unix_ms) {
            let mark = state.summary_mark;
            let counted = state.counted_subjects();
            drop(state);
            let store: &Store = (**store).borrow();
            let current = still_current(&store.readers.get(), mark, request.cut_unix_ms);
            let moved = moved_since(&store.readers.get(), &counted, request.cut_unix_ms);
            let slots = self.movement.lock().unwrap_or_else(PoisonError::into_inner).clone()
                .map(|movement| movement(&store.readers.get(), &counted, request.cut_unix_ms))
                .unwrap_or_else(|| Ok(ReaderMoves::new()));
            state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.summary.as_ref().is_some_and(|summary| summary.cut_unix_ms == request.cut_unix_ms) {
                match moved.and_then(|moved| Ok((moved, slots?))) {
                    Ok((moved, slots)) => state.remark_moving(&moved, &slots),
                    Err(error) => eprintln!("st3: answers at {}: moving keys could not be read: {error:#}", request.cut_unix_ms),
                }
            }
            match current {
                Ok(Some(newest)) => state.summary_mark = state.summary_mark.max(newest),
                Ok(None) => {
                    eprintln!(
                        "st3: answers at {}: a claim older than the cut arrived since they were computed; \
                         computing them again",
                        request.cut_unix_ms
                    );
                    state.summary = None;
                    state.readers.clear();
                }
                Err(error) => eprintln!("st3: answers at {} could not be checked: {error:#}", request.cut_unix_ms),
            }
        }
        match &request.kind {
            None => {
                if let Some(summary) = &state.summary
                    && summary.cut_unix_ms == request.cut_unix_ms
                {
                    return AnswersReply { summary: Some(summary.clone()), ..AnswersReply::default() };
                }
            }
            Some(kind) => {
                let summary_kept =
                    state.summary.as_ref().is_some_and(|summary| summary.cut_unix_ms == request.cut_unix_ms);
                let mut kept = state
                    .readers
                    .iter()
                    .filter(|_| summary_kept)
                    .chain(state.kinds.iter().filter(|(at, _)| fresh(at)).map(|(_, every)| every));
                if let Some(every) =
                    kept.find(|every| every.kind == *kind && every.cut_unix_ms == request.cut_unix_ms)
                {
                    let keys = every
                        .keys
                        .iter()
                        .filter(|key| request.buckets.contains(&bucket(&key.subject, &key.origin)))
                        .cloned()
                        .collect();
                    return AnswersReply {
                        buckets: Some(KindBuckets { keys, ..every.clone_without_keys() }),
                        ..AnswersReply::default()
                    };
                }
                if summary_kept && kind.starts_with(READER_PREFIX) {
                    // A reader this member does not run answers for nothing.
                    return AnswersReply {
                        buckets: Some(KindBuckets {
                            kind: kind.clone(),
                            cut_unix_ms: request.cut_unix_ms,
                            buckets: vec![String::new(); BUCKETS],
                            keys: Vec::new(),
                        }),
                        ..AnswersReply::default()
                    };
                }
            }
        }
        // A reader's keys come only with the summary that computes them.
        let reader = request.kind.as_deref().is_some_and(|kind| kind.starts_with(READER_PREFIX));
        let job = (request.cut_unix_ms, request.kind.clone().filter(|_| !reader));
        if !state.computing.insert(job.clone()) {
            return AnswersReply::pending();
        }
        let (cache, store) = (self.clone(), store.clone());
        let readers = self.readers.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let thread_job = job.clone();
        let spawned = std::thread::Builder::new().name("st3-answers".into()).spawn(move || {
            let (cut, kind) = thread_job;
            let started = Instant::now();
            let store: &Store = (*store).borrow();
            let computed = crate::task_cpu::blocking("replication.answers", || match &kind {
                None => {
                    if let Some(kept) = kept(store, cut) {
                        match still_current(&store.readers.get(), kept.mark, cut)? {
                            Some(newest) => return Ok(Computed::Kept(kept.summary, kept.readers, newest)),
                            None => eprintln!(
                                "st3: answers at {cut}: a claim older than the cut arrived since they were kept; \
                                 computing them again"
                            ),
                        }
                    }
                    let mark = newest_claim(&store.readers.get())?;
                    let mut summary = store.answers_summary(cut)?;
                    let mut every = Vec::new();
                    if let Some(readers) = &readers {
                        for (name, keys) in readers(&store.readers.get(), cut)? {
                            let (kind, answers, buckets) = reader_answers(&name, cut, keys);
                            summary.kinds.insert(kind, answers);
                            every.push(buckets);
                        }
                    }
                    Ok(Computed::Summary(summary, every, mark))
                }
                Some(kind) => store
                    .answer_buckets(kind, cut, &(0..BUCKETS).collect())
                    .map(Computed::Kind),
            });
            if let Ok(Computed::Summary(summary, readers, mark)) = &computed
                && let Err(error) = keep(store, summary, readers, *mark)
            {
                eprintln!("st3: answers at {cut} could not be kept: {error:#}");
            }
            let mut state = cache.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.computing.remove(&(cut, kind.clone()));
            match computed {
                Ok(Computed::Summary(summary, readers, mark)) => {
                    eprintln!(
                        "st3: answers at {cut} computed in {}s: {} kinds, {} of them fold readers",
                        started.elapsed().as_secs(),
                        summary.kinds.len(),
                        readers.len()
                    );
                    state.summary = Some(summary);
                    state.summary_mark = mark;
                    state.readers = readers;
                }
                Ok(Computed::Kept(summary, readers, mark)) => {
                    eprintln!("st3: answers at {cut} kept from an earlier run: {} kinds", summary.kinds.len());
                    state.summary = Some(summary);
                    state.summary_mark = mark;
                    state.readers = readers;
                }
                Ok(Computed::Kind(every)) => {
                    state.kinds.retain(|(_, kept)| kept.kind != every.kind);
                    state.kinds.push((Instant::now(), every));
                    if state.kinds.len() > KEPT_KINDS {
                        state.kinds.remove(0);
                    }
                }
                Err(error) => eprintln!(
                    "st3: answers for {} at {cut} failed: {error:#}",
                    kind.as_deref().unwrap_or("every kind")
                ),
            }
        });
        if let Err(error) = spawned {
            state.computing.remove(&job);
            eprintln!("st3: answers could not start: {error}");
        }
        AnswersReply::pending()
    }
}

impl KindBuckets {
    fn clone_without_keys(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            cut_unix_ms: self.cut_unix_ms,
            buckets: self.buckets.clone(),
            keys: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The columns a kind's answers read.
    fn fixture(claims: &[(u8, u8, u16)]) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE,
                     subject TEXT, kind TEXT, origin TEXT, accepted_at_unix_ms TEXT);
                 CREATE INDEX claims_kind_index ON claims(kind, store_index);
                 CREATE TABLE projection_digest_repaired_claims(id TEXT PRIMARY KEY);",
            )
            .unwrap();
        for (n, (subject, origin, at)) in claims.iter().enumerate() {
            // Distinct times, so no two claims of a key tie.
            let accepted = u64::from(*at) * 1_000 + n as u64;
            connection
                .execute(
                    "INSERT INTO claims(id, subject, kind, origin, accepted_at_unix_ms) VALUES (?1, ?2, 'example.state', ?3, ?4)",
                    rusqlite::params![format!("claim-{n}"), format!("item/{subject}"), format!("node-{origin}"), accepted.to_string()],
                )
                .unwrap();
        }
        connection
    }

    fn answers(connection: &Connection, cut: u128, page: i64) -> (u64, Vec<(AnswerKey, Option<String>)>) {
        let mut keys = Vec::new();
        let claims = each_answer_in_pages(connection, "example.state", cut, page, |key, before| {
            keys.push((key, before));
            Ok(())
        })
        .unwrap();
        (claims, keys)
    }

    proptest! {
        #[test]
        fn a_kind_read_in_pages_answers_as_it_does_in_one(
            claims in proptest::collection::vec((0u8..6, 0u8..3, 0u16..100), 0..60),
            cut in 0u128..110_000,
            page in 1i64..8,
        ) {
            let connection = fixture(&claims);
            prop_assert_eq!(answers(&connection, cut, page), answers(&connection, cut, PAGE));
        }
    }
}
