//! Candidate pages and their dependency accumulator commit together. A page never authorizes
//! a partial result. Relevant authority changes dirty the cursor in queue_linked_tx.
//! Completed, unblocked folds retain their result witness but discard their resume-only
//! cursor and roots digest. Invalidation always restarts them from retained candidates;
//! their global dependency links remain live regardless of when judgment completed.
use super::*;

pub(super) fn rank(verdict: &Verdict) -> u8 {
    match verdict {
        Verdict::Verified => 0,
        Verdict::Held(_) => 1,
        Verdict::Invalid(_) => 2,
        Verdict::Unsigned => 3,
    }
}
pub(super) fn candidate_count(connection: &Connection, claim_id: &str) -> Result<u64> {
    Ok(connection
        .prepare_cached("SELECT candidates FROM claim_signature_counts WHERE claim_id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h5a2bf49b2f3f owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}
pub(super) fn complete(connection: &Connection, claim_id: &str) -> Result<bool> {
    Ok(connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claim_signature_judging p JOIN claim_signature_counts c USING(claim_id) WHERE p.claim_id=?1 AND p.complete=1 AND p.dirty=0 AND p.blocked=0 AND p.candidate_count=c.candidates)")?
        .query_row(/* sqlite-large-value: allow-for-now lv-h4cf561874b59 owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id],|row|row.get(0))?)
}
pub(super) fn validate_cached(connection: &Connection, row: &ClaimRow) -> Result<()> {
    let (content,signature,verdict,reason)=connection.prepare_cached("SELECT content_digest,best_signature,best_verdict,best_reason FROM claim_signature_judging WHERE claim_id=?1")?.query_row(/* sqlite-large-value: allow-for-now lv-h772f8904542f owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [&row.id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,Option<SqlValue>>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?)))?;
    let signature = signature
        .map(|value| references::resolve(connection, &row.id, value))
        .transpose()?;
    anyhow::ensure!(
        content == content_digest(&row.subject, &row.kind, row.actor.as_deref(), &row.body),
        "signature-state-corrupt: claim content differs from completed judgment"
    );
    let cached = cached_verdict(connection, &row.id)?.ok_or_else(|| {
        anyhow::anyhow!("signature-state-corrupt: completed judgment has no cached verdict")
    })?;
    anyhow::ensure!(
        verdict.as_deref() == Some(cached.name()) && reason.as_deref() == cached.reason(),
        "signature-state-corrupt: cached verdict differs from completed judgment"
    );
    let facade = stored_signature(connection, &row.id)?.ok_or_else(|| {
        anyhow::anyhow!("signature-state-corrupt: completed judgment has no signature")
    })?;
    anyhow::ensure!(
        signature.as_deref() == Some(serde_json::to_string(&facade)?.as_str()),
        "signature-state-corrupt: signature facade differs from completed judgment"
    );
    Ok(())
}

pub(super) fn unavailable(connection: &Connection, claim_id: &str) -> Result<Option<String>> {
    let count = candidate_count(connection, claim_id)?;
    let pending: bool=connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claim_signature_judging WHERE claim_id=?1 AND (complete=0 OR dirty=1 OR blocked=1))")?.query_row(/* sqlite-large-value: allow-for-now lv-h6199c4573f91 owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id],|row|row.get(0))?;
    if pending || (count > CANDIDATE_PAGE as u64 && !complete(connection, claim_id)?) {
        return Ok(Some(format!(
            "signature-judgment-incomplete: {claim_id} has {count} candidates"
        )));
    }
    if let Some(link) = requeue_ahead_of(connection, claim_id)? {
        return Ok(Some(format!(
            "signature-judgment-incomplete: {claim_id} waits for the requeue of {link}"
        )));
    }
    Ok(None)
}

/// A settled judgment: complete, clean, unblocked, its verdict published, and small enough
/// that `judge_claim` judges it directly without reading this row. Old and new readers treat a
/// missing row of such a claim as never paged: `unavailable` sees nothing pending, and a
/// later requeue inserts a fresh row before judging. So the row is dropped once settled.
/// The two statements below spell the same predicate; 32 is `CANDIDATE_PAGE`.
const _: () = assert!(CANDIDATE_PAGE == 32);

/// Drop `claim_id`'s judging row if it is settled. Returns whether it did.
pub(super) fn discard_settled_tx(connection: &Connection, claim_id: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h323477dd058f owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_signature_judging WHERE claim_id=?1
             AND complete=1 AND dirty=0 AND blocked=0 AND notified=1 AND candidate_count<=32
             AND candidate_count=COALESCE((SELECT candidates FROM claim_signature_counts c
                                           WHERE c.claim_id=claim_signature_judging.claim_id), 0)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h8858f9f9045d owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ params![claim_id])?
        != 0)
}

/// Rows the prune looks at in one writer loan, and the time after which it commits early.
pub const PRUNE_PAGE: usize = 512;
pub const PRUNE_BUDGET: std::time::Duration = std::time::Duration::from_millis(10);
/// The last claim the prune of settled judging rows has passed, or `done`.
pub(super) const PRUNE_CURSOR: &str = "claim_signature_judging_prune";

/// One page of the prune of settled judging rows that releases before this one kept: walk the
/// rows in claim order from the cursor and delete each settled one. Returns `(examined,
/// deleted, finished)`. The cursor only moves forward; a row that settles behind it is dropped
/// by `discard_settled_tx` when it settles.
pub(super) fn prune_settled_page_tx(connection: &Connection) -> Result<(usize, usize, bool)> {
    let mut cursor = super::super::fleet_meta(connection, PRUNE_CURSOR)?.unwrap_or_default();
    if cursor == "done" {
        // Once pruned, only pending folds and large ones keep rows, so this reads few rows. A
        // settled row here was written by an earlier release after a rollback: start over.
        let left: bool = connection
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM claim_signature_judging
                 WHERE complete=1 AND dirty=0 AND blocked=0 AND notified=1 AND candidate_count<=32
                 AND candidate_count=COALESCE((SELECT candidates FROM claim_signature_counts c
                                               WHERE c.claim_id=claim_signature_judging.claim_id), 0))",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h84b85cdb55be owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
        if !left {
            return Ok((0, 0, true));
        }
        cursor = String::new();
    }
    let started = std::time::Instant::now();
    let claims = connection
        .prepare_cached(
            "SELECT claim_id FROM claim_signature_judging WHERE claim_id>?1 ORDER BY claim_id LIMIT ?2",
        )?
        .query_map(params![cursor, PRUNE_PAGE as i64], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut examined = 0;
    let mut deleted = 0;
    let mut last = None;
    for claim_id in &claims {
        if examined != 0 && started.elapsed() >= PRUNE_BUDGET {
            break;
        }
        examined += 1;
        deleted += usize::from(discard_settled_tx(connection, claim_id)?);
        last = Some(claim_id.as_str());
    }
    let finished = claims.len() < PRUNE_PAGE && examined == claims.len();
    let next = if finished { "done" } else { last.unwrap_or(cursor.as_str()) };
    super::set_meta_tx(connection, PRUNE_CURSOR, next)?;
    Ok((examined, deleted, finished))
}

pub(super) type Judgment = (Verdict, Option<ClaimSignature>);
/// At most budget candidates are judged in this writer loan. Unprocessed work stays queued.
/// The caller already read the current claim in this transaction; reuse that decoded body.
pub(super) fn advance(
    connection: &Connection,
    roots: &BTreeSet<(String, String)>,
    row: &ClaimRow,
    memo: &RefCell<BTreeMap<String, Verdict>>,
    budget: usize,
) -> Result<(Option<Judgment>, usize)> {
    let claim_id = &row.id;
    let count = candidate_count(connection, claim_id)?;
    let roots_digest = crate::hash::canonical_hash(roots)?;
    let previous=connection.prepare_cached("SELECT candidate_count,roots_digest,last_signature,best_signature,best_verdict,best_reason,complete,dirty,blocked FROM claim_signature_judging WHERE claim_id=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-he591a2246f4a owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id],|row|Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?,row.get::<_,SqlValue>(2)?,row.get::<_,Option<SqlValue>>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,bool>(6)?,row.get::<_,bool>(7)?,row.get::<_,bool>(8)?))).optional()?;
    let (last, mut best, mut blocked) = match previous {
        Some((n, root, last, signature, verdict, reason, false, false, blocked))
            if n == count && root == roots_digest =>
        {
            let last = references::resolve(connection, claim_id, last)?;
            let signature = signature
                .map(|value| references::resolve(connection, claim_id, value))
                .transpose()?;
            if let Some(name) = &verdict
                && !matches!(name.as_str(), "verified" | "held" | "invalid" | "unsigned")
            {
                anyhow::bail!(
                    "signature-state-corrupt: unknown candidate accumulator verdict {name}"
                );
            }
            (
                last,
                signature
                    .zip(verdict)
                    .map(|(signature, verdict)| (verdict_from(&verdict, reason), signature)),
                blocked,
            )
        }
        _ => {
            // Retained candidates can become relevant again. Keep their dependency
            // superset incrementally; clearing/rebuilding it here would make a single
            // loan proportional to the whole candidate set. Forget/recheck own cleanup.
            connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h85728a5c3b07 owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_signature_judging(claim_id,candidate_count,roots_digest,content_digest,last_signature,complete,dirty) VALUES (?1,?2,?3,?4,'',0,0) ON CONFLICT(claim_id) DO UPDATE SET candidate_count=excluded.candidate_count,roots_digest=excluded.roots_digest,content_digest=excluded.content_digest,last_signature='',best_signature=NULL,best_verdict=NULL,best_reason=NULL,complete=0,dirty=0,notified=0,blocked=0")?.execute(/* sqlite-large-value: allow-for-now lv-h86161607bbb6 owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ params![claim_id,count,roots_digest,content_digest(&row.subject,&row.kind,row.actor.as_deref(),&row.body)])?;
            (String::new(), None, false)
        }
    };
    let mut texts=connection.prepare_cached("SELECT signature FROM claim_signature_candidates WHERE claim_id=?1 AND signature>?2 ORDER BY signature LIMIT ?3")?
        .query_map(params![claim_id,last,budget as i64],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    // A compacted claim's facade is its single candidate.
    if texts.is_empty() && last.is_empty() && budget > 0
        && let Some(text) = compaction::facade(connection, claim_id)?
    {
        texts.push(text);
    }
    let candidates = texts
        .iter()
        .map(|text| Ok(serde_json::from_str::<ClaimSignature>(text)?))
        .collect::<Result<Vec<_>>>()?;
    let (verdict, links, signature) =
        judge_candidates(connection, roots, row, candidates, 0, memo)?;
    blocked |= matches!(&verdict,Verdict::Held(reason) if reason.starts_with("signature-judgment-incomplete:"));
    // Final once found: later batches only add constraints, and a forgotten one must not undo it.
    if matches!(&verdict, Verdict::Invalid(reason) if reason == crate::principal::BATCH_DISORDER) {
        connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-he263347cc8da owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_batch_disorder(claim_id) VALUES (?1)")?
            .execute(/* sqlite-large-value: allow-for-now lv-h81fa36b9b7b4 owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id])?;
    }
    for link in &links {
        connection
            .prepare_cached(
                /* sqlite-large-value: allow-for-now lv-h85bbaa862d7c owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_links(link,claim_id) VALUES (?1,?2)",
            )?
            .execute(/* sqlite-large-value: allow-for-now lv-h618b3370f56f owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ params![link, claim_id])?;
    }
    if let Some(signature) = signature {
        let text = serde_json::to_string(&signature)?;
        if best
            .as_ref()
            .is_none_or(|(old, old_text)| (rank(&verdict), &text) < (rank(old), old_text))
        {
            best = Some((verdict, text));
        }
    }
    let finished = texts.len() < budget;
    let (best_name, best_reason, best_signature) =
        best.as_ref()
            .map_or((None, None, None), |(verdict, signature)| {
                (
                    Some(verdict.name()),
                    verdict.reason(),
                    Some(signature.as_str()),
                )
            });
    // Only unfinished or blocked folds can resume from this cursor. Completed
    // judgments restart from canonical candidates when dirtied; retaining their
    // final full signature and roots digest duplicates evidence indefinitely.
    let cold = finished && !blocked;
    let through = if cold {
        ""
    } else {
        texts.last().map_or(last.as_str(), String::as_str)
    };
    let through = if through.is_empty() {
        references::SignatureValue::Text(through)
    } else {
        references::write_value(connection, claim_id, through, true)?
    };
    let best_signature = best_signature
        .map(|text| references::write_value(connection, claim_id, text, true))
        .transpose()?;
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hc47520fe1f56 owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_signature_judging SET last_signature=?1,best_signature=?2,best_verdict=?3,best_reason=?4,complete=?5,blocked=?6,roots_digest=CASE WHEN ?8 THEN '' ELSE roots_digest END WHERE claim_id=?7")?.execute(/* sqlite-large-value: allow-for-now lv-h2a77ab38ae8d owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ params![through,best_signature,best_name,best_reason,finished,blocked,claim_id,cold])?;
    if !finished {
        connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h8ef495ef23ca owner=smallclaims::store::principals::judging TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO claim_verdict_queue(claim_id) VALUES (?1)")?
            .execute(/* sqlite-large-value: allow-for-now lv-hbacf8f94aea6 owner=smallclaims::store::principals::judging TODO=trace-statement-and-value-representation */ [claim_id])?;
        return Ok((None, texts.len()));
    }
    if count != 0 && best.is_none() {
        anyhow::bail!(
            "signature-state-corrupt: nonempty candidate judgment has no result for {claim_id}"
        );
    }
    let result = if let Some((verdict, text)) = best {
        (
            if blocked {
                Verdict::Held(
                    "signature-judgment-incomplete: an authority dependency is pending".into(),
                )
            } else {
                verdict
            },
            Some(serde_json::from_str(&text)?),
        )
    } else {
        (Verdict::Unsigned, None)
    };
    Ok((Some(result), texts.len()))
}
