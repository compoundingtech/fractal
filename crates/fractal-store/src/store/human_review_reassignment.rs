//! A one-shot reviewer repair belongs to the existing request, not its published gate spec.
//! Readers fold immutable reassignment claims; the request's operation and episode stay intact.
use super::*;
use crate::model::PersonRenameRequest;

/// SQL for the reviewer assigned to this exact request. An answer uses the assignment at its
/// canonical moment, so later repairs cannot erase an answer already made by its reviewer.
/// Apply accepted edges in canonical order only when their previous reviewer is the
/// effective reviewer so far; a downstream repair cannot revive a losing concurrent hop.
/// Unary `+` removes the request ID's TEXT affinity so its JSON-expression index is seekable;
/// both IDs remain strings, as in the operation-conflict query.
pub fn reviewer_sql(request: &str, answer: Option<&str>) -> String {
    let before_answer = answer
        .map(|answer| format!(" AND {}", canonical::after_sql(answer, "reassignment")))
        .unwrap_or_default();
    let accepted = person_rename::accepted_reassignment_sql(
        "reassignment",
        "previous_reviewer",
        "reviewer",
    );
    format!(
        "(WITH RECURSIVE ordered_reassignments(position, previous_reviewer, reviewer) AS MATERIALIZED (
             SELECT row_number() OVER (ORDER BY {}),
                    json_extract(reassignment.body, '$.fields.previous_reviewer'),
                    json_extract(reassignment.body, '$.fields.reviewer')
             FROM claims reassignment INDEXED BY claims_human_gate_reassignment_request_index
             WHERE reassignment.subject={request}.subject
               AND reassignment.kind='gate.reviewer-reassigned'
               AND json_extract(reassignment.body, '$.fields.request')=+{request}.id
               AND json_extract(reassignment.body, '$.fields.owner')=json_extract({request}.body, '$.fields.owner')
               AND {accepted}
               {before_answer}
         ), reviewer_chain(position, reviewer) AS (
             SELECT 0, json_extract({request}.body, '$.fields.reviewer')
             UNION ALL
             SELECT edge.position,
                    CASE WHEN edge.previous_reviewer=chain.reviewer
                         THEN edge.reviewer ELSE chain.reviewer END
             FROM reviewer_chain chain
             JOIN ordered_reassignments edge ON edge.position=chain.position+1
         )
         SELECT reviewer FROM reviewer_chain
         WHERE position=(SELECT count(*) FROM ordered_reassignments))",
        canonical::order_sql("reassignment", false),
    )
}

pub(super) fn reviewer_tx(connection: &Connection, request: &ClaimRecord) -> Result<String> {
    smallclaims::touched::note_read(|| request.subject.clone());
    connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h1a7e1457e965 owner=fractal_store::store::human_review_reassignment TODO=trace-statement-and-value-representation */
            &format!(
                "SELECT {} FROM claims request WHERE request.id=?1 AND request.kind='gate.requested'",
                reviewer_sql("request", None),
            ),
            [&request.id],
            |row| row.get(0),
        )
        .context("the human review request does not name a reviewer")
}

/// Restrict inbox hydration to the requested effective reviewer and unresolved, newest
/// episodes. Candidate routing includes repairs received before their immutable request.
/// Supersession uses the original reviewer, never the repaired reviewer: neither answering
/// nor moving the newest request may revive a superseded copy in OLD's inbox.
pub fn pending_requests_sql(filtered: bool) -> String {
    let candidates = if filtered {
        "SELECT id FROM claims INDEXED BY claims_human_gate_request_index
         WHERE kind='gate.requested' AND json_extract(body, '$.fields.reviewer') IS NOT NULL
           AND json_extract(body, '$.fields.reviewer')=?1
         UNION
         SELECT json_extract(body, '$.fields.request')
         FROM claims INDEXED BY claims_human_gate_reassignment_reviewer_index
         WHERE kind='gate.reviewer-reassigned' AND json_extract(body, '$.fields.reviewer')=?1"
    } else {
        // A global attention read must start from live owners, not every historical request.
        // These are the owner forms accepted by human_review_currency; that function still
        // decides revision, generation, attempt and root-run currency after hydration.
        // Loop descendants include the legacy item and round/candidate owner forms.
        // CROSS JOIN keeps the small owner set outside the indexed claim seeks. Unary +
        // removes TEXT affinity from owner IDs, as it does for exact request IDs above.
        // Keep the parameter shape identical to the reviewer-filtered query.
        "WITH open_steps(subject) AS MATERIALIZED (
             SELECT step_runs.subject
             FROM mission_runs INDEXED BY mission_runs_open_index
             CROSS JOIN step_runs INDEXED BY step_runs_run_index
               ON step_runs.run_id=mission_runs.id
             WHERE mission_runs.status NOT IN ('completed','failed','cancelled')
               AND step_runs.status NOT IN ('completed','failed','cancelled')
         ), owners(owner) AS (
             SELECT 'mission-run/' || id FROM mission_runs INDEXED BY mission_runs_open_index
             WHERE status NOT IN ('completed','failed','cancelled')
             UNION ALL SELECT subject FROM open_steps
             UNION ALL SELECT 'loop-run/' || substr(subject,10) FROM open_steps
         )
         SELECT request.id
         FROM owners
         CROSS JOIN claims request INDEXED BY claims_human_gate_episode_index
           ON json_extract(request.body, '$.fields.owner')=+owners.owner
         WHERE request.kind='gate.requested'
           AND json_extract(request.body, '$.fields.reviewer') IS NOT NULL
           AND ?1 IS NULL
         UNION
         SELECT request.id
         FROM open_steps
         CROSS JOIN claims request INDEXED BY claims_human_gate_episode_index
           ON json_extract(request.body, '$.fields.owner')>='loop-run/' || substr(open_steps.subject,10) || '/'
          AND json_extract(request.body, '$.fields.owner')<'loop-run/' || substr(open_steps.subject,10) || '0'
         WHERE request.kind='gate.requested'
           AND json_extract(request.body, '$.fields.reviewer') IS NOT NULL
           AND ?1 IS NULL"
    };
    let episode = |alias: &str, request: &str| {
        ["owner", "reviewer", "attempt", "mission_revision", "step_definition"]
            .map(|field| {
                format!(
                    "json_extract({alias}.body, '$.fields.{field}')=json_extract({request}.body, '$.fields.{field}')"
                )
            })
            .join(" AND ")
    };
    let effective_reviewer = reviewer_sql("request", None);
    // Finish answer/supersession filtering before looking up an episode's first request.
    // Otherwise SQLite may run that canonical sort for every historical candidate, including
    // answered episodes that cannot contribute a review or its waiting-since timestamp.
    // Keep candidates outside hydration too: an ordinary JOIN lets SQLite scan the
    // partial reviewer index and probe an automatic candidates index for every historical
    // request, despite the bounded owner seeks inside candidates.
    canonical_sql(&format!(
        "WITH candidates(id) AS ({candidates}),
         unresolved AS MATERIALIZED (
             SELECT request.*
             FROM candidates
             CROSS JOIN claims request ON request.id=candidates.id
             WHERE request.kind='gate.requested'
               AND json_extract(request.body, '$.fields.reviewer') IS NOT NULL
               AND (?1 IS NULL OR {effective_reviewer}=?1)
               AND NOT EXISTS (
                   SELECT 1 FROM claims result INDEXED BY claims_human_gate_answer_request_index
                   WHERE {})
               AND NOT EXISTS (
                   SELECT 1 FROM claims newer INDEXED BY claims_human_gate_episode_index
                   WHERE newer.kind='gate.requested'
                     AND json_extract(newer.body, '$.fields.reviewer') IS NOT NULL
                     AND {} AND {})
         )
         SELECT pending.id, pending.store_index, pending.batch_id, pending.subject,
                pending.kind, pending.origin, pending.actor, pending.body,
                pending.predecessors, pending.accepted_at_unix_ms,
                first.accepted_at_unix_ms
         FROM unresolved pending
         JOIN claims first ON first.id=(
             SELECT earliest.id FROM claims earliest INDEXED BY claims_human_gate_episode_index
             WHERE earliest.kind='gate.requested'
               AND json_extract(earliest.body, '$.fields.reviewer') IS NOT NULL
               AND {}
             ORDER BY {} LIMIT 1)
         ORDER BY {}",
        reviewer_answer_sql("result"),
        episode("newer", "request"),
        canonical::after_sql("newer", "request"),
        episode("earliest", "pending"),
        canonical::order_sql("earliest", false),
        canonical::order_sql("first", false),
    ))
}

/// Append one reassignment per current, unanswered request under the rename transaction's
/// writer lock. The original request is both evidence and a causal predecessor; no gate is
/// re-asked, and no historical body or actor is rewritten.
pub fn reassign_reviews_tx(
    tx: &Transaction<'_>,
    origin: &str,
    input: &PersonRenameRequest,
) -> Result<Vec<String>, St3Error> {
    let reviews = attention::pending_human_reviews_tx(tx, Some(&input.old_person)).map_err(internal)?;
    let mut owners = Vec::with_capacity(reviews.len());
    for review in reviews {
        let request = claim_by_id_tx(tx, &review.request)
            .map_err(internal)?
            .ok_or_else(|| St3Error::new("missing-review-request", "the pending review request is missing"))?;
        let key = canonical_hash(&(
            "st3.person-review-reassignment.v1",
            &input.idempotency_key,
            &request.id,
        ))
        .map_err(internal)?;
        let claim_input = ClaimInput {
            subject: request.subject.clone(),
            kind: "gate.reviewer-reassigned".into(),
            actor: Some(input.actor.clone()),
            fields: BTreeMap::from([
                ("request".into(), json!(request.id)),
                ("owner".into(), json!(review.owner)),
                ("previous_reviewer".into(), json!(input.old_person)),
                ("reviewer".into(), json!(input.new_person)),
            ]),
            evidence: vec![request.id.clone()],
            expected_subject: None,
            idempotency_key: Some(format!("person-review-reassignment:{key}")),
        };
        let (operation_id, request_digest) = claim_operation(&claim_input)?
            .expect("a reviewer reassignment always has an idempotency key");
        if let Some((stored_digest, _, state)) = operation_tx(tx, &operation_id).map_err(internal)? {
            if state != "active" || stored_digest != request_digest {
                return Err(St3Error::new(
                    "idempotency-conflict",
                    "this review reassignment key already names a different repair",
                ));
            }
            continue;
        }
        checkpointed_operation_outcome(tx, &operation_id, &request_digest)?;
        let body = json!({
            "fields": claim_input.fields,
            "evidence": claim_input.evidence,
            "_operation": {"id": operation_id, "request_digest": request_digest},
        });
        let mut predecessors = latest_claim_id_tx(tx, &request.subject)
            .map_err(internal)?
            .into_iter()
            .collect::<Vec<_>>();
        if !predecessors.contains(&request.id) {
            predecessors.push(request.id);
        }
        let claim = append_claim_tx(
            tx,
            origin,
            &request.subject,
            &claim_input.kind,
            Some(&input.actor),
            &body,
            &predecessors,
            None,
        )
        .map_err(claim_append_error)?;
        register_operation_tx(tx, &claim).map_err(internal)?;
        owners.push(review.owner);
    }
    owners.sort();
    owners.dedup();
    Ok(owners)
}

