use super::*;

#[cfg(test)]
mod tests;

/// Immutable native ask identity; absent legacy bindings require explicit recovery.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AskContinuityBinding {
    pub version: u32,
    pub step: String,
    pub contract_hash: String,
    pub input_digest: String,
    pub authority_digest: String,
    pub asker: String,
    pub person: String,
    pub title: String,
    pub question: String,
    pub request: Value,
    pub deadline_ms: Option<u64>,
    pub default_answer: Option<String>,
}

pub fn continuity_step_binding_tx(connection: &Connection, step: &StepRunView) -> Result<(String, String, String)> {
    let mission = fractal_store::store::pinned_generation_mission_tx(connection, &step.generation)?
        .ok_or_else(|| anyhow::anyhow!("missing pinned mission for ask continuity"))?;
    let run = fractal_store::store::mission_run_header_tx(connection, &step.run)?;
    let hashes = fractal_store::store::step_hashes(&mission);
    let dependencies = fractal_store::store::flattened_dependencies(&mission);
    let mut pending = vec![step.step.clone()];
    let mut contracts = BTreeMap::new();
    while let Some(path) = pending.pop() {
        if contracts.contains_key(&path) { continue; }
        let hash = hashes.get(&path).ok_or_else(|| anyhow::anyhow!("missing ask step contract"))?;
        contracts.insert(path.clone(), hash);
        pending.extend(dependencies.get(&path).into_iter().flatten().cloned());
    }
    let digest = |value: Value| -> Result<String> {
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&value)?)))
    };
    Ok((digest(json!(contracts))?,
        digest(json!({"mission":mission.subject,"inputs":run.inputs,"workspace":run.workspace}))?,
        digest(json!({"authority":mission.authority,"owners":mission.revision_owners,
            "human_only":mission.revisions_human_only,"reviewer":mission.revision_reviewer,
            "requester":run.requester}))?))
}

pub(super) fn ask_binding_tx(connection: &Connection, step: &StepRunView, input: &PersonAskRequest, request: &Value) -> Result<AskContinuityBinding> {
    let (contract_hash, input_digest, authority_digest) = continuity_step_binding_tx(connection, step)?;
    Ok(AskContinuityBinding {
        version: 1, step: step.step.clone(), contract_hash, input_digest, authority_digest,
        asker: input.actor.clone(), person: input.person.clone(), title: input.title.clone(),
        question: input.reason.clone(), request: request.clone(), deadline_ms: input.deadline_ms,
        default_answer: input.default_answer.clone(),
    })
}

fn origin_asks_tx(connection: &Connection, source: &StepRunView) -> Result<Vec<ClaimRecord>> {
    connection.prepare_cached(&canonical_sql(
        "WITH RECURSIVE sources(subject,attempt) AS (
           SELECT ?1,?2 UNION SELECT json_extract(c.body,'$.fields.source_step_run'),
             coalesce(json_extract(c.body,'$.fields.source_attempt'),sources.attempt)
           FROM sources JOIN claims c ON c.subject=sources.subject AND c.kind='step-run.carried'
             AND json_extract(c.body,'$.fields.attempt')=sources.attempt)
         SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE kind='work.person-asked' AND EXISTS(SELECT 1 FROM sources
           WHERE json_extract(body,'$.fields.origin_step')=sources.subject
             AND json_extract(body,'$.fields.origin_attempt')=sources.attempt)
         ORDER BY CANONICAL_ASC(claims)"
    ))?.query_map(params![source.subject, source.attempt], claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn reusable_answer_tx(connection: &Connection, ask: &ClaimRecord) -> Result<bool> {
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN
         ('work.person-cancelled','work.person-reassigned'))", [&ask.subject], |row| row.get(0))?;
    if invalid { return Ok(false); }
    let Some(mut view) = step(connection, &ask.subject)? else { return Ok(false); };
    if view.status != "completed" { return Ok(false); }
    view.person_answers.clear();
    enrich_responses(connection, &mut view)?;
    if view.person_answers.is_empty() { return Ok(false); }
    for answer in &view.person_answers {
        if answer.status != "completed" { return Ok(false); }
        if let Some(grant) = &answer.assistant_grant_id {
            let revoked: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='custom.client.assistant.v1.revoked'
                  AND (subject=?1 OR subject=(SELECT subject FROM claims WHERE id=?1)))",
                [grant], |row| row.get(0))?;
            if revoked { return Ok(false); }
        }
    }
    let human: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN ('work.person-done','work.person-override')
          AND coalesce(json_extract(body,'$.fields.resolved_by'),'')!='deadline')",
        [&ask.subject], |row| row.get(0))?;
    Ok(human)
}

pub fn step_answers_bound_tx(connection: &Connection, source: &StepRunView, target: &StepRunView) -> Result<bool> {
    step_answers_eligible_tx(connection, source, target, false)
}

/// Explicit recovery confirms absent legacy bindings, never invalidated answers.
pub fn step_answers_eligible_tx(connection: &Connection, source: &StepRunView, target: &StepRunView, allow_legacy: bool) -> Result<bool> {
    let asks = origin_asks_tx(connection, source)?;
    if asks.is_empty() { return Ok(true); }
    let (contract, inputs, authority) = continuity_step_binding_tx(connection, target)?;
    for ask in asks {
        if !reusable_answer_tx(connection, &ask)? { return Ok(false); }
        let Some(value) = ask.body["fields"].get("continuity") else {
            if allow_legacy { continue; }
            return Ok(false);
        };
        let Ok(binding) = serde_json::from_value::<AskContinuityBinding>(value.clone()) else { return Ok(false); };
        if binding.version != 1 || binding.step != target.step || binding.contract_hash != contract
            || binding.input_digest != inputs || binding.authority_digest != authority
            || ask.actor.as_deref() != Some(binding.asker.as_str())
            || ask.body["fields"]["person"] != binding.person { return Ok(false); }
    }
    Ok(true)
}

pub(super) fn equivalent_answer_tx(connection: &Connection, origin: &StepRunView, binding: &AskContinuityBinding) -> Result<Option<ClaimRecord>> {
    let mut query = connection.prepare_cached(&canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE kind='work.person-asked' AND actor=?1
         AND json_extract(body,'$.fields.continuity.step')=?2
         AND json_extract(body,'$.fields.person')=?3 ORDER BY CANONICAL_DESC(claims)"))?;
    for ask in query.query_map(params![binding.asker, origin.step, binding.person], claim_from_row)? {
        let ask = ask?;
        if serde_json::from_value::<AskContinuityBinding>(ask.body["fields"]["continuity"].clone()).ok().as_ref() == Some(binding)
            && reusable_answer_tx(connection, &ask)? { return Ok(Some(ask)); }
    }
    Ok(None)
}
