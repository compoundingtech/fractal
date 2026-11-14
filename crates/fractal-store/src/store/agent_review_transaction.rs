//! Dormant writer seam; FINAL must share this policy with generic append before opening the wire.
use super::agent_review_answer::{ReviewPolicy, validate};
use super::agent_review_decode::{incoming_answer, stored_request};
use super::*;

/// Snapshot loaded only inside the answer writer's transaction.
#[doc(hidden)]
pub struct ReviewOwnerSnapshot {
    pub responsible_agents: BTreeSet<String>,
    pub reviewer: String,
    pub agentless: bool,
}

/// Only fractal-missions::store::missions::review_owner_snapshot may implement `load`.
/// A savepoint isolates every mutation, even if the caller commits after an error.
/// Authentication remains the separate slice 16 boundary; this is not a public ingress.
#[doc(hidden)]
pub fn append_agent_review_answer_tx(
    tx: &Transaction<'_>,
    origin: &str,
    input: &ClaimInput,
    load: impl FnOnce(
        &Transaction<'_>,
        &str,
        &str,
    ) -> std::result::Result<ReviewOwnerSnapshot, FractalError>,
) -> std::result::Result<ClaimRecord, FractalError> {
    let operation = identities::claim_operation(input)?;
    if let Some(record) = claim_operation_outcome_tx(tx, operation.as_ref())? {
        return Ok(record);
    }
    let answer = incoming_answer(input)?;
    let record = claim_by_id_tx(tx, answer.binding.request)
        .map_err(internal)?
        .ok_or_else(|| FractalError::new("invalid-agent-review-claim", "request is not stored"))?;
    let request = stored_request(&record)?;
    let current = human_review_reassignment::reviewer_tx(tx, &record).map_err(internal)?;
    let scope = crate::references::Scope {
        run: BTreeSet::new(),
        intent: None,
    };
    let reviewer = crate::references::agent_review::resolve_reviewer(&scope, tx, &current)?;
    let snapshot = load(tx, request.owner, &reviewer)?;
    if snapshot.responsible_agents.is_empty() && !snapshot.agentless {
        return Err(FractalError::new(
            "reviewer-owner-unresolved",
            request.owner,
        ));
    }
    validate(
        &request.request(),
        &ReviewPolicy {
            resolved_reviewer: &snapshot.reviewer,
            resolved_responsible_agents: &snapshot.responsible_agents,
        },
        &answer,
    )
    .map_err(|error| FractalError::new("agent-review-answer-rejected", format!("{error:?}")))?;
    // Reuse exact owner binding and ancestor liveness rules, never human actor/delegation policy.
    human_review_currency(tx, record)
        .map_err(internal)?
        .map_err(|why| FractalError::new("agent-review-stale", why))?;
    let predecessor = latest_claim_id_tx(tx, &input.subject).map_err(internal)?;
    if input
        .expected_subject
        .as_ref()
        .is_some_and(|expected| expected != &predecessor)
    {
        return Err(FractalError::new("stale-subject", &input.subject));
    }
    for evidence in &input.evidence {
        if claim_by_id_tx(tx, evidence).map_err(internal)?.is_none() {
            return Err(FractalError::new("unknown-evidence", evidence));
        }
    }
    let mut body = json!({"fields": input.fields, "evidence": input.evidence});
    if let Some((id, request_digest)) = operation {
        body["_operation"] = json!({"id": id, "request_digest": request_digest});
    }
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h9376410940d5 owner=fractal_store::store::agent_review_transaction TODO=trace-statement-and-value-representation */ "SAVEPOINT agent_review_answer")
        .map_err(internal)?;
    let result = (|| {
        let record = append_claim_record_tx(
            tx,
            origin,
            &input.subject,
            &input.kind,
            input.actor.as_deref(),
            &body,
            &predecessor.into_iter().collect::<Vec<_>>(),
            None,
        )
        .map_err(claim_append_error)?;
        insert_event(tx, record.store_index, &input.kind, &input.subject, &body)
            .map_err(internal)?;
        projectors::project_claim(projectors::APPEND, tx, &record).map_err(internal)?;
        if !register_operation_tx(tx, &record).map_err(internal)? {
            return Err(FractalError::new(
                "idempotency-conflict",
                "answer operation conflicts",
            ));
        }
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h25f2de100e85 owner=fractal_store::store::agent_review_transaction TODO=trace-statement-and-value-representation */ "RELEASE agent_review_answer")
            .map_err(internal)?;
        Ok(record)
    })();
    if result.is_err() {
        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hf3bdff08c789 owner=fractal_store::store::agent_review_transaction TODO=trace-statement-and-value-representation */ "ROLLBACK TO agent_review_answer; RELEASE agent_review_answer")
            .map_err(internal)?;
    }
    result
}
