//! Spontaneous work uses minimal ask-run projection and normal step verbs. A handoff is
//! a release with additive assignment fields; acknowledgment is progress plus a read receipt.
use super::*;


pub fn latest_handoff(
    connection: &Connection,
    view: &StepRunView,
    at: u128,
) -> Result<Option<ClaimRecord>, St3Error> {
    connection.query_row(/* sqlite-large-value: allow-for-now lv-hec57b03b4f2f owner=st3_store::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='work.released' AND json_extract(body,'$.fields.handoff_message') IS NOT NULL AND json_extract(body,'$.fields.attempt')=?2 AND CAST(accepted_at_unix_ms AS INTEGER)<=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
        params![view.subject, view.attempt, at as i64], claim_from_row).optional().map_err(internal)
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
) -> Result<bool, St3Error> {
    latest_handoff(connection, view, now_ms())?
        .map(|handoff| {
            acknowledged(connection, &handoff, now_ms())
                .map(|ack| !ack)
                .map_err(internal)
        })
        .transpose()
        .map(|pending| pending.unwrap_or(false))
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

pub fn project_start(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<(), St3Error> {
    let fields = &claim.body["fields"];
    let mission: MissionSpec =
        serde_json::from_value(fields["mission_spec"].clone()).map_err(internal)?;
    let run = claim.subject.trim_start_matches("mission-run/");
    let generation = fields["current_generation"]
        .as_str()
        .ok_or_else(|| {
            St3Error::new(
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
        St3Error::new(
            "invalid-mission-run-claim",
            "a minimal work run needs its work step",
        )
    })?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h973335df7ab3 owner=st3_store::store::adhoc_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,assignee,available_to,agentless,title,goals,created_at_unix_ms,updated_at_unix_ms,constraints) VALUES(?1,?2,?3,'work',?4,'ready',1,?5,'[]',0,?6,?7,?8,?8,'[]')",
        params![subject, run, generation, step.definition_hash, claim.actor, fields["ad_hoc_title"].as_str(), serde_json::to_string(&vec![fields["ad_hoc_title"].as_str().unwrap_or_default()]).map_err(internal)?, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
    Ok(())
}
