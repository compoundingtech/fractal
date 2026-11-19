use super::*;
mod human_gates;
mod root_gates;
pub use root_gates::{root_gate_evidence_tx, eligible_root_gate_requests_tx, carry_root_gate_answers_tx};


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

/// A person's one-time recovery confirmation, bound to the recovered step's native context.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StepPersonConfirmation {
    version: u32,
    step: String,
    contract_hash: String,
    input_digest: String,
    authority_digest: String,
    evidence: Vec<String>,
}

pub fn legacy_step_confirmation_tx(connection: &Connection, source: &StepRunView, target: &StepRunView) -> Result<Option<Value>> {
    let evidence = person_evidence_tx(connection, source)?.into_iter()
        .filter(|record| record.body["fields"]["continuity"].is_null())
        .map(|record| record.id).collect::<Vec<_>>();
    if evidence.is_empty() { return Ok(None); }
    let (contract_hash, input_digest, authority_digest) = continuity_step_binding_tx(connection, target)?;
    Ok(Some(serde_json::to_value(StepPersonConfirmation {
        version: 1, step: target.step.clone(), contract_hash, input_digest, authority_digest, evidence,
    })?))
}

fn inherited_step_confirmations_tx(connection: &Connection, source: &StepRunView, target: &StepRunView) -> Result<BTreeSet<String>> {
    let expected = continuity_step_binding_tx(connection, target)?;
    let mut query = connection.prepare_cached(
        /* sqlite-large-value: allow-for-now lv-h909c95573ae4 owner=fractal_store::store::person_work::continuity TODO=trace-statement-and-value-representation */
        "WITH RECURSIVE sources(subject,attempt) AS (
           SELECT ?1,?2 UNION SELECT json_extract(c.body,'$.fields.source_step_run'),
             coalesce(json_extract(c.body,'$.fields.source_attempt'),sources.attempt)
           FROM sources JOIN claims c ON c.subject=sources.subject AND c.kind='step-run.carried'
             AND json_extract(c.body,'$.fields.attempt')=sources.attempt)
         SELECT c.body FROM sources JOIN claims c ON c.subject=sources.subject
           AND c.kind='step-run.carried' AND json_extract(c.body,'$.fields.attempt')=sources.attempt
           AND c.actor LIKE 'person/%' AND json_type(c.body,'$.fields.person_continuity')='object'"
    )?;
    let mut confirmed = BTreeSet::new();
    for body in query.query_map(params![source.subject,source.attempt], |row| row.get::<_,String>(0))? {
        let body: Value = serde_json::from_str(&body?)?;
        let Ok(receipt) = serde_json::from_value::<StepPersonConfirmation>(body["fields"]["person_continuity"].clone()) else { continue; };
        if receipt.version == 1 && receipt.step == target.step
            && (receipt.contract_hash,receipt.input_digest,receipt.authority_digest) == expected {
            confirmed.extend(receipt.evidence);
        }
    }
    Ok(confirmed)
}

pub fn continuity_step_binding_tx(connection: &Connection, step: &StepRunView) -> Result<(String, String, String)> {
    continuity_context_binding_tx(connection, &step.run, &step.generation, Some(&step.step))
}

fn continuity_context_binding_tx(connection: &Connection, run_id: &str, generation: &str, path: Option<&str>) -> Result<(String, String, String)> {
    let mission = crate::store::pinned_generation_mission_tx(connection, generation)?
        .ok_or_else(|| anyhow::anyhow!("missing pinned mission for ask continuity"))?;
    let run = crate::store::mission_run_header_tx(connection, run_id.trim_start_matches("mission-run/"))?;
    let hashes = crate::store::step_hashes(&mission);
    let dependencies = crate::store::flattened_dependencies(&mission);
    let mut pending: Vec<String> = path.map_or_else(|| hashes.keys().cloned().collect(), |path| vec![path.to_owned()]);
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
    let contract = if path.is_none() {
        json!({"steps":contracts,"gates":mission.gates,"completion":mission.completion,
            "constraints":mission.constraints,"agent_constraints":mission.agent_constraints,
            "declarations":mission.declarations_kdl,"work_selector":mission.work_selector})
    } else { json!(contracts) };
    Ok((digest(contract)?,
        digest(json!({"mission":mission.subject,"inputs":run.inputs,"workspace":run.workspace}))?,
        digest(json!({"authority":mission.authority,"owners":mission.revision_owners,
            "human_only":mission.revisions_human_only,"reviewer":mission.revision_reviewer,
            "requester":run.requester}))?))
}

/// Synthetic asks have durable request identity, but no originating declared step to bind.
pub fn ask_binding_tx(connection: &Connection, step: &StepRunView, input: &PersonAskRequest, request: &Value) -> Result<Option<AskContinuityBinding>> {
    if super::request(connection, &step.subject)?.is_some() { return Ok(None); }
    let (contract_hash, input_digest, authority_digest) = continuity_step_binding_tx(connection, step)?;
    Ok(Some(AskContinuityBinding {
        version: 1, step: step.step.clone(), contract_hash, input_digest, authority_digest,
        asker: input.actor.clone(), person: input.person.clone(), title: input.title.clone(),
        question: input.reason.clone(), request: request.clone(), deadline_ms: input.deadline_ms,
        default_answer: input.default_answer.clone(),
    }))
}

pub fn direct_person_binding_tx(connection: &Connection, step: &StepRunView, actor: &str) -> Result<AskContinuityBinding> {
    let (contract_hash, input_digest, authority_digest) = continuity_step_binding_tx(connection, step)?;
    Ok(AskContinuityBinding {
        version: 1, step: step.step.clone(), contract_hash, input_digest, authority_digest,
        asker: actor.into(), person: step.assigned_to.clone().unwrap_or_else(|| actor.into()),
        title: step.title.clone().unwrap_or_else(|| step.step.clone()), question: step.goals.join("\n"),
        request: json!({"kind":"authored-person-step"}), deadline_ms: None, default_answer: None,
    })
}

pub fn person_evidence_tx(connection: &Connection, source: &StepRunView) -> Result<Vec<ClaimRecord>> {
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h4f30e7197200 owner=fractal_store::store::person_work::continuity TODO=trace-statement-and-value-representation */ &canonical_sql(
        "WITH RECURSIVE sources(subject,attempt) AS (
           SELECT ?1,?2 UNION SELECT json_extract(c.body,'$.fields.source_step_run'),
             coalesce(json_extract(c.body,'$.fields.source_attempt'),sources.attempt)
           FROM sources JOIN claims c ON c.subject=sources.subject AND c.kind='step-run.carried'
             AND json_extract(c.body,'$.fields.attempt')=sources.attempt)
         SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE id IN (
           SELECT evidence.id FROM sources CROSS JOIN claims evidence INDEXED BY claims_person_ask_origin_step_index
           WHERE evidence.kind='work.person-asked'
             AND json_extract(evidence.body,'$.fields.origin_step')=sources.subject
             AND json_extract(evidence.body,'$.fields.origin_attempt')=sources.attempt
           UNION
           SELECT evidence.id FROM sources CROSS JOIN claims evidence
           WHERE evidence.kind='work.person-done' AND evidence.subject=sources.subject
             AND json_extract(evidence.body,'$.fields.attempt')=sources.attempt
             AND NOT EXISTS(SELECT 1 FROM claims request
               WHERE request.subject=evidence.subject AND request.kind='work.person-asked')
           UNION
           SELECT evidence.id FROM sources CROSS JOIN claims evidence INDEXED BY claims_human_gate_episode_index
           WHERE evidence.kind='gate.requested'
             AND json_extract(evidence.body,'$.fields.reviewer') IS NOT NULL
             AND json_type(evidence.body,'$.fields.reviewer')='text'
             AND json_extract(evidence.body,'$.fields.owner')=sources.subject
             AND json_extract(evidence.body,'$.fields.attempt')=sources.attempt)
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
    let asks = person_evidence_tx(connection, source)?;
    if asks.is_empty() { return Ok(true); }
    let (contract, inputs, authority) = continuity_step_binding_tx(connection, target)?;
    let inherited = inherited_step_confirmations_tx(connection, source, target)?;
    for ask in asks {
        if ask.kind == "gate.requested" {
            let Some(answer) = crate::store::human_review_answer_tx(connection, &ask.id)? else { return Ok(false); };
            if answer.body["fields"]["verdict"] != "pass" { return Ok(false); }
        } else if !reusable_answer_tx(connection, &ask)? { return Ok(false); }
        let Some(value) = ask.body["fields"].get("continuity").filter(|value| !value.is_null()) else {
            if allow_legacy || inherited.contains(&ask.id) { continue; }
            return Ok(false);
        };
        let Ok(binding) = serde_json::from_value::<AskContinuityBinding>(value.clone()) else { return Ok(false); };
        if binding.version != 1 || binding.step != target.step || binding.contract_hash != contract
            || binding.input_digest != inputs || binding.authority_digest != authority
            || (ask.kind != "gate.requested" && ask.actor.as_deref() != Some(binding.asker.as_str()))
            || (ask.kind == "gate.requested" && ask.body["fields"]["reviewer"] != binding.person)
            || (ask.kind == "work.person-asked" && ask.body["fields"]["person"] != binding.person) { return Ok(false); }
    }
    Ok(true)
}

pub fn equivalent_answer_tx(connection: &Connection, origin: &StepRunView, binding: &AskContinuityBinding) -> Result<Option<ClaimRecord>> {
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hdb770a2c69e8 owner=fractal_store::store::person_work::continuity TODO=trace-statement-and-value-representation */ &canonical_sql(
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
