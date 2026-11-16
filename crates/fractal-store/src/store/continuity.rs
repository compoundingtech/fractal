//! Read-only previews and explicit recovery of original superseded-run evidence.
use super::*;

fn preview_hash(value: &Value) -> Result<String, FractalError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value).map_err(internal)?)))
}

fn stable_run_evidence(run: &MissionRunView) -> Result<Value, FractalError> {
    let mut evidence = serde_json::to_value(run).map_err(internal)?;
    if let Some(steps) = evidence["steps"].as_array_mut() {
        for step in steps {
            if let Some(fields) = step.as_object_mut() {
                // Presentation clocks are not evidence and must not expire a preview.
                fields.remove("execution_elapsed_ms");
                fields.remove("ready_age_ms");
                fields.remove("wake");
            }
        }
    }
    Ok(evidence)
}

fn native_person_evidence_tx(connection: &Connection, step: &StepRunView) -> Result<Vec<Value>, FractalError> {
    person_work::continuity::person_evidence_tx(connection, step).map_err(internal)?
        .into_iter().map(|request| {
            let gate_answer = if request.kind == "gate.requested" {
                human_review_answer_tx(connection, &request.id).map_err(internal)?
            } else {
                None
            };
            let legacy_unbound = request.body["fields"]["continuity"].is_null();
            Ok(json!({"request": request, "gate_answer": gate_answer,
                "legacy_unbound": legacy_unbound}))
        }).collect()
}

fn recovery_runs(
    connection: &Connection,
    run: &str,
) -> Result<(MissionRunView, MissionRunView), FractalError> {
    let source = mission_run_view_tx(connection, run.strip_prefix("mission-run/").unwrap_or(run))
        .map_err(internal)?;
    let mut next = mission_run_lineage_tx(connection, &source.subject).map_err(internal)?
        .superseded_by.ok_or_else(|| FractalError::new("mission-not-superseded",
            "recovery requires an original run with a recorded supersession"))?;
    let mut seen = BTreeSet::from([source.subject.clone()]);
    loop {
        if !seen.insert(next.clone()) {
            return Err(FractalError::new("invalid-mission-lineage", "supersession lineage contains a cycle"));
        }
        let target = mission_run_view_tx(connection, next.trim_start_matches("mission-run/"))
            .map_err(internal)?;
        match mission_run_lineage_tx(connection, &target.subject).map_err(internal)?.superseded_by {
            Some(successor) => next = successor,
            None => return Ok((source, target)),
        }
    }
}

fn recovery_preview_tx(connection: &Connection, run: &str) -> Result<Value, FractalError> {
    let (source, target) = recovery_runs(connection, run)?;
    let workspace = missions::resolved_supersession_workspace(connection, &source)?;
    // Recovery creates fresh pending subjects; it never rewrites already dispatched work.
    let mut pending = target.clone();
    pending.workspace = workspace.clone();
    for step in &mut pending.steps {
        step.status = "pending".to_owned();
    }
    let ordinary = missions::continuity_carryable_steps_tx(connection, &source, &pending, &BTreeSet::new())?;
    let legacy_candidates = source.steps.iter().map(|step| step.subject.clone()).collect();
    let confirmed = missions::continuity_carryable_steps_tx(connection, &source, &pending, &legacy_candidates)?;
    let declaration_refusal = missions::plan_continuity_declarations_tx(connection, &source, &pending, &confirmed)
        .err().map(|error| json!({"code": error.code, "message": error.message}));
    let workspace_matches = target.workspace == workspace;
    let target_active = target.ended().is_none();
    let target_idle = mission_supersession::supersession_idle(connection, &target.id)?;
    let mut work = Vec::new();
    for step in &source.steps {
        let person_evidence = native_person_evidence_tx(connection, step)?;
        if step.status != "completed" && step.person_answers.is_empty() && person_evidence.is_empty() {
            continue;
        }
        work.push(json!({
            "kind": "step", "evidence_id": step.subject,
            "source_step_run": step.subject, "step": step.step, "status": step.status,
            "definition_hash": step.definition_hash, "attempt": step.attempt,
            "answered_asks": step.person_answers, "person_evidence": person_evidence,
            "eligibility": if !target_active { "successor-terminal" }
                else if !target_idle { "successor-has-active-work" }
                else if declaration_refusal.is_some() { "declaration-transfer-refused" }
                else if ordinary.contains(&step.subject) { "eligible" }
                else if confirmed.contains(&step.subject) { "requires-person-confirmation" }
                else { "binding-or-state-ineligible" },
        }));
    }
    let root_requests = person_work::continuity::root_gate_evidence_tx(connection, &source).map_err(internal)?;
    let root_ids = root_requests.iter().map(|request| request.id.clone()).collect();
    let bound_roots: BTreeSet<String> = person_work::continuity::eligible_root_gate_requests_tx(
        connection, &source, &pending, &BTreeSet::new(),
    ).map_err(internal)?.into_iter().map(|request| request.id).collect();
    let confirmed_roots: BTreeSet<String> = person_work::continuity::eligible_root_gate_requests_tx(
        connection, &source, &pending, &root_ids,
    ).map_err(internal)?.into_iter().map(|request| request.id).collect();
    let mut root_evidence = Vec::new();
    for request in root_requests {
        let answer = human_review_answer_tx(connection, &request.id).map_err(internal)?;
        root_evidence.push(json!({
            "kind": "run-human-gate", "evidence_id": request.id,
            "eligibility": if !target_active { "successor-terminal" }
                else if !target_idle { "successor-has-active-work" }
                else if bound_roots.contains(&request.id) { "eligible" }
                else if confirmed_roots.contains(&request.id) { "requires-person-confirmation" }
                else { "binding-or-state-ineligible" },
            "request": request, "answer": answer,
        }));
    }
    let mut preview = json!({
        "source_run": source.subject, "source_generation": source.generation,
        "target_run": target.subject, "target_generation": target.generation,
        "current_resolved_workspace": workspace, "target_workspace": target.workspace,
        "workspace_matches": workspace_matches, "target_active": target_active, "target_idle": target_idle,
        "original_work": work, "original_root_gates": root_evidence,
        "recovery_refusal": declaration_refusal,
        // The complete observed source and target bind answers, input claim references,
        // authorities and execution state, not just their presentation labels.
        "source_evidence": stable_run_evidence(&source)?, "target_evidence": stable_run_evidence(&target)?,
    });
    preview["preview_hash"] = json!(preview_hash(&preview)?);
    Ok(preview)
}

impl Store {
    /// Inspect original evidence without creating a generation, answer, or completion.
    pub fn mission_continuity_preview(&self, run: &str) -> Result<Value, FractalError> {
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction().map_err(internal)?;
        recovery_preview_tx(&transaction, run)
    }

    /// Adopt only eligible original references into the current linked successor.
    /// Legacy answers require an explicit person's confirmation of the named originals.
    pub fn recover_mission_continuity(
        &self,
        run: &str,
        expected_preview_hash: &str,
        actor: &str,
        confirmed_legacy_evidence: &[String],
    ) -> Result<Value, FractalError> {
        self.with_all_runs_fenced_internal(|transaction| {
            let preview = recovery_preview_tx(transaction, run)?;
            if preview["preview_hash"].as_str() != Some(expected_preview_hash) {
                return Err(FractalError::new("stale-continuity-preview",
                    "continuity evidence or bindings changed; inspect a fresh preview"));
            }
            if !preview["recovery_refusal"].is_null() {
                return Err(FractalError::new("continuity-declaration-transfer-refused",
                    preview["recovery_refusal"].to_string()));
            }
            let (source, target) = recovery_runs(transaction, run)?;
            let owned: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='mission-run.created' AND origin=?2)",
                params![target.subject, self.origin], |row| row.get(0),
            ).map_err(internal)?;
            if !owned {
                return Err(FractalError::new("mission-run-not-owned", "recovery must run on the successor's owner"));
            }
            if preview["target_active"] != true || preview["target_idle"] != true {
                return Err(FractalError::new("continuity-target-ineligible",
                    "recovery requires an active successor with no executing work"));
            }
            if !confirmed_legacy_evidence.is_empty()
                && !actor.strip_prefix("person/").is_some_and(|name| !name.is_empty())
            {
                return Err(FractalError::new("person-confirmation-required",
                    "only an explicit person may confirm legacy unbound answers"));
            }
            let confirmations: BTreeSet<String> = confirmed_legacy_evidence.iter().cloned().collect();
            for original in &confirmations {
                if !["original_work", "original_root_gates"].into_iter().any(|collection| {
                    preview[collection].as_array().into_iter().flatten().any(|evidence| {
                        evidence["evidence_id"].as_str() == Some(original.as_str())
                            && evidence["eligibility"] == "requires-person-confirmation"
                    })
                }) {
                    return Err(FractalError::new("invalid-legacy-confirmation",
                        "confirmations must name previewed eligible unbound original evidence"));
                }
            }
            let mission = missions::mission_storage_id(transaction, &target.mission)?;
            let publication: String = transaction.query_row(
                "SELECT claim_id FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
                params![mission, target.revision], |row| row.get(0),
            ).map_err(internal)?;
            let fresh = self.create_superseding_mission_run_tx(
                transaction, &target, &publication, Some(&target.revision),
            )?;
            let step_confirmations = confirmations.iter().filter(|id| {
                preview["original_work"].as_array().into_iter().flatten()
                    .any(|evidence| evidence["evidence_id"].as_str() == Some(id.as_str()))
            }).cloned().collect();
            missions::carry_completed_run_steps_with_legacy_tx(
                transaction, &self.origin, actor, &source, &fresh, &step_confirmations,
            )?;
            let root_confirmations = confirmations.iter().filter(|id| {
                preview["original_root_gates"].as_array().into_iter().flatten()
                    .any(|evidence| evidence["evidence_id"].as_str() == Some(id.as_str()))
            }).cloned().collect();
            let root_references = person_work::continuity::carry_root_gate_answers_tx(
                transaction, &self.origin, &source, &fresh, actor, &root_confirmations,
            ).map_err(internal)?;
            for evidence in preview["original_root_gates"].as_array().into_iter().flatten() {
                let Some(id) = evidence["evidence_id"].as_str() else { continue };
                if (evidence["eligibility"] == "eligible" || confirmations.contains(id))
                    && !root_references.iter().any(|reference| reference == id)
                {
                    return Err(FractalError::new("continuity-root-gate-ineligible",
                        "a previewed original root approval could not be referenced by the fresh successor"));
                }
            }
            cancel_mission_run_tx(transaction, &self.origin, &target.subject,
                &format!("continuity recovered by reference into {}", fresh.subject),
                None, now_ms())?;
            Ok(json!({"source_run": source.subject, "recovered_run": fresh.subject,
                "preview_hash": expected_preview_hash, "confirmed_legacy_evidence": confirmations,
                "root_gate_references": root_references}))
        })
    }

    /// Inspect the same owner-local publication candidates without draining queues or pinning runs.
    pub fn mission_supersession_preview(&self) -> Result<Value, FractalError> {
        let connection = self.readers.get();
        let transaction = connection.unchecked_transaction().map_err(internal)?;
        let heads = {
            let mut query = transaction.prepare(
                "SELECT d.mission_id,d.revision,d.claim_id,c.store_index FROM mission_definitions d JOIN claims c ON c.id=d.claim_id ORDER BY d.mission_id",
            ).map_err(internal)?;
            query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, u64>(3)?)))
                .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
        };
        let mut runs = Vec::new();
        for (mission, revision, publication, index) in heads {
            let ids = {
                let mut query = transaction.prepare("SELECT id FROM mission_runs WHERE mission_id=?1 ORDER BY id").map_err(internal)?;
                query.query_map([&mission], |row| row.get::<_, String>(0)).map_err(internal)?
                    .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
            };
            let candidates = {
                let mut query = transaction.prepare(mission_supersession::SUPERSESSION_SOURCE_QUERY).map_err(internal)?;
                query.query_map(params![mission, revision, self.origin, index, publication,
                    mission_supersession::FAULT_SCOPE, Option::<String>::None,
                    serde_json::to_string(&ids).map_err(internal)?], |row| row.get::<_, String>(0))
                    .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?
            };
            for id in candidates {
                let source = mission_run_view_tx(&transaction, &id).map_err(internal)?;
                if mission_run_lineage_tx(&transaction, &source.subject).map_err(internal)?.superseded_by.is_some() {
                    continue;
                }
                let automatic = mission_supersession::automatic_supersession_policy(&transaction, &source)?.is_some();
                let idle = mission_supersession::supersession_idle(&transaction, &id)?;
                let mut person_evidence = Vec::new();
                for step in &source.steps {
                    person_evidence.extend(native_person_evidence_tx(&transaction, step)?);
                }
                let mut root_gate_evidence = Vec::new();
                for request in person_work::continuity::root_gate_evidence_tx(&transaction, &source).map_err(internal)? {
                    let answer = human_review_answer_tx(&transaction, &request.id).map_err(internal)?;
                    root_gate_evidence.push(json!({"kind": "run-human-gate", "evidence_id": request.id,
                        "request": request, "answer": answer}));
                }
                runs.push(json!({"source_run": source.subject, "source_revision": source.revision,
                    "target_revision": revision, "publication": publication,
                    "current_resolved_workspace": missions::resolved_supersession_workspace(&transaction, &source)?,
                    "automatic_policy_allows": automatic, "idle": idle, "person_evidence": person_evidence,
                    "root_gate_evidence": root_gate_evidence,
                    "completed_work": source.steps.iter().filter(|step| step.status == "completed").map(|step| &step.subject).collect::<Vec<_>>(),
                    "answered_asks": source.steps.iter().flat_map(|step| &step.person_answers).collect::<Vec<_>>() }));
            }
        }
        let mut preview = json!({"runs": runs});
        preview["preview_hash"] = json!(preview_hash(&preview)?);
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_hash_is_deterministic_and_binds_original_evidence() {
        let original = json!({"source_step_run": "step-run/original/work", "answer": "approved", "head": "a"});
        assert_eq!(preview_hash(&original).unwrap(), preview_hash(&original).unwrap());
        for field in ["source_step_run", "answer", "head"] {
            let mut changed = original.clone();
            changed[field] = json!("changed");
            assert_ne!(preview_hash(&original).unwrap(), preview_hash(&changed).unwrap());
        }
    }
}
