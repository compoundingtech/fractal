//! Spontaneous work uses minimal ask-run projection and normal step verbs. A handoff is
//! a release with additive assignment fields; acknowledgment is progress plus a read receipt.
use super::*;


pub fn latest_handoff(
    connection: &Connection,
    view: &StepRunView,
    at: u128,
) -> Result<Option<ClaimRecord>, FractalError> {
    let handoff = connection.query_row(/* sqlite-large-value: allow-for-now lv-hdf17be72f35e owner=fractal_store::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='work.released' AND json_extract(body,'$.fields.handoff_message') IS NOT NULL AND json_extract(body,'$.fields.attempt')=?2 AND CAST(accepted_at_unix_ms AS INTEGER)<=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
        params![view.subject, view.attempt, at as i64], claim_from_row).optional().map_err(internal)?;
    if let Some(handoff) = &handoff {
        let resolved: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='work.handoff-returned' AND json_extract(body,'$.fields.handoff_claim')=?2 AND CAST(accepted_at_unix_ms AS INTEGER)<=?3)",
            params![view.subject, handoff.id, at as i64], |row| row.get(0)).map_err(internal)?;
        if resolved { return Ok(None); }
    }
    Ok(handoff)
}

pub fn acknowledged(
    connection: &Connection,
    handoff: &ClaimRecord,
    at: u128,
) -> rusqlite::Result<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='work.progress' AND actor=?2 AND json_extract(body,'$.fields.handoff_acknowledged')=?3 AND CAST(accepted_at_unix_ms AS INTEGER)<=?4)",
        params![handoff.subject, handoff.body["fields"]["handoff_to"].as_str(), handoff.body["fields"]["handoff_message"].as_str(), at as i64], |row| row.get(0))
}

pub fn pending_handoff(
    connection: &Connection,
    view: &StepRunView,
) -> Result<bool, FractalError> {
    let handoff = latest_handoff(connection, view, now_ms())?;
    handoff_pending(connection, handoff, now_ms())
}

fn handoff_pending(
    connection: &Connection,
    handoff: Option<ClaimRecord>,
    at: u128,
) -> Result<bool, FractalError> {
    handoff
        .map(|handoff| acknowledged(connection, &handoff, at).map(|ack| !ack).map_err(internal))
        .transpose()
        .map(|pending| pending.unwrap_or(false))
}

impl Store {
    /// Ready demand for declared seats. Call only while pinned to the publication's graph cut.
    pub fn ready_work_cohort_count<'a>(
        &self,
        work: impl IntoIterator<Item = &'a StepRunView>,
        cohort: &[String],
        at: u128,
    ) -> Result<Option<usize>> {
        use super::identities::{IdentityKind, resolve_identity_conn};

        let connection = self.readers.get();
        let mut seats = BTreeSet::new();
        for agent in cohort {
            let Ok(identity) = resolve_identity_conn(&connection, IdentityKind::Agent, agent) else {
                return Ok(None);
            };
            if !person_work::declaration_live(&connection, &identity.identity)? {
                return Ok(None);
            }
            seats.insert(identity.identity);
        }
        let mut count = 0;
        for step in work {
            if step.status != "ready"
                || step.agentless
                || step.claimant.is_some()
                || step.blocked_reason.is_some()
                || !step.blockers.is_empty()
                || step.not_before_unix_ms.is_some_and(|due| due > at)
                || !step.assigned_to.iter().chain(&step.available_to).any(|agent| seats.contains(agent))
            {
                continue;
            }
            let handoff = latest_handoff(&connection, step, at)?;
            if !handoff_pending(&connection, handoff, at)? {
                count += 1;
            }
        }
        Ok(Some(count))
    }
}

pub fn enrich_handoff(
    connection: &Connection,
    view: &mut StepRunView,
    at: u128,
) -> rusqlite::Result<()> {
    // History snapshots must not display a transfer that happened after their boundary.
    let handoff = latest_handoff(connection, view, at)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    if let Some(handoff) = handoff.filter(|h| h.accepted_at_unix_ms <= at) {
        let fields = &handoff.body["fields"];
        let ack = acknowledged(connection, &handoff, at)?;
        view.constraints.push(format!("Handoff from {} to {} ({}): {}\nNote: {}\nAcknowledge: st work acknowledge {} --message {} --as {}",
            handoff.actor.as_deref().unwrap_or_default(), fields["handoff_to"].as_str().unwrap_or_default(), if ack { "acknowledged" } else { "awaiting acknowledgment" },
            fields["handoff_message"].as_str().unwrap_or_default(), fields["summary"].as_str().unwrap_or_default(), view.subject,
            fields["handoff_message"].as_str().unwrap_or_default(), fields["handoff_to"].as_str().unwrap_or_default()));
    }
    Ok(())
}

pub fn project_start(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<(), FractalError> {
    let fields = &claim.body["fields"];
    let mission: MissionSpec =
        serde_json::from_value(fields["mission_spec"].clone()).map_err(internal)?;
    let run = claim.subject.trim_start_matches("mission-run/");
    let generation = fields["current_generation"]
        .as_str()
        .ok_or_else(|| {
            FractalError::new(
                "invalid-mission-run-claim",
                "a minimal run needs a generation",
            )
        })?
        .trim_start_matches("run-generation/");
    person_work::project_minimal_run(
        tx,
        claim,
        &mission,
        run,
        generation,
        run,
        "spontaneous work",
    )?;
    let subject = format!("step-run/{generation}/work");
    let step = mission.steps.get("work").ok_or_else(|| {
        FractalError::new(
            "invalid-mission-run-claim",
            "a minimal work run needs its work step",
        )
    })?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h652937426473 owner=fractal_store::store::adhoc_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,assignee,available_to,agentless,title,goals,created_at_unix_ms,updated_at_unix_ms,constraints) VALUES(?1,?2,?3,'work',?4,'ready',1,?5,'[]',0,?6,?7,?8,?8,'[]')",
        params![subject, run, generation, step.definition_hash, claim.actor, fields["ad_hoc_title"].as_str(), serde_json::to_string(&vec![fields["ad_hoc_title"].as_str().unwrap_or_default()]).map_err(internal)?, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
    Ok(())
}
