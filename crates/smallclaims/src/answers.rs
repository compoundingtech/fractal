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
//! and kept, never per request.
use std::collections::{BTreeMap, BTreeSet};

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
    Ok(AnswersSummary { version: ANSWERS_VERSION, cut_unix_ms, kinds: kinds_answers })
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
        summary(&self.readers.get(), cut_unix_ms)
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
