use super::*;
use super::human_gates::gate_binding_tx;

/// Original run-level human requests, including originals carried into this generation.
pub fn root_gate_evidence_tx(connection: &Connection, source: &MissionRunView) -> Result<Vec<ClaimRecord>> {
    connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h929ee3c2fca9 owner=fractal_store::store::person_work::continuity::root_gates TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE kind='gate.requested' AND json_type(body,'$.fields.reviewer')='text'
           AND (json_extract(body,'$.fields.owner')=?1 OR id IN (
             SELECT json_extract(body,'$.fields.source_request') FROM claims
             WHERE subject=?1 AND kind='mission-run.gate-carried'
               AND json_extract(body,'$.fields.target_generation')=?2))
         ORDER BY CANONICAL_ASC(claims)"
    ))?.query_map(params![source.subject,source.generation], claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn original_confirmation_matches_tx(connection: &Connection, id: &str, request: &ClaimRecord, answer: &ClaimRecord, binding: &Value) -> Result<bool> {
    let receipt = connection.query_row(
        "SELECT actor,body FROM claims WHERE id=?1 AND kind='mission-run.gate-carried'",
        [id], |row| Ok((row.get::<_,Option<String>>(0)?,row.get::<_,String>(1)?)),
    ).optional()?;
    let Some((actor,body)) = receipt else { return Ok(false); };
    let body: Value = serde_json::from_str(&body)?;
    Ok(actor.as_deref().is_some_and(|actor| actor.starts_with("person/"))
        && body["fields"]["legacy_confirmed"] == true
        && body["fields"]["source_request"] == request.id
        && body["fields"]["source_answer"] == answer.id
        && body["fields"]["continuity"] == *binding)
}

fn root_confirmation_tx(connection: &Connection, run: &MissionRunView, request: &ClaimRecord, answer: &ClaimRecord, binding: &Value) -> Result<Option<String>> {
    let mut query = connection.prepare_cached(
        "SELECT id,body FROM claims WHERE subject=?1 AND kind='mission-run.gate-carried'
           AND json_extract(body,'$.fields.target_generation')=?2
           AND json_extract(body,'$.fields.source_request')=?3
           AND json_extract(body,'$.fields.source_answer')=?4"
    )?;
    for row in query.query_map(params![run.subject,run.generation,request.id,answer.id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))? {
        let (id,body) = row?;
        let body: Value = serde_json::from_str(&body)?;
        if body["fields"]["continuity"] != *binding { continue; }
        if original_confirmation_matches_tx(connection,&id,request,answer,binding)? {
            return Ok(Some(id));
        }
        if let Some(original) = body["fields"]["source_confirmation"].as_str()
            && original_confirmation_matches_tx(connection,original,request,answer,binding)? {
            return Ok(Some(original.to_owned()));
        }
    }
    Ok(None)
}

pub(super) fn root_reference_matches_tx(connection: &Connection, run: &MissionRunView, request: &ClaimRecord, answer: &ClaimRecord, binding: &Value) -> Result<bool> {
    if request.body["fields"]["continuity"].is_null() {
        return Ok(root_confirmation_tx(connection,run,request,answer,binding)?.is_some());
    }
    let mut query = connection.prepare_cached(
        "SELECT body FROM claims WHERE subject=?1 AND kind='mission-run.gate-carried'
           AND json_extract(body,'$.fields.target_generation')=?2
           AND json_extract(body,'$.fields.source_request')=?3
           AND json_extract(body,'$.fields.source_answer')=?4"
    )?;
    for body in query.query_map(params![run.subject,run.generation,request.id,answer.id], |row| row.get::<_,String>(0))? {
        let body: Value = serde_json::from_str(&body?)?;
        if body["fields"]["continuity"] == *binding { return Ok(true); }
    }
    Ok(false)
}

/// Confirming an old question cannot override changed contracts, inputs, or authority.
/// Standing authenticated refusals and feedback carry too; reference carry never changes their verdict.
pub fn eligible_root_gate_requests_tx(connection: &Connection, source: &MissionRunView, target: &MissionRunView, confirmed_request_ids: &BTreeSet<String>) -> Result<Vec<ClaimRecord>> {
    let requests = root_gate_evidence_tx(connection, source)?;
    if requests.is_empty() { return Ok(Vec::new()); }
    if continuity_context_binding_tx(connection,&source.subject,&source.generation,None)?
        != continuity_context_binding_tx(connection,&target.subject,&target.generation,None)? { return Ok(Vec::new()); }
    let mut eligible = Vec::new();
    for request in requests {
        let Some(answer) = crate::store::human_review_answer_tx(connection,&request.id)? else { continue; };
        let fields = serde_json::from_value::<BTreeMap<String,Value>>(request.body["fields"].clone())?;
        let expected = serde_json::to_value(gate_binding_tx(connection,source,&source.subject,&fields)?)?;
        let stored = &request.body["fields"]["continuity"];
        if !stored.is_null() {
            if *stored != expected { continue; }
        } else if !confirmed_request_ids.contains(&request.id)
            && !root_reference_matches_tx(connection,source,&request,&answer,&expected)? { continue; }
        eligible.push(request);
    }
    Ok(eligible)
}

pub(super) fn record_root_gate_carry_tx(
    tx: &Transaction<'_>, origin: &str, target: &MissionRunView, actor: &str,
    (request, answer, binding): (&ClaimRecord, &ClaimRecord, &Value),
    legacy_confirmed: bool, source_confirmation: Option<&str>,
) -> Result<()> {
    if root_reference_matches_tx(tx,target,request,answer,binding)? { return Ok(()); }
    let mut body = json!({"fields":{"source_run":request.body["fields"]["owner"],"source_request":request.id,
        "source_answer":answer.id,"target_generation":target.generation,"continuity":binding,
        "legacy_confirmed":legacy_confirmed}});
    let mut evidence = vec![request.id.clone(),answer.id.clone()];
    if let Some(confirmation) = source_confirmation {
        body["fields"]["source_confirmation"] = json!(confirmation);
        evidence.push(confirmation.to_owned());
    }
    body["evidence"] = json!(evidence);
    append_claim_tx(tx,origin,&target.subject,"mission-run.gate-carried",Some(actor),&body,&evidence,None)?;
    Ok(())
}

/// Recovery records original authenticated gate references, never a replacement gate.result.
pub fn carry_root_gate_answers_tx(tx: &Transaction<'_>, origin: &str, source: &MissionRunView, target: &MissionRunView, actor: &str, confirmed_request_ids: &BTreeSet<String>) -> Result<Vec<String>> {
    let mut carried = Vec::new();
    for request in eligible_root_gate_requests_tx(tx,source,target,confirmed_request_ids)? {
        let answer = crate::store::human_review_answer_tx(tx,&request.id)?
            .ok_or_else(|| anyhow::anyhow!("the authenticated root gate answer is no longer retained"))?;
        let fields = serde_json::from_value::<BTreeMap<String,Value>>(request.body["fields"].clone())?;
        let binding = serde_json::to_value(gate_binding_tx(tx,target,&target.subject,&fields)?)?;
        let legacy = request.body["fields"]["continuity"].is_null();
        let confirmed_here = legacy && confirmed_request_ids.contains(&request.id);
        if confirmed_here && !actor.starts_with("person/") { anyhow::bail!("legacy root gate recovery requires a person confirmation"); }
        let source_confirmation = if legacy && !confirmed_here {
            Some(root_confirmation_tx(tx,source,&request,&answer,&binding)?
                .ok_or_else(|| anyhow::anyhow!("the original person confirmation is no longer retained"))?)
        } else { None };
        record_root_gate_carry_tx(tx,origin,target,actor,(&request,&answer,&binding),confirmed_here,source_confirmation.as_deref())?;
        carried.push(request.id);
    }
    Ok(carried)
}
