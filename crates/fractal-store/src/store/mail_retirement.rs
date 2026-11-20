//! Closed graph mail leaves the graph a week after it closes.
//!
//! A graph thread (a `message/...` subject's `message.*` claims) retires when its
//! `message.closed` was accepted at least `RETAIN_AFTER_CLOSE_MS` ago, neither its `from` nor its
//! `to` is a person, and no claim outside the thread cites any of its claims. Person threads and
//! anything cited stay for good. The rule reads only the claims, so every member that applies it
//! to the same claims retires the same threads, with no coordination: each deleted ID enters
//! `retired_claim_ids` in the deleting transaction, peers compare the retired set beside the
//! retained one, and admission refuses a retired ID, so no peer sends it back.
//!
//! Only a member whose retired ledger is the authority (`[trim] local`) retires, and only while
//! `[trim] mail` is on (the default there). It deletes through the trim's own path, one claim at
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

/// Closed claims judged per step, ahead of the writer hold.
const CANDIDATES: i64 = 16;

/// What one step did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MailRetirement {
    /// Whole threads deleted.
    pub threads: usize,
    /// Claims deleted, including those of a thread the budget cut part way.
    pub claims: usize,
    /// The writer hold, commit included.
    pub hold: Duration,
    /// Whether more closed threads old enough to judge remain after this step.
    pub more: bool,
}

/// One thread the rule retires: its claims, the closed claim last, so a step that stops part
/// way leaves the closed claim and the next step finds the rest.
struct Thread {
    claims: Vec<(String, Option<String>, String)>,
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
        let (judged_through, threads, more) = self.judge_closed_threads(cutoff)?;
        let mut done = MailRetirement { more, ..MailRetirement::default() };
        let mut connection = self.connection.write_background();
        let started = Instant::now();
        let transaction = connection.transaction()?;
        let mut cursor = None;
        'threads: for (closed_index, thread) in &threads {
            if let Some(thread) = thread {
                let mut finishing = false;
                for (id, operation_id, kind) in &thread.claims {
                    // At least one delete a hold, so a slow store still makes progress. Once the
                    // sent claim goes, the closed claims follow in the same hold: judging a thread
                    // reads both, and a closed claim left without its sent claim would stay.
                    finishing |= kind == "message.sent";
                    if !finishing && done.claims > 0 && started.elapsed() >= budget {
                        done.more = true;
                        break 'threads;
                    }
                    delete_dropped_rows_tx(&transaction, &[], &[tombstone(id, operation_id.clone())])?;
                    done.claims += 1;
                }
                done.threads += 1;
            }
            cursor = Some(*closed_index);
        }
        let cursor = match (cursor, done.more) {
            // Every judged thread finished: the cursor may pass all of them.
            (Some(_), false) => Some(judged_through),
            (cursor, _) => cursor,
        };
        if let Some(cursor) = cursor {
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h3caa5ecf593d owner=fractal_store::store::mail_retirement TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                params![CURSOR, cursor.to_string()],
            )?;
        }
        transaction.commit()?;
        done.hold = started.elapsed();
        Ok(done)
    }

    /// The next closed threads after the cursor, each judged by the rule: `Some` to retire,
    /// `None` to keep. Stops before the first closed claim younger than `cutoff`. Returns the
    /// store index judged through, the threads, and whether more old enough to judge remain.
    #[allow(clippy::type_complexity)]
    fn judge_closed_threads(&self, cutoff: u64) -> Result<(i64, Vec<(i64, Option<Thread>)>, bool)> {
        let connection = self.readers.get();
        let after: i64 = connection
            .query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |row| row.get::<_, String>(0))
            .optional()?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let closed: Vec<(i64, String, String)> = connection
            .prepare_cached(
                "SELECT store_index, subject, accepted_at_unix_ms FROM claims INDEXED BY claims_kind_index
                 WHERE kind='message.closed' AND store_index>?1 ORDER BY store_index LIMIT ?2",
            )?
            .query_map(params![after, CANDIDATES], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let full = closed.len() as i64 == CANDIDATES;
        let mut judged = Vec::new();
        let mut through = after;
        let mut seen = std::collections::BTreeSet::new();
        for (index, subject, accepted) in closed {
            // Closed claims arrive in roughly the order they close; the first one too young ends
            // this step's judging, and the next step starts there again.
            if accepted.parse::<u64>().map_or(true, |accepted| accepted > cutoff) {
                return Ok((through, judged, false));
            }
            through = index;
            // A thread with two closed claims is judged once.
            let thread = if seen.insert(subject.clone()) { judge_thread(&connection, &subject)? } else { None };
            judged.push((index, thread.map(|claims| Thread { claims })));
        }
        Ok((through, judged, full))
    }
}

/// The thread's claims in deletion order when the rule retires it, else `None`.
fn judge_thread(connection: &Connection, subject: &str) -> Result<Option<Vec<(String, Option<String>, String)>>> {
    let claims: Vec<(String, Option<String>, String, Option<String>, Option<String>)> = connection
        .prepare_cached(
            "SELECT id, json_extract(body,'$._operation.id'), kind,
                    json_extract(body,'$.fields.from'), json_extract(body,'$.fields.to')
             FROM claims INDEXED BY claims_subject_kind_index
             WHERE subject=?1 AND kind>='message.' AND kind<'message/' ORDER BY store_index",
        )?
        .query_map([subject], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // Without its sent claim a thread's people are unknown; keep it.
    let Some((_, _, _, from, to)) = claims.iter().find(|(_, _, kind, _, _)| kind == "message.sent") else {
        return Ok(None);
    };
    let person = |party: &Option<String>| party.as_deref().is_some_and(|party| party.starts_with("person/"));
    if person(from) || person(to) {
        return Ok(None);
    }
    let own: std::collections::BTreeSet<&str> = claims.iter().map(|(id, ..)| id.as_str()).collect();
    let mut citers = connection.prepare_cached(
        "SELECT citation.citer FROM local_trim_citations AS citation
         WHERE citation.cited=?1 AND EXISTS(SELECT 1 FROM claims WHERE claims.id=citation.citer)",
    )?;
    for (id, ..) in &claims {
        let cited_from_outside = citers
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|citer| !own.contains(citer.as_str()));
        if cited_from_outside {
            return Ok(None);
        }
    }
    let mut ordered: Vec<_> = claims.into_iter().map(|(id, operation, kind, _, _)| (id, operation, kind)).collect();
    // The sent and then the closed claims go last: a step cut part way leaves both, and they
    // are what finds and judges the thread again.
    ordered.sort_by_key(|(_, _, kind)| match kind.as_str() {
        "message.sent" => 1,
        "message.closed" => 2,
        _ => 0,
    });
    Ok(Some(ordered))
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
