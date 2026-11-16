use super::*;

pub(super) fn gate_binding_tx(connection: &Connection, run: &MissionRunView, owner: &str, fields: &BTreeMap<String, Value>) -> Result<AskContinuityBinding> {
    let target = run.steps.iter().find(|step| step.subject == owner);
    let (contract_hash, input_digest, authority_digest) = continuity_context_binding_tx(
        connection, &run.subject, &run.generation, target.map(|step| step.step.as_str()),
    )?;
    Ok(AskContinuityBinding {
        version: 1, step: target.map_or_else(|| "$run".into(), |step| step.step.clone()),
        contract_hash, input_digest, authority_digest, asker: "daemon/runtime".into(),
        person: fields["reviewer"].as_str().unwrap_or_default().into(),
        title: String::new(), question: fields["question"].as_str().unwrap_or_default().into(),
        request: json!({"kind":"human-gate","mode":fields["mode"],
            "decisions":fields["decisions"],"review_targets":fields["review_targets"]}),
        deadline_ms: None, default_answer: None,
    })
}

impl Store {
    /// Bind authored human gates before emission and reuse only authenticated original approval.
    pub fn human_gate_continuity(&self, run: &MissionRunView, owner: &str, fields: &BTreeMap<String, Value>) -> Result<(Value, Option<ClaimRecord>), FractalError> {
        self.with_run_fence(owner, |tx| {
            let target = run.steps.iter().find(|step| step.subject == owner);
            let binding = gate_binding_tx(tx, run, owner, fields).map_err(internal)?;
            let value = serde_json::to_value(&binding).map_err(internal)?;
            if target.is_none() {
                for request in root_gate_evidence_tx(tx, run).map_err(internal)? {
                    let Some(answer) = crate::store::human_review_answer_tx(tx, &request.id).map_err(internal)? else { continue; };
                    if answer.body["fields"]["verdict"] == "pass"
                        && super::root_gates::root_reference_matches_tx(tx,run,&request,&answer,&value).map_err(internal)? {
                        return Ok((value,Some(request)));
                    }
                }
            }
            let mut query = tx.prepare_cached(&canonical_sql(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                 FROM claims WHERE kind='gate.requested' AND json_extract(body,'$.fields.reviewer')=?1
                   AND json_extract(body,'$.fields.continuity.step')=?2 ORDER BY CANONICAL_DESC(claims)"
            )).map_err(internal)?;
            for request in query.query_map(params![binding.person, binding.step], claim_from_row).map_err(internal)? {
                let request = request.map_err(internal)?;
                if request.body["fields"]["continuity"] != value { continue; }
                let Some(answer) = crate::store::human_review_answer_tx(tx, &request.id).map_err(internal)? else { continue; };
                // A refusal or request for changes is not approval to repeat effects.
                if answer.body["fields"]["verdict"] != "pass" { continue; }
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
                        &request,&answer,&value,false,None).map_err(internal)?;
                }
                return Ok((value, Some(request)));
            }
            Ok((value, None))
        })
    }
}
