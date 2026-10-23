//! The part of `store::adhoc_work` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::{WorkAcknowledgeRequest, WorkHandoffRequest, WorkStartRequest};
pub use fractal_delivery::store::adhoc_work::*;
pub use fractal_delivery::store::adhoc_work::{enrich_handoff, pending_handoff, project_start};

pub fn validate_recipient(
    tx: &Transaction<'_>,
    input: &WorkHandoffRequest,
    current: &StepRunView,
) -> Result<(), St3Error> {
    if input.note.trim().is_empty() {
        return Err(St3Error::new(
            "handoff-needs-note",
            "a handoff needs a nonempty note",
        ));
    }
    if input.to == input.actor
        || !(input.to.starts_with("agent/") || input.to.starts_with("person/"))
        || input.to.split('/').skip(1).any(str::is_empty)
    {
        return Err(St3Error::new(
            "invalid-handoff-recipient",
            "name another exact agent/ or person/ recipient",
        ));
    }
    if input.to.starts_with("agent/")
        && !person_work::declaration_live(tx, &input.to).map_err(internal)?
    {
        return Err(St3Error::new(
            "missing-handoff-recipient",
            "the recipient seat needs a live declaration",
        ));
    }
    if !matches!(current.status.as_str(), "claimed" | "working") {
        return Err(St3Error::new(
            "work-not-claimed",
            "only a running claim can be handed off",
        ));
    }
    // A parent's selector also controls its children; changing just one would split ownership.
    let children: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM step_runs WHERE generation_id=?1 AND instr(step_path,?2)=1)",
            params![
                generation_id_from_subject(&current.generation),
                format!("{}/", current.step)
            ],
            |row| row.get(0),
        )
        .map_err(internal)?;
    if children {
        return Err(St3Error::new(
            "handoff-has-children",
            "hand off a leaf step; a parent owns its child work",
        ));
    }
    Ok(())
}

pub fn transfer_tx(
    tx: &Transaction<'_>,
    origin: &str,
    current: &StepRunView,
    input: &WorkHandoffRequest,
    body: &mut Value,
) -> Result<(), St3Error> {
    let key = serde_json::to_vec(&(
        &current.subject,
        current.attempt,
        &input.actor,
        &input.idempotency_key,
    ))
    .map_err(internal)?;
    let message = format!("message/{}", &hex::encode(Sha256::digest(key))[..16]);
    body["fields"]["handoff_key"] = json!(input.idempotency_key);
    body["fields"]["handoff_request"] = serde_json::to_value(input).map_err(internal)?;
    body["fields"]["handoff_to"] = json!(input.to);
    body["fields"]["handoff_message"] = json!(message);
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h484ea3403f07 owner=fractal_missions::store::adhoc_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET assignee=?2, available_to='[]' WHERE subject=?1",
        params![current.subject, input.to],
    )
    .map_err(internal)?;
    let content = format!(
        "{} handed off `{}`: {}\n\n{}\n\nRead this note, then acknowledge it with `st work acknowledge {} --message {} --as {}`. {}",
        input.actor,
        current.subject,
        current.title.as_deref().unwrap_or(&current.step),
        input.note,
        current.subject,
        message,
        input.to,
        if input.to.starts_with("person/") {
            format!(
                "Close with evidence using `st work done {} --as {} --summary TEXT --evidence REF`.",
                current.subject, input.to
            )
        } else {
            format!(
                "Then claim it with `st work claim {} --as {}`.",
                current.subject, input.to
            )
        }
    );
    append_claim_tx(tx, origin, &message, "message.sent", Some(&input.actor), &json!({"fields": {
        "from": input.actor, "to": input.to, "content": content, "status": "sent", "title": format!("Work handoff: {}", current.title.as_deref().unwrap_or(&current.step)),
        "in_reply_to": null, "tags": [format!("st3-work-handoff:{}", current.subject), format!("mission-run:{}", current.run)]
    }, "evidence": input.evidence}), &input.evidence, None).map_err(claim_append_error)?;
    Ok(())
}

pub fn is_adhoc(connection: &Connection, run: &str) -> rusqlite::Result<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='mission-run.created' AND json_extract(body,'$.fields.ad_hoc_title') IS NOT NULL)", [run], |row| row.get(0))
}

pub fn validate_retry(
    connection: &Connection,
    subject: &str,
    input: &WorkHandoffRequest,
) -> Result<(), St3Error> {
    let stored: Option<String> = connection.query_row("SELECT body FROM claims WHERE subject=?1 AND kind='work.released' AND json_extract(body,'$.fields.handoff_key')=?2 AND actor=?3 LIMIT 1",
        params![normalize_step_run(subject), input.idempotency_key, input.actor], |row| row.get(0)).optional().map_err(internal)?;
    let expected = serde_json::to_value(input).map_err(internal)?;
    if stored
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .is_none_or(|body| body["fields"]["handoff_request"] != expected)
    {
        return Err(St3Error::new(
            "idempotency-conflict",
            "this handoff key already names another transfer",
        ));
    }
    Ok(())
}

pub trait AdhocWorkStore {
    fn handoff_retry(
            &self,
            subject: &str,
            input: &WorkHandoffRequest,
        ) -> Result<Option<StepRunView>, St3Error>;
    fn start_work(&self, input: &WorkStartRequest) -> Result<StepRunView, St3Error>;
    fn acknowledge_work(
            &self,
            subject: &str,
            input: &WorkAcknowledgeRequest,
        ) -> Result<StepRunView, St3Error>;
}

impl AdhocWorkStore for Store {

    fn handoff_retry(
            &self,
            subject: &str,
            input: &WorkHandoffRequest,
        ) -> Result<Option<StepRunView>, St3Error> {
        let response = self
            .cached_idempotency_response(&input.idempotency_key)
            .map_err(internal)?;
        if response.is_some() {
            validate_retry(&self.readers.get(), subject, input)?;
        }
        Ok(response)
    }

    fn start_work(&self, input: &WorkStartRequest) -> Result<StepRunView, St3Error> {
        if !input.actor.starts_with("agent/")
            || input.title.trim().is_empty()
            || input.idempotency_key.trim().is_empty()
        {
            return Err(St3Error::new(
                "invalid-work-start",
                "start needs an exact agent actor, title and idempotency key",
            ));
        }
        let hash = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&input.actor, &input.idempotency_key)).map_err(internal)?,
        ));
        let run = format!("mission-run/work/{}", &hash[..32]);
        let generation = format!("work-{}", &hash[..32]);
        let subject = format!("step-run/{generation}/work");
        let mission_id = format!("spontaneous-work/{}", &hash[..32]);
        let title = input.title.replace("${", "$${");
        let kdl = format!(
            "version 2\nmission {mission_id:?} state=\"ready\" {{ goal {title:?}; step \"work\" {{ title {title:?}; assigned-to {:?}; goal {title:?}; }} }}",
            input.actor
        );
        let mission = crate::graph::parse_internal_intent(&kdl, &self.origin)?
            .missions
            .remove(&mission_id)
            .ok_or_else(|| St3Error::new("internal", "minimal work mission missing"))?;
        self.connection.batched(|tx| {
            if let Some(existing) = tx.query_row(/* sqlite-large-value: allow-for-now lv-h1a7eefa5184a owner=fractal_missions::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='mission-run.created' ORDER BY CANONICAL_ASC(claims) LIMIT 1"), [&run], claim_from_row).optional().map_err(internal)? {
                if existing.body["fields"]["ad_hoc_title"] != input.title || existing.actor.as_deref() != Some(&input.actor) {
                    return Err(St3Error::new("idempotency-conflict", "this start key already names another task"));
                }
                return person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the work is no longer retained"));
            }
            if !person_work::declaration_live(tx, &input.actor).map_err(internal)? {
                return Err(St3Error::new("missing-work-actor", "the calling seat needs a live declaration"));
            }
            let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM step_runs WHERE lease_owner=?1 AND status IN ('claimed','working','verifying','blocked') AND CAST(lease_expires_at_unix_ms AS INTEGER)>?2)", params![input.actor, now_ms() as i64], |row| row.get(0)).map_err(internal)?;
            if active { return Err(St3Error::new("agent-capacity", "finish or release independent claimed work before starting another run")); }
            let claim = append_claim_tx(tx, &self.origin, &run, "mission-run.created", Some(&input.actor), &json!({"fields": {
                "status": "running", "mission": mission.subject, "revision": mission.revision, "initial_revision": mission.revision,
                "current_generation": format!("run-generation/{generation}"), "root_revision": mission.revision, "root_mission_run": run,
                "workspace": ".", "requester": input.actor, "inputs": {}, "mode": "run", "ad_hoc_title": input.title, "mission_spec": mission
            }}), &[], None).map_err(claim_append_error)?;
            project_start(tx, &claim)?;
            person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the work could not be projected"))
        }).map_err(internal)?
    }

    fn acknowledge_work(
            &self,
            subject: &str,
            input: &WorkAcknowledgeRequest,
        ) -> Result<StepRunView, St3Error> {
        let subject = normalize_step_run(subject);
        self.connection.batched(|tx| {
            let mut view = person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the handed-off step is missing"))?;
            let handoff = latest_handoff(tx, &view, now_ms())?.ok_or_else(|| St3Error::new("missing-handoff", "this attempt has no handoff note"))?;
            if handoff.body["fields"]["handoff_message"] != input.message || handoff.body["fields"]["handoff_to"] != input.actor || view.assigned_to.as_deref() != Some(&input.actor) {
                return Err(St3Error::new("stale-handoff", "only the current recipient can acknowledge the exact current note"));
            }
            if acknowledged(tx, &handoff, now_ms()).map_err(internal)? {
                enrich_step_queue(tx, &mut view).map_err(internal)?;
                return Ok(view);
            }
            if view.status != "ready" || step_owner_is_terminal_tx(tx, &subject)? {
                return Err(St3Error::new("stale-handoff", "this handoff is no longer waiting for acknowledgment"));
            }
            let seen: Option<String> = tx.query_row(/* sqlite-large-value: allow-for-now lv-h21741300df1e owner=fractal_missions::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id FROM claims WHERE subject=?1 AND actor=?2 AND kind IN ('message.read','message.closed') ORDER BY CANONICAL_ASC(claims) LIMIT 1"), params![input.message, input.actor], |row| row.get(0)).optional().map_err(internal)?;
            let receipt = match seen {
                Some(receipt) => receipt,
                None => append_claim_tx(tx, &self.origin, &input.message, "message.read", Some(&input.actor), &json!({"fields": {"status": "read"}}), &[], None).map_err(claim_append_error)?.id,
            };
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.progress", Some(&input.actor), &json!({"fields": {
                "attempt": view.attempt, "status": "ready", "summary": "Handoff acknowledged", "worker_reported": view.worker_reported,
                "readiness_epoch": view.readiness_epoch, "handoff_acknowledged": input.message
            }, "evidence": [receipt]}), &[receipt, handoff.id], None).map_err(claim_append_error)?;
            project_mission_run_update(tx, &claim)?;
            enrich_step_queue(tx, &mut view).map_err(internal)?;
            Ok(view)
        }).map_err(internal)?
    }
}

