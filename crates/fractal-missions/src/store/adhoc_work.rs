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
) -> Result<(), FractalError> {
    if input.note.trim().is_empty() {
        return Err(FractalError::new(
            "handoff-needs-note",
            "a handoff needs a nonempty note",
        ));
    }
    if input.to == input.actor
        || !(input.to.starts_with("agent/") || input.to.starts_with("person/"))
        || input.to.split('/').skip(1).any(str::is_empty)
    {
        return Err(FractalError::new(
            "invalid-handoff-recipient",
            "name another exact agent/ or person/ recipient",
        ));
    }
    if input.to.starts_with("agent/")
        && !person_work::declaration_live(tx, &input.to).map_err(internal)?
    {
        if super::stop_intake::stop_cut(tx, &input.to).map_err(internal)?.is_some() {
            return Err(super::stop_intake::stopping(&input.to));
        }
        return Err(FractalError::new(
            "missing-handoff-recipient",
            "the recipient seat needs a live declaration",
        ));
    }
    if !matches!(current.status.as_str(), "claimed" | "working") {
        return Err(FractalError::new(
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
        return Err(FractalError::new(
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
) -> Result<(), FractalError> {
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

pub(super) fn clear_eta_tx(
    tx: &Transaction<'_>,
    origin: &str,
    current: &StepRunView,
    input: &WorkHandoffRequest,
) -> Result<(), FractalError> {
    let Some(summary) = current
        .progress_summary
        .as_deref()
        .and_then(fractal_store::work_eta::without_eta)
    else {
        return Ok(());
    };
    // Keep this summary-only progress after the release in the same transaction:
    // replay clears only the advisory note, without changing ownership or the clock.
    append_claim_tx(
        tx,
        origin,
        &current.subject,
        "work.progress",
        Some(&input.actor),
        &json!({"fields": {"attempt": current.attempt, "summary": summary}}),
        &[],
        None,
    )
    .map_err(claim_append_error)?;
    Ok(())
}

pub fn is_adhoc(connection: &Connection, run: &str) -> rusqlite::Result<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='mission-run.created' AND json_extract(body,'$.fields.ad_hoc_title') IS NOT NULL)", [run], |row| row.get(0))
}

pub fn validate_retry(
    connection: &Connection,
    subject: &str,
    input: &WorkHandoffRequest,
) -> Result<(), FractalError> {
    let stored: Option<String> = connection.query_row("SELECT body FROM claims WHERE subject=?1 AND kind='work.released' AND json_extract(body,'$.fields.handoff_key')=?2 AND actor=?3 LIMIT 1",
        params![normalize_step_run(subject), input.idempotency_key, input.actor], |row| row.get(0)).optional().map_err(internal)?;
    let expected = serde_json::to_value(input).map_err(internal)?;
    if stored
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .is_none_or(|body| body["fields"]["handoff_request"] != expected)
    {
        return Err(FractalError::new(
            "idempotency-conflict",
            "this handoff key already names another transfer",
        ));
    }
    Ok(())
}

/// Whether the step's current handoff note is this message, to this recipient.
fn names_current_handoff(
    connection: &Connection,
    subject: &str,
    input: &WorkAcknowledgeRequest,
) -> Result<bool, FractalError> {
    let Some(view) = person_work::step(connection, subject).map_err(internal)? else {
        return Ok(false);
    };
    let Some(handoff) = latest_handoff(connection, &view, now_ms())? else {
        return Ok(false);
    };
    Ok(handoff.body["fields"]["handoff_message"] == input.message
        && handoff.body["fields"]["handoff_to"] == input.actor
        && view.assigned_to.as_deref() == Some(&input.actor))
}

/// The recipient's receipt for a handoff note that is a stream item: the read or close it
/// already recorded, else the read [`stream_note_read`] records. It names the step, as a stream
/// receipt does, and writes nothing.
fn stream_note_receipt(store: &Store, note: &str, actor: &str) -> Result<String, FractalError> {
    use crate::store::messages_shard::{lifecycle_receipt, mailbox_reads};
    let read = BTreeMap::from([("status".to_owned(), json!("read"))]);
    for kind in ["message.read", "message.closed"] {
        if mailbox_reads::lifecycle_step(store, note, kind)
            .map_err(internal)?
            .is_some_and(|(recorded, _)| recorded == actor)
        {
            return Ok(lifecycle_receipt(&store.origin, note, kind, Some(actor), &read).id);
        }
    }
    Ok(lifecycle_receipt(&store.origin, note, "message.read", Some(actor), &read).id)
}

/// Record the recipient's read of a stream handoff note, unless it already read or closed it.
fn stream_note_read(store: &Store, note: &str, actor: &str) -> Result<(), FractalError> {
    use crate::store::messages_shard::{mailbox_reads, record_admitted_lifecycle};
    for kind in ["message.read", "message.closed"] {
        if mailbox_reads::lifecycle_step(store, note, kind)
            .map_err(internal)?
            .is_some_and(|(recorded, _)| recorded == actor)
        {
            return Ok(());
        }
    }
    // The graph's acknowledgement reads the note wherever it stands; so does the stream's.
    let read = BTreeMap::from([("status".to_owned(), json!("read"))]);
    record_admitted_lifecycle(store, note, "message.read", Some(actor), &read).map(drop)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffDisposition {
    Withdrawn,
    Declined,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HandoffReturnRequest {
    pub actor: String,
    pub message: String,
    pub attempt: u32,
    pub disposition: HandoffDisposition,
    pub reason: String,
    pub idempotency_key: String,
}

pub fn native_turn_subject(key: &str) -> String {
    format!("mission-run/native-turn/{}", hex::encode(Sha256::digest(key.as_bytes())))
}

fn native_parent_live(tx: &Transaction<'_>, owner: &StepRunView, attempt: u64) -> Result<bool, FractalError> {
    if u64::from(owner.attempt) != attempt
        || !matches!(owner.status.as_str(), "ready" | "working" | "blocked" | "waiting-person")
        || !super::missions::step_generation_is_current(tx, owner).map_err(internal)?
        || step_owner_is_terminal_tx(tx, &owner.subject)?
    {
        return Ok(false);
    }
    tx.query_row("SELECT status IN ('running','standing','blocked') AND phase='normal' FROM mission_runs WHERE id=?1",
        [owner.run.trim_start_matches("mission-run/")], |row| row.get(0)).map_err(internal)
}

/// Every worker mutation is fenced by the native episode's parent, not just its final report.
pub(super) fn admit_native_turn_work(tx: &Transaction<'_>, step: &StepRunView) -> Result<(), FractalError> {
    if !step.run.starts_with("mission-run/native-turn/") { return Ok(()); }
    let created = latest_claim_of_kind_tx(tx, &step.run, "mission-run.created").map_err(internal)?
        .ok_or_else(|| FractalError::new("missing-native-turn", "native turn creation is missing"))?;
    let fields = &created.body["fields"];
    let owner = fields["parent_step_run"].as_str()
        .map(|parent| person_work::step(tx, parent)).transpose().map_err(internal)?.flatten();
    let valid = match (owner, fields["parent_attempt"].as_u64()) {
        (Some(owner), Some(attempt)) => created.actor.as_deref() == Some(owner.run.as_str())
            && native_parent_live(tx, &owner, attempt)?,
        _ => false,
    };
    if !valid {
        return Err(FractalError::new("stale-native-turn-parent", "native turn parent left its exact live attempt"));
    }
    Ok(())
}

pub trait AdhocWorkStore {
    fn handoff_retry(
            &self,
            subject: &str,
            input: &WorkHandoffRequest,
        ) -> Result<Option<StepRunView>, FractalError>;
    fn start_work(&self, input: &WorkStartRequest) -> Result<StepRunView, FractalError>;
    fn create_native_turn(
        &self, parent: &MissionRunView, step: &str, attempt: u32, key: &str,
        turn: &crate::model::NativeTurn,
    ) -> Result<MissionRunView, FractalError>;
    fn complete_native_turn(&self, run: &str, executions: &[fractal_kernel::StepExecution]) -> Result<bool>;
    fn return_handoff(&self, subject: &str, input: &HandoffReturnRequest) -> Result<ClaimRecord, FractalError>;
    fn acknowledge_work(
            &self,
            subject: &str,
            input: &WorkAcknowledgeRequest,
        ) -> Result<StepRunView, FractalError>;
}

impl AdhocWorkStore for Store {
    fn create_native_turn(
        &self, parent: &MissionRunView, step: &str, attempt: u32, key: &str,
        turn: &crate::model::NativeTurn,
    ) -> Result<MissionRunView, FractalError> {
        turn.validate()?;
        let run = native_turn_subject(key);
        let payload = serde_json::to_value(turn).map_err(internal)?;
        self.with_run_fence(&parent.subject, |tx| {
            let owner = person_work::step(tx, step).map_err(internal)?
                .ok_or_else(|| FractalError::new("missing-step-run", "native turn parent step is missing"))?;
            if owner.run != parent.subject || !native_parent_live(tx, &owner, u64::from(attempt))?
            {
                return Err(FractalError::new("stale-program-attempt", "native turn parent left its exact live attempt"));
            }
            let existing = tx.query_row(
                /* sqlite-large-value: allow-for-now lv-h343eb56d49a5 owner=fractal_missions::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                    FROM claims WHERE subject=?1 AND kind='mission-run.created' ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
                [&run], claim_from_row,
            ).optional().map_err(internal)?;
            if let Some(existing) = existing {
                let fields = &existing.body["fields"];
                if fields["native_turn"] != payload || fields["parent_step_run"] != step
                    || fields["parent_attempt"].as_u64() != Some(u64::from(attempt))
                    || existing.actor.as_deref() != Some(parent.subject.as_str())
                {
                    return Err(FractalError::new("idempotency-conflict", "native turn durable key already owns another request"));
                }
                return Ok(());
            }
            if !person_work::declaration_live(tx, &turn.agent).map_err(internal)? {
                return Err(FractalError::new("missing-work-actor", "native turn target needs a live literal seat declaration"));
            }
            if super::stop_intake::stop_cut(tx, &turn.agent).map_err(internal)?.is_some() {
                return Err(super::stop_intake::stopping(&turn.agent));
            }
            let hash = hex::encode(Sha256::digest(key.as_bytes()));
            let body = json!({"fields": {
                    "native_turn": payload, "native_turn_key": key, "parent_attempt": attempt,
                    "status": "running", "mission": "mission/native-turn", "revision": hash,
                    "current_generation": format!("run-generation/native-turn-{hash}"),
                    "root_revision": parent.root_revision, "root_mission_run": parent.root_mission_run,
                    "parent_step_run": step, "workspace": parent.workspace, "requester": parent.requester,
                    "inputs": {}, "mode": parent.mode
                }});
            // Admission resolves subtree ownership from run rows. Establish that edge
            // before appending, without making a ready leaf visible outside the transaction.
            fractal_store::store::adhoc_work::prepare_native_turn(tx, &run, &body["fields"], now_ms())?;
            let claim = append_claim_tx(tx, &self.origin, &run, "mission-run.created", Some(&parent.subject),
                &body, &[], None).map_err(claim_append_error)?;
            fractal_store::store::adhoc_work::project_native_turn(tx, &claim)
        })?;
        self.mission_run(&run).map_err(internal)?
            .ok_or_else(|| FractalError::new("missing-mission-run", "native turn creation did not project"))
    }

    fn complete_native_turn(&self, run: &str, executions: &[fractal_kernel::StepExecution]) -> Result<bool> {
        self.with_all_runs_fenced_internal(|tx| {
            let created = latest_claim_of_kind_tx(tx, run, "mission-run.created")?
                .ok_or_else(|| anyhow::anyhow!("native turn creation is missing"))?;
            let fields = &created.body["fields"];
            let turn: crate::model::NativeTurn = serde_json::from_value(fields["native_turn"].clone())?;
            let parent = fields["parent_step_run"].as_str().ok_or_else(|| anyhow::anyhow!("native turn parent is missing"))?;
            let Some(owner) = person_work::step(tx, parent)? else { return Ok(false); };
            if !native_parent_live(tx, &owner, fields["parent_attempt"].as_u64().unwrap_or(0))?
            {
                return Ok(false);
            }
            let generation = fields["current_generation"].as_str().ok_or_else(|| anyhow::anyhow!("native turn generation is missing"))?
                .trim_start_matches("run-generation/");
            let subject = format!("step-run/{generation}/turn");
            let Some(step) = person_work::step(tx, &subject)? else { return Ok(false); };
            if step.status != "verifying" || !step.worker_reported || step_owner_is_terminal_tx(tx, &subject)? {
                return Ok(false);
            }
            let report = latest_claim_of_kind_tx(tx, &subject, "work.submitted")?
                .filter(|report| report.actor.as_deref() == Some(turn.agent.as_str())
                    && report.body["fields"]["attempt"].as_u64() == Some(u64::from(step.attempt)))
                .ok_or_else(|| anyhow::anyhow!("native turn has no current literal worker report"))?;
            let mut products = BTreeMap::new();
            for product in &turn.products {
                let resource = format!("resource/{run}/{}", product.path);
                let Some(value) = fractal_store::store::latest_actual(tx, &resource)? else { return Ok(false); };
                let actual = value.get("fields").unwrap_or(&value);
                let field = |name: &str| name.split('.').try_fold(actual, |value, part| value.get(part));
                if field("kind").and_then(Value::as_str) != Some(product.kind.as_str())
                    || !product.fields.iter().all(|(name, expected)| field(name) == Some(expected))
                {
                    return Ok(false);
                }
                products.insert(product.path.clone(), value);
            }
            // Verification is a daemon receipt, not another outward effect by the parent.
            let claim = append_claim_tx(tx, &self.origin, run, "program.turn.report", None,
                &json!({"fields": {"report": {
                    "run": run, "step": subject, "summary": report.body["fields"]["summary"].as_str().unwrap_or_default(),
                    "evidence": report.body.get("evidence").cloned().unwrap_or_else(|| json!([])),
                    "products": products, "executions": executions
                }}}), &[report.id], None)?;
            let terminal = append_claim_tx(tx, &self.origin, &subject, "step-run.state", None,
                &json!({"fields": {"status": "completed", "attempt": step.attempt}}), &[claim.id], None)?;
            project_mission_run_update(tx, &terminal)?;
            super::missions::set_mission_run_state_tx(tx, &self.origin, run, "completed", "terminal", None)
        })
    }


    fn handoff_retry(
            &self,
            subject: &str,
            input: &WorkHandoffRequest,
        ) -> Result<Option<StepRunView>, FractalError> {
        let response = self
            .cached_idempotency_response(&input.idempotency_key)
            .map_err(internal)?;
        if response.is_some() {
            self.with_run_fence(subject, |tx| validate_retry(tx, subject, input))?;
        }
        Ok(response)
    }

    fn return_handoff(&self, subject: &str, input: &HandoffReturnRequest) -> Result<ClaimRecord, FractalError> {
        let subject = normalize_step_run(subject);
        if input.reason.trim().is_empty() || input.idempotency_key.trim().is_empty() {
            return Err(FractalError::new("invalid-handoff-return", "a reason and idempotency key are required"));
        }
        self.with_run_fence(&subject, |tx| {
            let existing = tx.query_row(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE kind='work.handoff-returned' AND json_extract(body,'$.fields.return_request.idempotency_key')=?1",
                [&input.idempotency_key], claim_from_row).optional().map_err(internal)?;
            if let Some(existing) = existing {
                if existing.subject != subject || existing.body["fields"]["return_request"] != json!(input) {
                    return Err(FractalError::new("idempotency-conflict", "this key names another return"));
                }
                return Ok(existing);
            }
            let view = person_work::step(tx, &subject).map_err(internal)?
                .ok_or_else(|| FractalError::new("missing-step-run", "the handed-off step is missing"))?;
            let handoff = latest_handoff(tx, &view, now_ms())?
                .ok_or_else(|| FractalError::new("stale-handoff", "this attempt has no unresolved handoff"))?;
            let from = handoff.actor.as_deref().unwrap_or_default();
            let to = handoff.body["fields"]["handoff_to"].as_str().unwrap_or_default();
            let authorized = match input.disposition {
                HandoffDisposition::Withdrawn => input.actor == from,
                HandoffDisposition::Declined => input.actor == to,
            };
            if step_owner_is_terminal_tx(tx, &subject)? || view.status != "ready"
                || view.claimant.is_some() || view.attempt != input.attempt
                || view.claim_incarnation.is_some() || view.claim_expires_at_unix_ms.is_some()
                || handoff.body["fields"]["handoff_message"] != input.message
                || !from.starts_with("agent/") || !to.starts_with("person/")
                || view.assigned_to.as_deref() != Some(to) || !authorized {
                return Err(FractalError::new("stale-handoff", "only the handing agent or assigned person can return the exact unclaimed live handoff"));
            }
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.handoff-returned", Some(&input.actor),
                &json!({"fields": {"attempt": view.attempt, "status": "ready", "handoff_claim": handoff.id,
                    "handoff_message": input.message, "handoff_to": from, "disposition": input.disposition,
                    "reason": input.reason, "readiness_epoch": view.readiness_epoch + 1, "return_request": input}}),
                &[handoff.id], None).map_err(claim_append_error)?;
            project_mission_run_update(tx, &claim)?;
            Ok(claim)
        })
    }

    fn start_work(&self, input: &WorkStartRequest) -> Result<StepRunView, FractalError> {
        if !input.actor.starts_with("agent/")
            || input.title.trim().is_empty()
            || input.idempotency_key.trim().is_empty()
        {
            return Err(FractalError::new(
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
            .ok_or_else(|| FractalError::new("internal", "minimal work mission missing"))?;
        self.with_run_fence(&run, |tx| {
            if let Some(existing) = tx.query_row(/* sqlite-large-value: allow-for-now lv-h1a7eefa5184a owner=fractal_missions::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='mission-run.created' ORDER BY CANONICAL_ASC(claims) LIMIT 1"), [&run], claim_from_row).optional().map_err(internal)? {
                if existing.body["fields"]["ad_hoc_title"] != input.title || existing.actor.as_deref() != Some(&input.actor) {
                    return Err(FractalError::new("idempotency-conflict", "this start key already names another task"));
                }
                return person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| FractalError::new("missing-step-run", "the work is no longer retained"));
            }
            if !person_work::declaration_live(tx, &input.actor).map_err(internal)? {
                if super::stop_intake::stop_cut(tx, &input.actor).map_err(internal)?.is_some() {
                    return Err(super::stop_intake::stopping(&input.actor));
                }
                return Err(FractalError::new("missing-work-actor", "the calling seat needs a live declaration"));
            }
            let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM step_runs WHERE lease_owner=?1 AND status IN ('claimed','working','verifying','blocked') AND CAST(lease_expires_at_unix_ms AS INTEGER)>?2)", params![input.actor, now_ms() as i64], |row| row.get(0)).map_err(internal)?;
            if active { return Err(FractalError::new("agent-capacity", "finish or release independent claimed work before starting another run")); }
            let claim = append_claim_tx(tx, &self.origin, &run, "mission-run.created", Some(&input.actor), &json!({"fields": {
                "status": "running", "mission": mission.subject, "revision": mission.revision, "initial_revision": mission.revision,
                "current_generation": format!("run-generation/{generation}"), "root_revision": mission.revision, "root_mission_run": run,
                "workspace": ".", "requester": input.actor, "inputs": {}, "mode": "run", "ad_hoc_title": input.title, "mission_spec": mission
            }}), &[], None).map_err(claim_append_error)?;
            project_start(tx, &claim)?;
            person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| FractalError::new("missing-step-run", "the work could not be projected"))
        })
    }

    fn acknowledge_work(
            &self,
            subject: &str,
            input: &WorkAcknowledgeRequest,
        ) -> Result<StepRunView, FractalError> {
        let subject = normalize_step_run(subject);
        // A note in the messages shard is a stream item: its read is this member's record there.
        // The progress below cites the read's receipt, whose id is known before it is recorded.
        // The read itself is recorded only once the acknowledgment has committed, so a refused
        // one leaves the note unread, and outside the graph's writer: the shard's admission can
        // seal graph batches under the shard's writer, so the graph's writer must never wait on
        // the shard's. The note is cited only when it is this step's current handoff to the
        // reader; the transaction below checks that again under the writer.
        let stream_receipt = match self.message(&input.message).map_err(internal)? {
            Some(note) if crate::store::messages_shard::mailbox_reads::is_stream_item(&note) => {
                let current = self.with_run_fence(&subject, |tx| {
                    names_current_handoff(tx, &subject, input)
                })?;
                Some(current.then(|| stream_note_receipt(self, &input.message, &input.actor)).transpose()?)
            }
            _ => None,
        };
        let view = self.with_run_fence(&subject, |tx| {
            let mut view = person_work::step(tx, &subject).map_err(internal)?.ok_or_else(|| FractalError::new("missing-step-run", "the handed-off step is missing"))?;
            let handoff = latest_handoff(tx, &view, now_ms())?.ok_or_else(|| FractalError::new("missing-handoff", "this attempt has no handoff note"))?;
            if handoff.body["fields"]["handoff_message"] != input.message || handoff.body["fields"]["handoff_to"] != input.actor || view.assigned_to.as_deref() != Some(&input.actor) {
                return Err(FractalError::new("stale-handoff", "only the current recipient can acknowledge the exact current note"));
            }
            if acknowledged(tx, &handoff, now_ms()).map_err(internal)? {
                enrich_step_queue(tx, &mut view).map_err(internal)?;
                return Ok(view);
            }
            if view.status != "ready" || step_owner_is_terminal_tx(tx, &subject)? {
                return Err(FractalError::new("stale-handoff", "this handoff is no longer waiting for acknowledgment"));
            }
            let (receipt, predecessors) = if let Some(receipt) = &stream_receipt {
                // The receipt is not a graph claim, so the progress cites it without following it.
                let receipt = receipt.clone().ok_or_else(|| FractalError::new("stale-handoff", "the handoff changed while it was acknowledged; acknowledge it again"))?;
                (receipt, vec![handoff.id])
            } else {
                let seen: Option<String> = tx.query_row(/* sqlite-large-value: allow-for-now lv-h21741300df1e owner=fractal_missions::store::adhoc_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id FROM claims WHERE subject=?1 AND actor=?2 AND kind IN ('message.read','message.closed') ORDER BY CANONICAL_ASC(claims) LIMIT 1"), params![input.message, input.actor], |row| row.get(0)).optional().map_err(internal)?;
                // Without a graph read, the reader's local mark (message_marks.rs) is its read: the
                // progress cites its receipt, which is not a graph claim and so no predecessor.
                let marked: Option<String> = if seen.is_some() { None } else { tx.query_row(
                    "SELECT receipt FROM local_message_marks WHERE message=?1 AND consumer=?2
                     AND kind IN ('message.read','message.closed') ORDER BY recorded_at_unix_ms LIMIT 1",
                    params![input.message, input.actor], |row| row.get::<_, String>(0),
                ).optional().map_err(internal)? };
                match (seen, marked) {
                    (Some(receipt), _) => (receipt.clone(), vec![receipt, handoff.id]),
                    (None, Some(marked)) => (marked, vec![handoff.id]),
                    // A reader this member hosts records its read as a mark once marks are on.
                    (None, None) if crate::store::message_marks::local_consumer(tx, &self.origin, &input.message, &input.actor).map_err(internal)? => {
                        let (receipt, _) = crate::store::message_marks::record_tx(tx, &self.origin, &input.message, "message.read", &input.actor)?;
                        (receipt.id, vec![handoff.id])
                    }
                    (None, None) => {
                        let receipt = append_claim_tx(tx, &self.origin, &input.message, "message.read", Some(&input.actor), &json!({"fields": {"status": "read"}}), &[], None).map_err(claim_append_error)?.id;
                        (receipt.clone(), vec![receipt, handoff.id])
                    }
                }
            };
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.progress", Some(&input.actor), &json!({"fields": {
                "attempt": view.attempt, "status": "ready", "summary": "Handoff acknowledged", "worker_reported": view.worker_reported,
                "readiness_epoch": view.readiness_epoch, "handoff_acknowledged": input.message
            }, "evidence": [receipt]}), &predecessors, None).map_err(claim_append_error)?;
            project_mission_run_update(tx, &claim)?;
            enrich_step_queue(tx, &mut view).map_err(internal)?;
            Ok(view)
        })?;
        // Acknowledged, now or before: the note is read. A read that fails here is recorded by
        // the repeated acknowledgment its error asks for.
        if let Some(Some(_)) = &stream_receipt {
            stream_note_read(self, &input.message, &input.actor)?;
        }
        Ok(view)
    }
}

