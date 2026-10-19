//! Bounded recording of a person's prior instruction. The actor remains the agent.
use super::*;
use crate::model::{DelegationPolicyRequest, DelegationProof};

pub const ACTIONS: &[&str] = &["answer-ask", "close-item", "record-go-stop"];

pub fn denied(message: impl Into<String>) -> St3Error {
    St3Error::new("delegation-refused", message)
}

pub fn policy_actions(fields: &BTreeMap<String, Value>) -> Result<Vec<String>, St3Error> {
    let actions: Vec<String> =
        serde_json::from_value(fields.get("actions").cloned().unwrap_or(Value::Null))
            .map_err(|_| denied("a policy needs an actions array"))?;
    let unique: BTreeSet<_> = actions.iter().collect();
    if unique.len() != actions.len()
        || actions
            .iter()
            .any(|action| !ACTIONS.contains(&action.as_str()))
    {
        return Err(denied(
            "only answer-ask, close-item and record-go-stop may be enabled",
        ));
    }
    Ok(actions)
}

impl Store {
    pub fn delegated_update_request(
        &self,
        proof: &DelegationProof,
    ) -> Result<ClaimRecord, St3Error> {
        let request = claim_by_id_tx(&self.readers.get(), &proof.episode)
            .map_err(internal)?
            .filter(|claim| {
                claim.kind == "work.person-asked"
                    && claim.body["fields"]["person"] == proof.person
                    && person_work::is_update(claim)
            })
            .ok_or_else(|| {
                denied("only an informational update may be closed through attention")
            })?;
        Ok(request)
    }

    pub fn delegation_evidence(
        &self,
        evidence: &mut Vec<String>,
        proof: &DelegationProof,
    ) -> Result<(), St3Error> {
        add_evidence(&self.readers.get(), evidence, proof)
    }
    pub fn set_delegation_policy(
        &self,
        input: &DelegationPolicyRequest,
    ) -> Result<ClaimRecord, St3Error> {
        self.append_claim(&ClaimInput {
            subject: input.person.clone(),
            kind: "person.delegation-set".into(),
            actor: Some(input.actor.clone()),
            fields: BTreeMap::from([("actions".into(), json!(input.actions))]),
            evidence: input.evidence.clone(),
            expected_subject: None,
            idempotency_key: Some(input.idempotency_key.clone()),
        })
    }
}

pub fn add_fields(fields: &mut BTreeMap<String, Value>, proof: &DelegationProof) {
    fields.insert("acted_for".into(), json!(proof.person));
    fields.insert("delegation".into(), json!(proof));
}

pub fn add_evidence(
    connection: &Connection,
    evidence: &mut Vec<String>,
    proof: &DelegationProof,
) -> Result<(), St3Error> {
    let instruction = message_sent(connection, &proof.message)?;
    for reference in [&instruction.id, &proof.policy] {
        if !evidence.contains(reference) {
            evidence.push(reference.clone());
        }
    }
    Ok(())
}

pub fn message_sent(connection: &Connection, subject: &str) -> Result<ClaimRecord, St3Error> {
    connection.query_row(/* sqlite-large-value: allow-for-now lv-he814e59cf457 owner=st3_store::store::delegation TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='message.sent' ORDER BY CANONICAL_ASC(claims) LIMIT 1"), [subject], claim_from_row)
        .optional().map_err(internal)?.ok_or_else(|| denied("instruction and target messages need an original sent claim"))
}

/// Runs under the writer lock on both specialized and generic local claim paths.
pub fn validate_write(
    tx: &Transaction<'_>,
    subject: &str,
    kind: &str,
    actor: Option<&str>,
    fields: &BTreeMap<String, Value>,
    evidence: &[String],
) -> Result<(), St3Error> {
    if kind == "person.delegation-set" {
        if !subject.starts_with("person/")
            || subject.matches('/').count() != 1
            || actor != Some(subject)
        {
            return Err(denied(
                "only the person may establish or replace their delegation policy",
            ));
        }
        policy_actions(fields)?;
        if evidence.is_empty() {
            return Err(denied("a policy needs its decision evidence"));
        }
        return Ok(());
    }
    let Some(value) = fields.get("delegation") else {
        if fields.contains_key("acted_for")
            || (kind == "work.person-done"
                && actor.is_some_and(|actor| actor.starts_with("agent/")))
        {
            return Err(denied(
                "an agent completing person work needs explicit delegation evidence",
            ));
        }
        return Ok(());
    };
    let proof: DelegationProof =
        serde_json::from_value(value.clone()).map_err(|_| denied("invalid delegation proof"))?;
    if !actor.is_some_and(|actor| actor.starts_with("agent/"))
        || fields.get("acted_for") != Some(&json!(proof.person))
    {
        return Err(denied(
            "a delegated action records an agent actor and the person acted for",
        ));
    }
    let policy = tx.query_row(/* sqlite-large-value: allow-for-now lv-h743ec088d6c1 owner=st3_store::store::delegation TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='person.delegation-set' ORDER BY CANONICAL_DESC(claims) LIMIT 1"), [&proof.person], claim_from_row)
        .optional().map_err(internal)?.ok_or_else(|| denied("the person has no delegation policy"))?;
    if policy.id != proof.policy || policy.actor.as_deref() != Some(&proof.person) {
        return Err(denied(
            "the policy changed or was not established by this person",
        ));
    }
    let action = match kind {
        "work.person-done" => {
            let mut step = person_work::step(tx, subject)
                .map_err(internal)?
                .ok_or_else(|| denied("missing person step"))?;
            let ask = person_work::request(tx, subject)
                .map_err(internal)?
                .ok_or_else(|| denied("authored person steps are outside the allowed list"))?;
            if step.assigned_to.as_deref() != Some(&proof.person)
                || proof.episode != ask.id
                || fields.get("episode") != Some(&json!(ask.id))
                || !person_work::current(tx, &ask, now_ms()).map_err(internal)?
            {
                return Err(denied("the ask belongs to another person or episode"));
            }
            apply_effective_step_state(tx, &mut step, now_ms()).map_err(internal)?;
            if step.status != "ready"
                || fields.get("attempt").and_then(Value::as_u64) != Some(step.attempt as u64)
                || super::adhoc_work::pending_handoff(tx, &step)?
            {
                return Err(denied(
                    "the person step is not ready or its handoff is unread",
                ));
            }
            // Generic claim callers receive the same structured answer validation.
            person_work::validate_delegated_answer(&ask, fields)?;
            if person_work::is_update(&ask) {
                "close-item"
            } else {
                "answer-ask"
            }
        }
        "gate.result" => {
            if fields.get("decision").and_then(Value::as_str) == Some("rejected")
                && fields
                    .get("reason")
                    .and_then(Value::as_str)
                    .is_none_or(|reason| reason.trim().is_empty())
            {
                return Err(denied(
                    "a delegated stop decision needs its rejection reason",
                ));
            }
            let request_id = fields
                .get("request")
                .and_then(Value::as_str)
                .ok_or_else(|| denied("a delegated review needs its exact request"))?;
            let request = claim_by_id_tx(tx, request_id)
                .map_err(internal)?
                .ok_or_else(|| denied("missing review request"))?;
            let review = current_human_review(tx, request.clone())
                .map_err(internal)?
                .ok_or_else(|| denied("the review revision is stale"))?;
            if request.kind != "gate.requested"
                || request.subject != subject
                || review.reviewer != proof.person
                || proof.episode != request.id
                || review.mode != "approve"
                || human_review_answer_tx(tx, &request.id)
                    .map_err(internal)?
                    .is_some()
                || !matches!(
                    (
                        fields.get("decision").and_then(Value::as_str),
                        fields.get("verdict").and_then(Value::as_str)
                    ),
                    (Some("approved"), Some("pass")) | (Some("rejected"), Some("fail"))
                )
            {
                return Err(denied(
                    "only an unanswered go/stop review for this person and revision may be delegated",
                ));
            }
            "record-go-stop"
        }
        "message.closed" => {
            let target = message_sent(tx, subject)?;
            if target.body["fields"]["to"] != proof.person || proof.episode != target.id {
                return Err(denied("the message belongs to another person or episode"));
            }
            "close-item"
        }
        _ => {
            return Err(denied(
                "this operation is outside the allowed delegation list",
            ));
        }
    };
    let actions: Vec<String> =
        serde_json::from_value(policy.body["fields"]["actions"].clone()).map_err(internal)?;
    if !actions.iter().any(|allowed| allowed == action) {
        return Err(denied(format!("the person has not enabled {action}")));
    }
    let instruction = message_sent(tx, &proof.message)?;
    if instruction.actor.as_deref() != Some(&proof.person)
        || instruction.body["fields"]["from"] != proof.person
    {
        return Err(denied(
            "the instruction's recorded sender must be the person acted for",
        ));
    }
    let mut content = instruction.body["fields"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if content.starts_with("doc/") {
        let (name, hash) = split_document_ref(&content)?;
        let bytes: Vec<u8> = tx.query_row("SELECT b.bytes FROM documents d JOIN blobs b ON b.hash=d.hash WHERE d.name=?1 AND d.hash=?2", params![name,hash], |row| row.get(0)).map_err(internal)?;
        content = String::from_utf8(bytes).map_err(internal)?;
    }
    if proof.quote.trim().is_empty() || !content.contains(&proof.quote) {
        return Err(denied(
            "the quote must occur verbatim in the person's original message",
        ));
    }
    if !evidence.contains(&instruction.id) || !evidence.contains(&proof.policy) {
        return Err(denied(
            "the completion must cite the instruction and policy",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::delegation_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::delegation_tests_shared_seats::{policy, proof, sent};

    

}
