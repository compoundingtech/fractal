//! Closed graph mail leaves the graph a week after it closes.
//!
//! A graph thread (a `message/...` subject's `message.*` claims) retires when its
//! `message.closed` was accepted at least `RETAIN_AFTER_CLOSE_MS` ago, neither its `from` nor its
//! `to` is a person, and no claim outside the thread cites any of its claims. Person threads and
//! anything cited stay for good. The rule reads only the claims, so every member that applies it
//! to the same claims retires the same threads, with no coordination: each deleted ID enters
//! `retired_claim_ids` in the deleting transaction, peers compare the retired set beside the
//! retained one, and admission refuses a retired ID, so no peer sends it back. A thread judged
//! before its sent claim arrived is judged again later (`DEFERRED`). A citation that reaches one
//! member before it judges a thread and another after it retired it leaves them apart: the first
//! keeps its copy until set exchange applies a peer's retired set to it (the retired set wins
//! there; today the claim range comparison only shows the difference).
//!
//! Only a member whose retired ledger is the authority (`[trim] local`) retires, and only while
//! `[trim] mail = true`. It deletes through the trim's own path, one claim at
//! a time inside a writer hold bounded by a time budget, and never touches envelopes,
//! checkpoints or seals. Stream mail lives in the messages shard and is not touched here.
use super::*;
use smallclaims::store::checkpoint::{ClaimTombstone, delete_dropped_rows_tx};
use std::time::{Duration, Instant};

/// A closed thread stays this long before it retires: 7 days.
pub const RETAIN_AFTER_CLOSE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// One writer hold stops starting new deletes after this long: a delete costs 3–4.5 ms on a
/// 57 GB member, so the hold, its commit included, stays well under 100 ms.
pub const HOLD_BUDGET: Duration = Duration::from_millis(40);

/// `meta`: the store index of the newest `message.closed` claim every thread before which has
/// been judged. Only a judged thread moves it, so a stop part way through resumes there.
pub const CURSOR: &str = "mail_retirement_closed_through";

/// `meta`: the subjects of closed threads judged before their sent claim arrived, as a JSON
/// array. Their people are unknown until it does, so later steps judge them again, and a member
/// that stored the same claims in another order reaches the same answer.
pub const DEFERRED: &str = "mail_retirement_deferred";

/// Closed claims, and deferred threads, judged per step ahead of the writer hold.
const CANDIDATES: usize = 16;

/// While this many threads wait for their sent claim the cursor waits too, so the list stays small.
const DEFERRED_MAX: usize = 1024;

/// What one step did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MailRetirement {
    /// Whole threads deleted.
    pub threads: usize,
    /// Claims deleted, including those of a thread the budget cut part way.
    pub claims: usize,
    /// The writer hold, commit included.
    pub hold: Duration,
    /// The part of the hold spent committing.
    pub commit: Duration,
    /// The longest single claim delete in the hold.
    pub slowest_delete: Duration,
    /// Whether more closed threads old enough to judge remain after this step.
    pub more: bool,
}

/// The rule's answer for one thread.
enum Judgement {
    /// Its claims in deletion order: the sent and then the closed claims last, so a step that
    /// stops part way leaves both, and the next step finds and judges the thread again.
    Retire(Vec<(String, Option<String>, String)>),
    /// A person's thread, a cited one, or one already gone.
    Keep,
    /// Closed, but its sent claim has not arrived: judge it again later.
    Defer,
}

impl Store {
    /// Retire closed graph mail for at most one bounded writer hold. Call again while `more`.
    pub fn retire_closed_mail_step(&self, now_ms: u64) -> Result<MailRetirement> {
        self.retire_closed_mail_step_within(now_ms, HOLD_BUDGET)
    }

    pub fn retire_closed_mail_step_within(&self, now_ms: u64, budget: Duration) -> Result<MailRetirement> {
        if !smallclaims::store::retired::authoritative(&self.readers.get())? {
            return Ok(MailRetirement::default());
        }
        // Citations live in claim bodies; the cache answers who cites a thread without reading them.
        self.catch_up_local_trim_citations()?;
        let cutoff = now_ms.saturating_sub(RETAIN_AFTER_CLOSE_MS);
        let (after, mut deferred) = self.retirement_progress()?;
        // Deferred threads first, the oldest first; one still deferred goes to the back.
        let mut judged: Vec<(Option<i64>, String, Judgement)> = Vec::new();
        {
            let connection = self.readers.get();
            for subject in deferred.iter().take(CANDIDATES) {
                judged.push((None, subject.clone(), judge_thread(&connection, subject, cutoff)?));
            }
        }
        let rejudged = judged.len();
        let (judged_through, more) = if deferred.len() < DEFERRED_MAX {
            self.judge_closed_threads(after, cutoff, &mut judged)?
        } else {
            (after, false)
        };
        warm(&self.readers.get(), &judged);
        let connection = self.connection.write_background();
        let started = Instant::now();
        let mut done = super::database_size::committing_without_fsync(&connection, || {
            let mut done = MailRetirement { more, ..MailRetirement::default() };
            let transaction = connection.unchecked_transaction()?;
            let mut cursor = after;
            let mut still_deferred = Vec::new();
            let mut cut = false;
            for (closed_index, subject, judgement) in &judged {
                // Once the budget runs out, a deferred thread keeps its place in the list and a new
                // one waits behind the cursor for the next step.
                let finished = !cut
                    && match judgement {
                        Judgement::Retire(claims) => delete_thread(&transaction, claims, started, budget, &mut done)?,
                        Judgement::Keep => true,
                        Judgement::Defer => {
                            still_deferred.push(subject.clone());
                            true
                        }
                    };
                if !finished {
                    cut = true;
                    if closed_index.is_none() {
                        still_deferred.push(subject.clone());
                    }
                } else if let Some(index) = closed_index {
                    cursor = *index;
                }
            }
            if cut {
                done.more = true;
            } else {
                cursor = cursor.max(judged_through);
            }
            let mut seen = std::collections::BTreeSet::new();
            deferred = deferred.split_off(rejudged.min(deferred.len()));
            deferred.extend(still_deferred);
            deferred.retain(|subject| seen.insert(subject.clone()));
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-hf3a8ffdc82a1 owner=fractal_store::store::mail_retirement TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2), (?3, ?4)",
                params![CURSOR, cursor.to_string(), DEFERRED, serde_json::to_string(&deferred)?],
            )?;
            let committing = Instant::now();
            transaction.commit()?;
            done.commit = committing.elapsed();
            Ok(done)
        })?;
        done.hold = started.elapsed();
        Ok(done)
    }

    /// The cursor and the deferred threads, as the last step left them.
    fn retirement_progress(&self) -> Result<(i64, Vec<String>)> {
        let connection = self.readers.get();
        let read = |key: &str| -> Result<Option<String>> {
            Ok(connection.query_row("SELECT value FROM meta WHERE key=?1", [key], |row| row.get(0)).optional()?)
        };
        let after = read(CURSOR)?.and_then(|value| value.parse().ok()).unwrap_or(0);
        let deferred = read(DEFERRED)?.and_then(|value| serde_json::from_str(&value).ok()).unwrap_or_default();
        Ok((after, deferred))
    }

    /// Judge the next closed threads after `after`, appending each to `judged` with its closed
    /// claim's store index. Stops before the first closed claim younger than `cutoff`. Returns
    /// the store index judged through and whether more old enough to judge remain.
    fn judge_closed_threads(
        &self,
        after: i64,
        cutoff: u64,
        judged: &mut Vec<(Option<i64>, String, Judgement)>,
    ) -> Result<(i64, bool)> {
        let connection = self.readers.get();
        let closed: Vec<(i64, String, String)> = connection
            .prepare_cached(
                "SELECT store_index, subject, accepted_at_unix_ms FROM claims INDEXED BY claims_kind_index
                 WHERE kind='message.closed' AND store_index>?1 ORDER BY store_index LIMIT ?2",
            )?
            .query_map(params![after, CANDIDATES as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let full = closed.len() == CANDIDATES;
        let mut through = after;
        for (index, subject, accepted) in closed {
            // Closed claims arrive in roughly the order they close; the first one too young ends
            // this step's judging, and the next step starts there again.
            if accepted.parse::<u64>().map_or(true, |accepted| accepted > cutoff) {
                return Ok((through, false));
            }
            through = index;
            // A thread with two closed claims, or already judged this step, is judged once.
            let judgement = if judged.iter().any(|(_, seen, _)| *seen == subject) {
                Judgement::Keep
            } else {
                judge_thread(&connection, &subject, cutoff)?
            };
            judged.push((Some(index), subject, judgement));
        }
        Ok((through, full))
    }
}

/// Delete a thread's claims inside the hold. Answers whether the whole thread went; `false`
/// when the budget ran out first. Once the sent claim goes the closed claims follow in the
/// same hold, since judging a thread reads both.
fn delete_thread(
    transaction: &Transaction<'_>,
    claims: &[(String, Option<String>, String)],
    started: Instant,
    budget: Duration,
    done: &mut MailRetirement,
) -> Result<bool> {
    let mut finishing = false;
    for (id, operation_id, kind) in claims {
        // At least one delete a hold, so a slow store still makes progress. The budget is read
        // before the sent claim too: a thread of only a sent and a closed claim must not pass
        // it whole, or one hold takes every thread judged.
        if !finishing && done.claims > 0 && started.elapsed() >= budget {
            return Ok(false);
        }
        let deleting = Instant::now();
        delete_dropped_rows_tx(transaction, &[], &[tombstone(id, operation_id.clone())])?;
        done.slowest_delete = done.slowest_delete.max(deleting.elapsed());
        done.claims += 1;
        finishing |= kind == "message.sent";
    }
    done.threads += 1;
    Ok(true)
}

/// Claims a hold can reach, at about 4 ms each in a 40 ms budget, with room to spare.
const WARM_CLAIMS: usize = 24;

/// Read, before the writer hold, the rows that deleting the first claims judged will touch, so
/// the hold finds their pages cached. On a 57 GB member most deletes took 3–8 ms, but the
/// signature cleanup (`principals::forget_claim_tx`) sometimes took 100–390 ms for one claim:
/// every plan is an index search, so it was waiting on cold pages across the signature tables.
/// Only a hint: a table this build lacks, or any other error, is skipped.
fn warm(connection: &Connection, judged: &[(Option<i64>, String, Judgement)]) {
    const BY_CLAIM: [&str; 9] = [
        "replica_records",
        "claim_signatures",
        "claim_signature_candidates",
        "claim_signature_nonce_uses",
        "claim_signature_judging",
        "claim_verdicts",
        "claim_verdict_links",
        "claim_verdict_dependency_edges",
        "claim_signature_originals",
    ];
    let read = |sql: &str, id: &str| -> rusqlite::Result<()> {
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h9fa486291dca owner=fractal_store::store::mail_retirement TODO=trace-statement-and-value-representation */ sql)?;
        let mut rows = statement.query([id])?;
        while rows.next()?.is_some() {}
        Ok(())
    };
    let claims = judged.iter().flat_map(|(_, _, judgement)| match judgement {
        Judgement::Retire(claims) => claims.as_slice(),
        _ => &[],
    });
    for (id, ..) in claims.take(WARM_CLAIMS) {
        for table in BY_CLAIM {
            let _ = read(&format!("SELECT * FROM {table} WHERE claim_id=?1"), id);
        }
        // The claims that share a nonce with this one, whose judging the delete marks dirty.
        let _ = read(
            "SELECT judging.* FROM claim_signature_nonce_uses AS n
             CROSS JOIN claim_verdict_dependency_links AS l ON l.link='nonce:'||n.key||'|'||n.nonce
             JOIN claim_signature_judging AS judging ON judging.claim_id=l.claim_id
             WHERE n.claim_id=?1",
            id,
        );
    }
}

/// How many threads deep a chain of mail citing mail is followed; past it the thread stays.
const CITATION_DEPTH: usize = 8;

/// The rule's answer for the thread `subject`, a function of the claims alone and never of how
/// far a pass has got, so every member that holds the same claims gives the same answer. A
/// claim outside the thread that cites it keeps it unless that claim is mail whose own thread
/// retires under this rule: judging a thread before or after its mail citer went must not
/// change the answer. A citer already gone was retired, so it keeps nothing.
fn judge_thread(connection: &Connection, subject: &str, cutoff: u64) -> Result<Judgement> {
    judge(connection, subject, cutoff, 0, &mut BTreeMap::new())
}

/// `depth` 0 is a thread whose closed claim the caller found old enough; deeper is a citer's
/// thread, which must show one itself. `kept` remembers citer threads already judged, and
/// marks one being judged as kept, so a cycle keeps its threads.
fn judge(
    connection: &Connection,
    subject: &str,
    cutoff: u64,
    depth: usize,
    kept: &mut BTreeMap<String, bool>,
) -> Result<Judgement> {
    let claims: Vec<(String, Option<String>, String, Option<String>, Option<String>, String)> = connection
        .prepare_cached(
            "SELECT id, json_extract(body,'$._operation.id'), kind,
                    json_extract(body,'$.fields.from'), json_extract(body,'$.fields.to'), accepted_at_unix_ms
             FROM claims INDEXED BY claims_subject_kind_index
             WHERE subject=?1 AND kind>='message.' AND kind<'message/' ORDER BY store_index",
        )?
        .query_map([subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if claims.is_empty() {
        return Ok(Judgement::Keep);
    }
    // Without its sent claim a thread's people are unknown: wait for it.
    let Some((_, _, _, from, to, _)) = claims.iter().find(|(_, _, kind, ..)| kind == "message.sent") else {
        return Ok(Judgement::Defer);
    };
    let person = |party: &Option<String>| party.as_deref().is_some_and(|party| party.starts_with("person/"));
    if person(from) || person(to) {
        return Ok(Judgement::Keep);
    }
    let old_enough = claims.iter().any(|(_, _, kind, _, _, accepted)| {
        kind == "message.closed" && accepted.parse::<u64>().is_ok_and(|accepted| accepted <= cutoff)
    });
    if depth > 0 && !old_enough {
        return Ok(Judgement::Keep);
    }
    let own: std::collections::BTreeSet<&str> = claims.iter().map(|(id, ..)| id.as_str()).collect();
    for (id, ..) in &claims {
        let citers: Vec<(String, String, String)> = connection
            .prepare_cached(
                "SELECT citation.citer, citer.kind, citer.subject FROM local_trim_citations AS citation
                 JOIN claims AS citer ON citer.id=citation.citer WHERE citation.cited=?1",
            )?
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (citer, kind, citer_subject) in citers {
            if own.contains(citer.as_str()) {
                continue;
            }
            if !kind.starts_with("message.") || depth + 1 >= CITATION_DEPTH {
                return Ok(Judgement::Keep);
            }
            let citer_kept = match kept.get(&citer_subject) {
                Some(citer_kept) => *citer_kept,
                None => {
                    kept.insert(citer_subject.clone(), true);
                    let citer_kept = !matches!(judge(connection, &citer_subject, cutoff, depth + 1, kept)?, Judgement::Retire(_));
                    kept.insert(citer_subject, citer_kept);
                    citer_kept
                }
            };
            if citer_kept {
                return Ok(Judgement::Keep);
            }
        }
    }
    let mut ordered: Vec<_> = claims.into_iter().map(|(id, operation, kind, ..)| (id, operation, kind)).collect();
    ordered.sort_by_key(|(_, _, kind)| match kind.as_str() {
        "message.sent" => 1,
        "message.closed" => 2,
        _ => 0,
    });
    Ok(Judgement::Retire(ordered))
}

fn tombstone(id: &str, operation_id: Option<String>) -> ClaimTombstone {
    // The delete path reads only the identity and the operation; the rest describes a
    // checkpoint's tombstone, which this pass never records.
    ClaimTombstone {
        id: id.to_owned(),
        writer: String::new(),
        sequence: 0,
        envelope_hash: String::new(),
        subject: String::new(),
        kind: String::new(),
        actor: None,
        predecessors: Vec::new(),
        operation_id,
        request_digest: None,
        accepted_at_unix_ms: 0,
    }
}
