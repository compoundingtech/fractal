//! What readers answer from each claim kind, summarized so two members can compare.
//!
//! A member that trims on its own deletes claims its peers may still hold, so members no longer
//! hold the same claims and their envelope and claim digests stop agreeing. What must still agree
//! is what a reader answers. For every kind a trim may touch, the answer for one key, a subject
//! and the writer that wrote it, is the key's newest claim in canonical order; a trim never
//! deletes it. So for each kind this summarizes the newest claim of every key, and two members
//! whose summaries differ disagree about what some reader answers.
//!
//! The summary is taken at a cut, so members compare the same history while new claims arrive.
//! A key counts only once its newest claim is older than the cut: a key with a newer claim is
//! still moving, and the same key is left out by every member that holds that claim. The cut
//! never selects an older claim of a key, since a trimmed member may no longer hold it.
//!
//! `keys` counts the keys a kind's digest covers and `claims` the kind's claims older than the
//! cut. A member that trims holds fewer claims than one that does not, so callers compare
//! `claims` only between members that trim the same way.
//!
//! Each key also falls in one of 256 buckets by a hash of the key, and each bucket has its own
//! digest, so a difference narrows to a bucket and then to the keys in it without listing a kind.
//!
//! A summary reads every claim of every kind, one kind per statement, on a reader connection. On
//! a store of three million claims that is about a minute and a half, so it is taken once a day
//! and kept, never per request: [`AnswersCache`] computes on a thread of its own and answers
//! `pending` until it has the answer.
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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswersSummary {
    pub version: u32,
    pub cut_unix_ms: u128,
    /// Whether the member trims on its own (`[trim] local`), so a peer knows whether its
    /// `claims` counts compare.
    #[serde(default)]
    pub trims_locally: bool,
    pub kinds: BTreeMap<String, KindAnswers>,
}

/// One key's answer: the newest claim of `kind` that `origin` wrote about `subject`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnswerKey {
    pub subject: String,
    pub origin: String,
    pub claim: String,
}

/// One kind's bucket digests at a cut, with the keys of the buckets asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindBuckets {
    pub kind: String,
    pub cut_unix_ms: u128,
    /// One digest per bucket; an empty bucket's is empty.
    pub buckets: Vec<String>,
    /// The keys of the buckets asked for, in key order.
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

/// Call `visit` with each key of `kind` whose newest claim is older than `cut_unix_ms`, in key
/// order, and return how many claims of the kind are older than the cut.
fn each_answer(
    connection: &Connection,
    kind: &str,
    cut_unix_ms: u128,
    mut visit: impl FnMut(AnswerKey) -> Result<()>,
) -> Result<u64> {
    let mut statement = connection.prepare_cached(
        "SELECT subject, origin, id, accepted_at_unix_ms FROM claims INDEXED BY claims_kind_index
         WHERE kind=?1 ORDER BY subject, origin",
    )?;
    let mut rows = statement.query([kind])?;
    let mut claims = 0;
    // The current key, and the claims that share its newest accepted time.
    let mut current: Option<(String, String, u128, Vec<String>)> = None;
    let mut finish = |key: Option<(String, String, u128, Vec<String>)>| -> Result<()> {
        let Some((subject, origin, accepted, mut newest)) = key else { return Ok(()) };
        if accepted >= cut_unix_ms {
            return Ok(());
        }
        let claim = if newest.len() == 1 {
            newest.pop().expect("one claim")
        } else {
            // Claims accepted in the same millisecond go in canonical order.
            let mut keyed = newest
                .into_iter()
                .map(|id| canonical::claim_key(connection, &id))
                .collect::<Result<Vec<_>>>()?;
            keyed.sort();
            keyed.pop().expect("claims share the time").5
        };
        visit(AnswerKey { subject, origin, claim })
    };
    while let Some(row) = rows.next()? {
        let subject: String = row.get(0)?;
        let origin: String = row.get(1)?;
        let id: String = row.get(2)?;
        let accepted: u128 = row
            .get::<_, String>(3)?
            .parse()
            .with_context(|| format!("claim {id} has an unreadable acceptance time"))?;
        if accepted < cut_unix_ms {
            claims += 1;
        }
        match &mut current {
            Some((key_subject, key_origin, newest, ids))
                if *key_subject == subject && *key_origin == origin =>
            {
                if accepted > *newest {
                    *newest = accepted;
                    ids.clear();
                }
                if accepted == *newest {
                    ids.push(id);
                }
            }
            _ => finish(current.replace((subject, origin, accepted, vec![id])))?,
        }
    }
    finish(current)?;
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
    let mut keys = 0;
    let claims = each_answer(connection, kind, cut_unix_ms, |key| {
        keys += 1;
        field(&mut hash, &key.subject);
        field(&mut hash, &key.origin);
        field(&mut hash, &key.claim);
        Ok(())
    })?;
    Ok(KindAnswers { keys, claims, digest: hex::encode(hash.finalize()) })
}

/// Every kind's answers at a cut. Each kind is read in its own statement, so no read holds one
/// snapshot for the whole pass.
pub fn summary(connection: &Connection, cut_unix_ms: u128) -> Result<AnswersSummary> {
    let mut kinds_answers = BTreeMap::new();
    for kind in kinds(connection)? {
        let answers = kind_answers(connection, &kind, cut_unix_ms)?;
        kinds_answers.insert(kind, answers);
    }
    Ok(AnswersSummary { version: ANSWERS_VERSION, cut_unix_ms, trims_locally: false, kinds: kinds_answers })
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
    each_answer(connection, kind, cut_unix_ms, |key| {
        let index = bucket(&key.subject, &key.origin);
        let hash = hashes[index].get_or_insert_with(|| {
            let mut hash = Sha256::new();
            domain(&mut hash, kind);
            hash
        });
        field(hash, &key.subject);
        field(hash, &key.origin);
        field(hash, &key.claim);
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
    let kinds = ours.kinds.keys().chain(theirs.kinds.keys()).collect::<BTreeSet<_>>();
    kinds
        .into_iter()
        .filter_map(|kind| {
            let (a, b) = (ours.kinds.get(kind), theirs.kinds.get(kind));
            // A member that holds no claim of a kind answers as one that counts none of its keys.
            let answers = |side: Option<&KindAnswers>| {
                let side = side.cloned().unwrap_or_default();
                let digest = if side.keys == 0 { String::new() } else { side.digest };
                (side.keys, digest, compare_claims.then_some(side.claims))
            };
            (answers(a) != answers(b)).then(|| KindDifference {
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

/// The keys whose answers differ, from both members' keys of the same buckets.
pub fn differing_keys(ours: &[AnswerKey], theirs: &[AnswerKey]) -> Vec<KeyDifference> {
    let side = |keys: &[AnswerKey]| {
        keys.iter()
            .map(|key| ((key.subject.clone(), key.origin.clone()), key.claim.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let (ours, theirs) = (side(ours), side(theirs));
    ours.keys()
        .chain(theirs.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| ours.get(*key) != theirs.get(*key))
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
        let mut summary = summary(&self.readers.get(), cut_unix_ms)?;
        summary.trims_locally = self.trims_locally();
        Ok(summary)
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

/// How long a computed answer serves requests. A check that found a difference asks again after
/// longer than this, so both members compute again and count claims that arrived late.
pub const ANSWERS_FRESH: Duration = Duration::from_secs(20 * 60);
/// The kinds whose every key is kept at once, for the bucket requests that narrow a difference.
const KEPT_KINDS: usize = 8;

/// Answers computed off the request path and kept for [`ANSWERS_FRESH`]: the summary at one cut,
/// and every key of the few kinds a check is narrowing.
#[derive(Default)]
pub struct AnswersCache {
    state: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    summary: Option<(Instant, AnswersSummary)>,
    kinds: Vec<(Instant, KindBuckets)>,
    computing: BTreeSet<(u128, Option<String>)>,
}

enum Computed {
    Summary(AnswersSummary),
    Kind(KindBuckets),
}

impl AnswersCache {
    /// Answer `request` from what is kept, or answer `pending` and start computing it on a thread
    /// of its own, unless one already does.
    pub fn answer<S>(self: &Arc<Self>, store: &Arc<S>, request: &AnswersRequest) -> AnswersReply
    where
        S: Borrow<Store> + Send + Sync + 'static,
    {
        let fresh = |at: &Instant| at.elapsed() < ANSWERS_FRESH;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match &request.kind {
            None => {
                if let Some((at, summary)) = &state.summary
                    && fresh(at)
                    && summary.cut_unix_ms == request.cut_unix_ms
                {
                    return AnswersReply { summary: Some(summary.clone()), ..AnswersReply::default() };
                }
            }
            Some(kind) => {
                if let Some((_, every)) = state.kinds.iter().find(|(at, every)| {
                    fresh(at) && every.kind == *kind && every.cut_unix_ms == request.cut_unix_ms
                }) {
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
            }
        }
        let job = (request.cut_unix_ms, request.kind.clone());
        if !state.computing.insert(job.clone()) {
            return AnswersReply::pending();
        }
        let (cache, store) = (self.clone(), store.clone());
        let thread_job = job.clone();
        let spawned = std::thread::Builder::new().name("st3-answers".into()).spawn(move || {
            let (cut, kind) = thread_job;
            let store: &Store = (*store).borrow();
            let computed = crate::task_cpu::blocking("replication.answers", || match &kind {
                None => store.answers_summary(cut).map(Computed::Summary),
                Some(kind) => store
                    .answer_buckets(kind, cut, &(0..BUCKETS).collect())
                    .map(Computed::Kind),
            });
            let mut state = cache.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.computing.remove(&(cut, kind.clone()));
            match computed {
                Ok(Computed::Summary(summary)) => state.summary = Some((Instant::now(), summary)),
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
