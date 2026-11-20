//! Native attach consent is a paired-person operation, not a respondent projection.
use super::*;

fn key_parts(key: &str) -> Option<(&str, &str)> {
    let (preparation, digest) = key.strip_prefix("native-attach/")?.split_once('/')?;
    let uuid = uuid::Uuid::parse_str(preparation).ok()?;
    let mut encoded = uuid::Uuid::encode_buffer();
    if uuid.hyphenated().encode_lower(&mut encoded) != preparation
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    Some((preparation, digest))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn admit(
    tx: &Transaction<'_>,
    ask: &ClaimRecord,
    view: &StepRunView,
    input: &PersonStepResponse,
    cancel: bool,
    assistant: bool,
    device: Option<(&str, &str)>,
) -> Result<Value, FractalError> {
    let refused = || FractalError::new("forbidden", "native attach requires the assigned person's explicit answer from a paired device with control.work");
    let now = now_ms();
    let fields = &ask.body["fields"];
    if cancel || assistant || input.delegation.is_some()
        || !crate::model::person_subject(&input.actor)
        || fields["person"] != input.actor
        || view.assigned_to.as_deref() != Some(input.actor.as_str())
        || input.episode.as_deref() != Some(ask.id.as_str())
        || input.answer.as_ref().and_then(|answer| answer.id.as_deref()).is_none()
        || !fields["default_answer"].is_null()
        || fields["deadline_unix_ms"].as_u64().is_none_or(|expiry| u128::from(expiry) <= now)
    {
        return Err(refused());
    }
    // A dismissal or decline is admissible person feedback, not attach acceptance.
    // The commit boundary must check the named accepting answer in addition to this receipt.
    let (preparation, candidate_digest) = fields["key"].as_str().and_then(key_parts).ok_or_else(refused)?;
    let (grant_subject, session_actor) = device.ok_or_else(refused)?;
    let paired = tx.query_row(
        /* sqlite-large-value: allow-for-now lv-he31a6c0e2b9c owner=fractal_missions::store::person_work::native_attach TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='custom.client.pairing-completed' ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
        [grant_subject], claim_from_row,
    ).optional().map_err(internal)?.ok_or_else(refused)?;
    let grant = &paired.body["fields"];
    if grant["person_id"] != input.actor || grant["session_actor"] != session_actor
        || grant["expires_at_unix_ms"].as_u64().is_none_or(|expiry| u128::from(expiry) <= now)
        || !grant["scopes"].as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope == "control.work"))
        || !fractal_store::pairing::completion_is_trusted(tx, &paired.id, &paired.origin, paired.actor.as_deref(), grant, now).map_err(internal)?
        || fractal_store::pairing::completion_is_revoked(tx, &paired.subject, &paired.id, &paired.origin).map_err(internal)?
    {
        return Err(refused());
    }
    Ok(json!({"ask":ask.subject,"episode":ask.id,"person":input.actor,
        "preparation":preparation,"candidate_digest":candidate_digest,
        "grant_id":paired.id,"grant_subject":paired.subject}))
}

#[cfg(test)]
#[path = "native_attach_tests.rs"]
mod tests;
