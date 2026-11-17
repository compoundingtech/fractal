use super::*;

pub(super) fn gate_binding_tx(connection: &Connection, run: &MissionRunView, owner: &str, fields: &BTreeMap<String, Value>) -> Result<AskContinuityBinding> {
    let target = run.steps.iter().find(|step| step.subject == owner);
    let (contract_hash, input_digest, authority_digest) = continuity_context_binding_tx(
        connection, &run.subject, &run.generation, target.map(|step| step.step.as_str()),
    )?;
    // Match the human review reader's semantics for schema-valid legacy requests.
    let strings = |name: &str| {
        fields.get(name).and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    Ok(AskContinuityBinding {
        version: 1, step: target.map_or_else(|| "$run".into(), |step| step.step.clone()),
        contract_hash, input_digest, authority_digest, asker: "daemon/runtime".into(),
        person: fields.get("reviewer").and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("human gate continuity requires a reviewer"))?.into(),
        title: String::new(), question: fields.get("question").and_then(Value::as_str).unwrap_or("Approve this work?").into(),
        request: json!({"kind":"human-gate","mode":fields.get("mode").and_then(Value::as_str).unwrap_or("approve"),
            "decisions":strings("decisions"),"review_targets":strings("review_targets")}),
        deadline_ms: None, default_answer: None,
    })
}

impl Store {
    /// Bind authored human gates before emission and reuse only authenticated original approval.
    pub fn human_gate_continuity(&self, run: &MissionRunView, owner: &str, fields: &BTreeMap<String, Value>) -> Result<(Value, Option<ClaimRecord>), FractalError> {
        // Equivalent requests can originate in another run. Track the candidate
        // collection even when empty, and each candidate before looking for its answer.
        smallclaims::touched::note_read(|| "kind:gate.requested".to_owned());
        // The fenced callback can run on the batched writer thread. Its read
        // recorder is thread-local, so return that frontier to the evaluating caller.
        let (result, reads) = self.with_run_fence(owner, |tx| Ok::<_, FractalError>(smallclaims::touched::record(|| {
            let target = run.steps.iter().find(|step| step.subject == owner);
            let binding = gate_binding_tx(tx, run, owner, fields).map_err(internal)?;
            let value = serde_json::to_value(&binding).map_err(internal)?;
            if target.is_none() {
                for request in root_gate_evidence_tx(tx, run).map_err(internal)? {
                    smallclaims::touched::note_read(|| request.subject.clone());
                    let Some(answer) = crate::store::human_review_answer_tx(tx, &request.id).map_err(internal)? else { continue; };
                    if super::root_gates::root_reference_matches_tx(tx,run,&request,&answer,&value).map_err(internal)? {
                        return Ok((value,Some(request)));
                    }
                }
            }
            let mut query = tx.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbc159cbc677e owner=fractal_store::store::person_work::continuity::human_gates TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                 FROM claims WHERE kind='gate.requested' AND json_extract(body,'$.fields.reviewer')=?1
                   AND json_extract(body,'$.fields.continuity.step')=?2 ORDER BY CANONICAL_DESC(claims)"
            )).map_err(internal)?;
            for request in query.query_map(params![binding.person, binding.step], claim_from_row).map_err(internal)? {
                let request = request.map_err(internal)?;
                if request.body["fields"]["continuity"] != value { continue; }
                smallclaims::touched::note_read(|| request.subject.clone());
                let Some(answer) = crate::store::human_review_answer_tx(tx, &request.id).map_err(internal)? else { continue; };
                // Carry the standing decision, never promote a refusal to approval.
                if let Some(target) = target {
                    let recorded: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='step-run.carried'
                          AND json_extract(body,'$.fields.attempt')=?2
                          AND json_extract(body,'$.fields.source_gate_request')=?3)",
                        params![owner, target.attempt, request.id], |row| row.get(0),
                    ).map_err(internal)?;
                    if !recorded && request.body["fields"]["owner"] != owner {
                        append_claim_tx(tx, &self.origin, owner, "step-run.carried", Some("daemon/runtime"),
                            &json!({"fields":{"source_step_run":request.body["fields"]["owner"],
                                "source_attempt":request.body["fields"]["attempt"],"source_gate_request":request.id,
                                "attempt":target.attempt,"status":target.status,"worker_reported":target.worker_reported},
                                "evidence":[request.id,answer.id]}), &[request.id.clone(), answer.id], None)
                            .map_err(claim_append_error)?;
                    }
                } else if request.body["fields"]["owner"] != owner {
                    super::root_gates::record_root_gate_carry_tx(tx,&self.origin,run,"daemon/runtime",
                        (&request,&answer,&value),false,None).map_err(internal)?;
                }
                return Ok((value, Some(request)));
            }
            Ok((value, None))
        })))?;
        for key in reads {
            smallclaims::touched::note_read(|| key);
        }
        result
    }
}
