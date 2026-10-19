//! The part of `store::person_work` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::{MessageAttachment, PersonAskRequest, PersonStepResponse};
use crate::person_request::{StructuredRequest, answer_summary};
pub use st3_delivery::store::person_work::*;
pub use st3_delivery::store::person_work::{current, declaration_live, enrich_responses, is_update, project, project_minimal_run, request, run_live, run_liveness, step, validate_delegated_answer};
#[cfg(test)]
mod revision_provenance_tests;

// Only ready/pending raw steps can be cancelled by reconcile_person_asks. Start from the
// existing open-step index so completed ask history costs no candidate reads. The explicit
// NOT IN repeats that index's predicate; CROSS JOIN keeps the sparse steps as the outer loop.
const HAS_PERSON_ASKS_FOR_RECONCILE: &str =
    "SELECT EXISTS(SELECT 1 FROM step_runs INDEXED BY step_runs_open_index CROSS JOIN claims
     WHERE step_runs.status NOT IN ('completed','failed','cancelled')
       AND step_runs.status IN ('ready','pending')
       AND claims.subject=step_runs.subject AND claims.kind='work.person-asked')";

pub const PERSON_ASKS_FOR_RECONCILE: &str =
    "SELECT claims.id, claims.store_index, claims.batch_id, claims.subject, claims.kind,
            claims.origin, claims.actor, claims.body, claims.predecessors, claims.accepted_at_unix_ms
     FROM step_runs INDEXED BY step_runs_open_index CROSS JOIN claims
     WHERE step_runs.status NOT IN ('completed','failed','cancelled')
       AND step_runs.status IN ('ready','pending')
       AND claims.subject=step_runs.subject AND claims.kind='work.person-asked'
     ORDER BY CANONICAL_ASC(claims)";

/// The images an ask carries and the two parties to it (`Store::ask_images`).
#[derive(Clone, Debug)]
pub struct AskImages {
    pub asker: String,
    pub person: Option<String>,
    pub attachments: Vec<MessageAttachment>,
}

pub fn person_asks_for_reconcile(connection: &Connection) -> Result<Vec<ClaimRecord>> {
    connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h23a782d96b14 owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql(PERSON_ASKS_FOR_RECONCILE))?
        .query_map([], claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// The ask's structured request in canonical form, or null for a free-text ask.
fn canonical_request(input: &PersonAskRequest) -> Result<Value, St3Error> {
    input
        .request
        .as_ref()
        .map(|request| StructuredRequest::parse(request).map(|(_, canonical)| canonical))
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Adds the structured request to an ask claim body. A free-text ask's body stays exactly as
/// before, so members that do not know the field still admit it.
fn with_request(mut body: Value, request: &Value) -> Value {
    if !request.is_null() {
        body["fields"]["request"] = request.clone();
    }
    body
}

/// The answer the person gives and the summary history keeps. A free-text ask takes text
/// only; a structured ask resolves the named answer against its request.
fn resolve_answer(
    ask: Option<&ClaimRecord>,
    input: &PersonStepResponse,
    cancel: bool,
) -> Result<(Option<Value>, String), St3Error> {
    let request = if cancel {
        None
    } else {
        ask.map(structured_request).transpose()?.flatten()
    };
    if cancel || request.is_none() {
        let Some(answer) = &input.answer else {
            return Ok((None, input.summary.clone()));
        };
        if cancel || answer.id.is_some() {
            return Err(St3Error::new(
                "invalid-person-answer",
                if cancel {
                    "a cancelled ask has no answer"
                } else {
                    "this ask is free text; it has no named answers"
                },
            ));
        }
        let summary = if input.summary.trim().is_empty() {
            answer.text.clone().unwrap_or_default()
        } else {
            input.summary.clone()
        };
        return Ok((None, summary));
    }
    let answer = request
        .expect("checked above")
        .answer(input.answer.as_ref(), &input.summary)?;
    let summary = if input.summary.trim().is_empty() {
        answer_summary(&answer)
    } else {
        input.summary.clone()
    };
    Ok((Some(answer), summary))
}

/// Whether `person` asked for `about`: a mission run or step run that the person requested, or
/// whose root run they requested, or a message the person sent to `agent`.
fn person_asked(connection: &Connection, person: &str, agent: &str, about: &str) -> Result<bool> {
    if about.starts_with("message/") {
        let mut query = connection
            .prepare_cached("SELECT body FROM claims WHERE subject=?1 AND kind='message.sent'")?;
        for body in query.query_map([about], |row| row.get::<_, String>(0))? {
            let body: Value = serde_json::from_str(&body?)?;
            if body["fields"]["from"] == person && body["fields"]["to"] == agent {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let run = if about.starts_with("step-run/") {
        match step(connection, about)? {
            Some(view) => view.run,
            None => return Ok(false),
        }
    } else {
        about.to_owned()
    };
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM mission_runs run
          LEFT JOIN mission_runs root ON root.id=run.root_run_id
          WHERE run.id=?1 AND (run.requester=?2 OR root.requester=?2))",
        params![run.trim_start_matches("mission-run/"), person],
        |row| row.get(0),
    )?)
}

/// Tell the requester of a new-run ask what the person answered. A step-owned ask needs no
/// message: the response readies the asking step again, and its wake reaches the seat.
fn send_answer_tx(
    tx: &Transaction<'_>,
    origin: &str,
    ask: &ClaimRecord,
    response: &ClaimRecord,
    summary: &str,
    answer: Option<&Value>,
) -> Result<(), St3Error> {
    let Some(requester) = ask.actor.as_deref() else {
        return Ok(());
    };
    let fields = &ask.body["fields"];
    let title = fields["title"].as_str().unwrap_or_default();
    let person = response.body["fields"]["acted_for"]
        .as_str()
        .unwrap_or_else(|| response.actor.as_deref().unwrap_or("the person"));
    let verb = if answer.is_some_and(|answer| answer["outcome"] == "dismissed") {
        "dismissed"
    } else {
        "answered"
    };
    let mut content = format!("{person} {verb} `{}`: {title}\n\n", ask.subject);
    if response.body["fields"].get("acted_for").is_some() {
        content.push_str(&format!(
            "Recorded by {} for {person}.\n\n",
            response.actor.as_deref().unwrap_or("the agent")
        ));
    }
    match answer.filter(|answer| answer["id"].is_string()) {
        Some(answer) => {
            content.push_str(&format!(
                "Answer: {} ({})",
                answer["label"].as_str().unwrap_or_default(),
                answer["id"].as_str().unwrap_or_default()
            ));
            if let Some(text) = answer["text"].as_str() {
                content.push_str(&format!("\n{text}"));
            }
        }
        None => content.push_str(summary),
    }
    content.push_str(&format!(
        "\n\nThe answer as data: `st work show {} --json` (person_answers).",
        ask.subject
    ));
    let key = format!("person-answer:{}:{}", ask.subject, response.id);
    let subject = format!(
        "message/{}",
        &hex::encode(Sha256::digest(key.as_bytes()))[..16]
    );
    let mut tags = vec![Value::String(format!("st3-person-answer:{}", ask.subject))];
    if let Some(run) = fields["owner_run"].as_str() {
        tags.push(Value::String(format!("mission-run:{run}")));
    }
    append_claim_tx(tx, origin, &subject, "message.sent", Some("daemon/runtime"),
        &json!({"fields": {"from": "daemon/runtime", "to": requester, "content": content, "status": "sent",
            "title": format!("Answered: {title}"), "in_reply_to": null, "tags": tags}, "evidence": [response.id]}),
        &[], None).map_err(claim_append_error)?;
    Ok(())
}

pub trait PersonWorkStore {
    /// A daemon-owned approval ask, projected in the same transaction as its durable claim.
    fn ask_daemon_approval(&self, person: &str, key: &str, proposal: Value) -> Result<StepRunView, St3Error>;
    fn ask_person(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error>;
    fn finish_person_step(
            &self,
            input: &PersonStepResponse,
            cancel: bool,
        ) -> Result<StepRunView, St3Error>;
    fn finish_person_step_authorized(
            &self,
            input: &PersonStepResponse,
            cancel: bool,
            authorization: Option<&super::assistant_write::AssistantWriteAuthorization>,
        ) -> Result<StepRunView, St3Error>;
    /// The images an ask's request carries, found by the ask's person step, with who asked and
    /// whom it asks: what a blob read needs to authorize the person looking at them.
    fn ask_images(&self, step_subject: &str) -> Result<Option<AskImages>>;
    fn reconcile_person_asks(&self) -> Result<bool>;
    /// Brings a person information they asked for. Nothing waits on an update: it stays on the
    /// person's home until they open or read it, and it is refused unless `about` names the
    /// person's own run or step, or their message to the poster.
    fn post_update(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error>;
    /// Nothing an ask waited on disappears silently: when its requester, originating step or
    /// owning run ends, the requester hears once that the ask is gone, and the person gets one
    /// update saying so.
    fn tell_ask_ended(&self, tx: &Transaction<'_>, ask: &ClaimRecord) -> Result<()>;
    /// One update step for `person`, about the work `structured` names.
    #[allow(clippy::too_many_arguments)]
    fn append_update_tx(
            &self,
            tx: &Transaction<'_>,
            actor: &str,
            person: &str,
            title: &str,
            reason: &str,
            idempotency_key: &str,
            structured: &Value,
        ) -> Result<StepRunView, St3Error>;
    fn ask_person_in_new_run(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error>;
}

impl PersonWorkStore for Store {

    fn ask_daemon_approval(&self, person: &str, key: &str, proposal: Value) -> Result<StepRunView, St3Error> {
        let subject = format!("step-run/chatgpt-{key}/ask");
        let mission_id = format!("chatgpt/{key}");
        let kdl = format!("version 2\nmission {mission_id:?} state=\"ready\" {{ goal \"Approve ChatGPT proposal\"; step \"ask\" {{ assigned-to {person:?}; goal \"Approve exact proposed action\"; }} }}");
        let mut intent = crate::graph::parse_internal_intent(&kdl, &self.origin)?;
        let mission = intent.missions.remove(&mission_id).ok_or_else(|| St3Error::new("internal","approval mission absent"))?;
        let input = PersonAskRequest { legacy_request:None, person:person.into(), actor:"daemon/chatgpt-approval".into(),
            title:"Approve ChatGPT proposal".into(), reason:serde_json::to_string(&proposal).map_err(internal)?,
            step:None, new_run:Some(mission_id), incarnation:None, idempotency_key:key.into(),
            request:Some(json!({"version":1,"type":"decision","question":"Execute this exact ChatGPT proposal?",
                "why_person":"Only the owner authorizes assistant writes.","summary":"Approve the exact typed action in the alert detail; original fences remain mandatory.",
                "answers":[{"id":"approve","label":"Approve","consequence":"Execute this exact action once, subject to its original fence.","outcome":"accept"},
                    {"id":"deny","label":"Deny","consequence":"Do not execute the action.","outcome":"decline"}]})) };
        let structured = canonical_request(&input)?;
        self.connection.batched(|tx| {
            if let Some(existing) = request(tx,&subject).map_err(internal)? {
                if existing.body["fields"]["person"] != person || existing.body["fields"]["request"] != structured {
                    return Err(St3Error::new("idempotency-conflict","approval key already names another request"));
                }
                return step(tx,&subject).map_err(internal)?.ok_or_else(|| St3Error::new("internal","approval ask absent"));
            }
            let claim = append_claim_tx(tx,&self.origin,&subject,"work.person-asked",Some("daemon/chatgpt-approval"),
                &with_request(json!({"fields":{"run":format!("mission-run/chatgpt/{key}"),
                    "generation":format!("run-generation/chatgpt-{key}"),"person":person,"title":input.title,"reason":input.reason,
                    "key":key,"attempt":1,"status":"ready","mission_spec":mission}}),&structured),&[],None).map_err(claim_append_error)?;
            project(tx,&claim)?;
            step(tx,&subject).map_err(internal)?.ok_or_else(|| St3Error::new("internal","approval ask absent"))
        }).map_err(internal)?
    }

    fn ask_person(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error> {
        if !input.person.starts_with("person/")
            || input.person.matches('/').count() != 1
            || input.person == "person/"
            || !input.actor.starts_with("agent/")
            || input.title.trim().is_empty()
            || input.reason.trim().is_empty()
            || input.idempotency_key.is_empty()
        {
            return Err(St3Error::new(
                "invalid-person-ask",
                "a person ask needs a person, title, reason and idempotency key",
            ));
        }
        if input
            .request
            .as_ref()
            .is_some_and(|request| request["type"] == "update")
        {
            return self.post_update(input);
        }
        if input.step.is_none() || input.new_run.is_some() {
            return self.ask_person_in_new_run(input);
        }
        self.connection.batched(|tx| {
            let structured = canonical_request(input)?;
            let named = step(tx, &normalize_step_run(input.step.as_deref().unwrap())).map_err(internal)?
                .ok_or_else(|| St3Error::new("missing-step-run", "the asking step does not exist"))?;
            // A revision moves a step into the run's new generation, and its worker may still
            // name it by the predecessor subject, as the seat's environment does. Work actions
            // accept that name; so does an ask, and only the claimant may ask from either.
            let origin = current_generation_successor_tx(tx, &named).map_err(internal)?.unwrap_or(named);
            let origin_subject = origin.subject.clone();
            let identity = serde_json::to_string(&(&origin.generation, &origin_subject, origin.attempt, &input.idempotency_key)).map_err(internal)?;
            let hash = hex::encode(Sha256::digest(identity.as_bytes()));
            let subject = format!("step-run/{}/ask-{}", generation_id_from_subject(&origin.generation), &hash[..32]);
            if let Some(existing) = request(tx, &subject).map_err(internal)? {
                if existing.actor.as_deref() != Some(input.actor.as_str())
                    || existing.body["fields"]["person"] != input.person
                    || existing.body["fields"]["title"] != input.title
                    || existing.body["fields"]["reason"] != input.reason
                    || existing.body["fields"]["request"] != structured {
                    return Err(St3Error::new("idempotency-conflict", "this ask key already names a different question"));
                }
                return step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the ask is no longer retained"));
            }
            let legacy_origin = input.legacy_request.as_ref().map(|id| -> Result<bool> {
                let legacy = tx.query_row("SELECT body FROM claims WHERE id=?1 AND kind='attention.requested' AND actor=?2", params![id, input.actor], |row| row.get::<_, String>(0)).optional()?;
                let Some(legacy) = legacy else { return Ok(false) };
                let legacy: Value = serde_json::from_str(&legacy)?;
                let accepted = canonical::claim_key(tx, id)?;
                let mut claimed = tx.prepare(/* sqlite-large-value: allow-for-now lv-h65dad5f0e6c4 owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='work.claimed' ORDER BY CANONICAL_DESC(claims)"))?;
                let mut proven = false;
                for claim in claimed.query_map([&origin_subject], claim_from_row)? {
                    let claim = claim?;
                    if canonical::claim_key(tx, &claim.id)? > accepted { continue; }
                    proven = claim.actor.as_deref() == Some(input.actor.as_str()) && claim.body["fields"]["attempt"].as_u64() == Some(origin.attempt as u64);
                    break;
                }
                Ok(proven && legacy["fields"]["step"] == origin_subject && legacy["fields"]["step_attempt"].as_u64() == Some(origin.attempt as u64)
                    && matches!(origin.status.as_str(), "ready" | "blocked" | "claimed" | "working"))
            }).transpose().map_err(internal)?.unwrap_or(false);
            if step_owner_is_terminal_tx(tx, &origin_subject)?
                || !declaration_live(tx, &input.actor).map_err(internal)?
                || (!legacy_origin && (!matches!(origin.status.as_str(), "claimed" | "working")
                || origin.claimant.as_deref() != Some(input.actor.as_str())
                || origin.claim_incarnation != input.incarnation
                || origin.claim_expires_at_unix_ms.is_none_or(|expiry| expiry <= now_ms()))) {
                return Err(St3Error::new("stale-work-ask", format!(
                    "only the current claimant and incarnation of live work can ask a person; expected incarnation `{}`, given `{}`",
                    origin.claim_incarnation.as_deref().unwrap_or("<none>"), input.incarnation.as_deref().unwrap_or("<none>"))));
            }
            let waiting_since = input.legacy_request.as_ref().map(|id| tx.query_row("SELECT accepted_at_unix_ms FROM claims WHERE id=?1 AND kind='attention.requested'", [id], |row| row.get::<_, String>(0))).transpose().map_err(internal)?;
            let evidence = input.legacy_request.iter().cloned().collect::<Vec<_>>();
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.person-asked", Some(&input.actor),
                &with_request(json!({"fields": {"run": origin.run, "generation": origin.generation,
                    "origin_step": origin_subject, "origin_attempt": origin.attempt,
                    "person": input.person, "title": input.title, "reason": input.reason,
                    "key": input.idempotency_key, "attempt": 1, "status": "ready", "waiting_since": waiting_since, "legacy_request": input.legacy_request}}), &structured), &evidence, None).map_err(claim_append_error)?;
            project(tx, &claim)?;
            let pause = append_claim_tx(tx, &self.origin, &origin_subject, "step-run.state", Some("daemon/runtime"),
                &json!({"fields": {"status": "waiting-person", "reason": subject, "attempt": origin.attempt}}), &[claim.id], None).map_err(claim_append_error)?;
            project_mission_run_update(tx, &pause)?;
            step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the ask could not be projected"))
        }).map_err(internal)?
    }

    fn finish_person_step(
            &self,
            input: &PersonStepResponse,
            cancel: bool,
        ) -> Result<StepRunView, St3Error> {
        self.finish_person_step_authorized(input, cancel, None)
    }

    fn finish_person_step_authorized(
            &self,
            input: &PersonStepResponse,
            cancel: bool,
            authorization: Option<&super::assistant_write::AssistantWriteAuthorization>,
        ) -> Result<StepRunView, St3Error> {
        if let Some(authorization) = authorization
            && (authorization.scope != "read.assistant.attention" || authorization.approval_subject.is_none()
                || authorization.owner != input.actor || cancel)
        {
            return Err(St3Error::new("forbidden", "assistant authorization does not bind this person answer"));
        }
        #[cfg(feature = "test-support")]
        let _gate = match authorization {
            Some(authorization) => super::assistant_write_gate::pause(&authorization.grant_subject)?,
            None => None,
        };
        self.connection.batched(|tx| {
            if let Some(authorization) = authorization {
                super::assistant_write::authorize(tx, &self.origin, authorization)?;
            }
            let subject = normalize_step_run(&input.subject);
            let mut view = step(tx, &subject).map_err(internal)?
                .ok_or_else(|| St3Error::new("missing-step-run", "the person step does not exist"))?;
            let ask = request(tx, &subject).map_err(internal)?;
            // Checked after authority and fences below; a replay compares against it first.
            let resolved = resolve_answer(ask.as_ref(), input, cancel);
            let kind = if cancel { "work.person-cancelled" } else { "work.person-done" };
            if let Some(existing) = tx.query_row(/* sqlite-large-value: allow-for-now lv-hce55d0ab3402 owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id, store_index, batch_id, subject, kind, origin,
                actor, body, predecessors, accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind=?2
                ORDER BY CANONICAL_DESC(claims) LIMIT 1"), params![subject, kind], claim_from_row).optional().map_err(internal)? {
                if existing.body["fields"]["key"] == input.idempotency_key && existing.actor.as_deref() == Some(input.actor.as_str()) {
                    let Ok((answer, summary)) = &resolved else {
                        return Err(St3Error::new("idempotency-conflict", "this response key already has another answer"));
                    };
                    let mut response_evidence = input.evidence.clone();
                    if let Some(proof) = &input.delegation { super::delegation::add_evidence(tx, &mut response_evidence, proof)?; }
                    if existing.body["fields"]["summary"] != *summary || existing.body["evidence"] != json!(response_evidence)
                        || existing.body["fields"].get("delegation") != input.delegation.as_ref().map(|proof| serde_json::to_value(proof).expect("serializable proof")).as_ref()
                        || existing.body["fields"]["answer"] != answer.clone().unwrap_or_default()
                        || existing.body["fields"]["attempt"].as_u64() != Some(view.attempt as u64)
                        || input.episode.as_ref().is_some_and(|episode| existing.body["fields"]["episode"] != *episode) {
                        return Err(St3Error::new("idempotency-conflict", "this response key already has another summary"));
                    }
                    return Ok(view);
                }
            }
            if cancel {
                if ask.as_ref().and_then(|a| a.actor.as_deref()) != Some(input.actor.as_str()) {
                    return Err(St3Error::new("forbidden", "only the requester can cancel its ask"));
                }
            } else if let Some(proof) = &input.delegation {
                if !input.actor.starts_with("agent/") || view.assigned_to.as_deref() != Some(&proof.person) || input.episode.as_deref() != Some(&proof.episode) {
                    return Err(St3Error::new("delegation-refused", "delegation needs the assigned person and exact asking episode"));
                }
            } else if !input.actor.starts_with("person/") || view.assigned_to.as_deref() != Some(input.actor.as_str()) {
                return Err(St3Error::new("forbidden", "only the assigned person can complete this step"));
            }
            apply_effective_step_state(tx, &mut view, now_ms()).map_err(internal)?;
            if view.status != "ready"
                || ask.as_ref().is_some_and(|a| input.episode.as_ref().is_some_and(|e| e != &a.id))
                || ask.as_ref().map(|a| current(tx, a, now_ms())).transpose().map_err(internal)?.is_some_and(|live| !live) {
                return Err(St3Error::new("stale-fence", "this person step is no longer waiting in that episode"));
            }
            if ask.is_none() && input.episode.as_ref().is_some_and(|episode| episode != &format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch)) {
                return Err(St3Error::new("stale-fence", "this authored person step has moved to another episode"));
            }
            if !cancel && super::adhoc_work::pending_handoff(tx, &view)? {
                return Err(St3Error::new("handoff-not-acknowledged", "read and acknowledge the current handoff note before closing this step"));
            }
            // Closing a handed-off step closes the spontaneous work and needs evidence; answering an
            // ask an agent made from that work closes only the ask, which is the person's answer.
            if !cancel && ask.is_none() && super::adhoc_work::is_adhoc(tx, &view.run).map_err(internal)? && input.evidence.is_empty() {
                return Err(St3Error::new("work-needs-evidence", "close spontaneous work with --evidence"));
            }
            let (answer, summary) = resolved?;
            if summary.trim().is_empty() {
                return Err(St3Error::new("invalid-person-response", "a response needs a summary"));
            }
            let evidence = ask.as_ref().map(|a| vec![a.id.clone()]).unwrap_or_default();
            let mut body = json!({"fields": {"attempt": view.attempt, "status": if cancel { "cancelled" } else { "completed" },
                "summary": summary, "key": input.idempotency_key, "episode": ask.as_ref().map(|a| a.id.clone()).unwrap_or_else(|| format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch))}, "evidence": input.evidence});
            if let Some(answer) = &answer {
                body["fields"]["answer"] = answer.clone();
            }
            if let Some(proof) = &input.delegation {
                if cancel { return Err(St3Error::new("delegation-refused", "cancelling another agent's ask is outside the delegation list")); }
                body["fields"]["acted_for"] = json!(proof.person);
                body["fields"]["delegation"] = json!(proof);
                let mut response_evidence = input.evidence.clone();
                super::delegation::add_evidence(tx, &mut response_evidence, proof)?;
                body["evidence"] = json!(response_evidence);
            }
            let claim = append_claim_tx(tx, &self.origin, &subject, kind, Some(&input.actor), &body, &evidence, None).map_err(claim_append_error)?;
            project(tx, &claim)?;
            if let Some(ask) = ask.as_ref().filter(|a| !cancel && a.body["fields"]["origin_step"].is_null() && !is_update(a)) {
                send_answer_tx(tx, &self.origin, ask, &claim, &summary, answer.as_ref())?;
            }
            if let Some(origin) = ask.as_ref().and_then(|a| a.body["fields"]["origin_step"].as_str()) {
                if let Some(origin_view) = step(tx, origin).map_err(internal)? {
                    if origin_view.status == "ready" {
                        let response = append_claim_tx(tx, &self.origin, origin, "step-run.state", Some("daemon/runtime"),
                            &json!({"fields": {"status": "ready", "attempt": origin_view.attempt, "readiness_epoch": origin_view.readiness_epoch,
                                "reason": format!("Person response: {summary}")}}), &[claim.id], None).map_err(claim_append_error)?;
                        project_mission_run_update(tx, &response)?;
                    }
                }
            }
            step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the person step disappeared"))
        }).map_err(internal)?
    }

    fn ask_images(&self, step_subject: &str) -> Result<Option<AskImages>> {
        let connection = self.readers.get();
        let Some(ask) = request(&connection, step_subject)? else {
            return Ok(None);
        };
        let attachments = ask.body["fields"]
            .pointer("/request/attachments")
            .cloned()
            .map(serde_json::from_value::<Vec<MessageAttachment>>)
            .transpose()?
            .unwrap_or_default();
        Ok(Some(AskImages {
            asker: ask.actor.unwrap_or_default(),
            person: step(&connection, step_subject)?.and_then(|view| view.assigned_to),
            attachments,
        }))
    }

    fn reconcile_person_asks(&self) -> Result<bool> {
        /// How many asks one pass ends, each with its cancellation, message and update.
        const ENDED_ASKS_PER_PASS: usize = 32;
        // Drop this read connection before submitting any cancellation transaction.
        let has_asks: bool = {
            let connection = self.readers.get();
            connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h0c1c0f4ba68d owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ HAS_PERSON_ASKS_FOR_RECONCILE)?
                .query_row(/* sqlite-large-value: allow-for-now lv-h37ac4e9738af owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?
        };
        if !has_asks {
            // A concurrent new ask is handled by the next notified reconcile pass.
            return Ok(false);
        }
        self.connection.batched(|tx| -> Result<bool> {
            let asks = person_asks_for_reconcile(tx)?;
            let mut changed = false;
            let mut ended = 0;
            for ask in asks {
                // Each ending writes a cancellation, a message and an update while holding the
                // single writer; the rest end on the next pass, which this one's commit wakes.
                if ended == ENDED_ASKS_PER_PASS {
                    break;
                }
                // A previous candidate may have cancelled this same subject. Recheck the raw
                // status inside this transaction; the joined candidate list is not authority.
                let Some(view) = step(tx, &ask.subject)? else { continue };
                if matches!(view.status.as_str(), "ready" | "pending") && !current(tx, &ask, now_ms())? {
                    let claim = append_claim_tx(tx, &self.origin, &ask.subject, "work.person-cancelled", Some("daemon/runtime"),
                        &json!({"fields": {"attempt": view.attempt, "status": "cancelled", "summary": "the requester, origin or owning run ended", "key": format!("person-owner-ended:{}", ask.id)}}), std::slice::from_ref(&ask.id), None)?;
                    project(tx, &claim).map_err(anyhow::Error::new)?;
                    // Telling is best effort: a notice that cannot be written must never undo
                    // this cancellation or the others in this pass.
                    if !is_update(&ask) {
                        tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hf940ddda012d owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ "SAVEPOINT tell_ask_ended")?;
                        match self.tell_ask_ended(tx, &ask) {
                            Ok(()) => tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h52cf322b5cc5 owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ "RELEASE tell_ask_ended")?,
                            Err(error) => {
                                tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h5b793186ea23 owner=st3_missions::store::person_work TODO=trace-statement-and-value-representation */ "ROLLBACK TO tell_ask_ended; RELEASE tell_ask_ended")?;
                                tracing::warn!("the ended ask {} could not be told: {error:#}", ask.subject);
                            }
                        }
                    }
                    ended += 1;
                    changed = true;
                }
            }
            Ok(changed)
        }).map_err(anyhow::Error::msg)?
    }

    fn post_update(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error> {
        if input.step.is_some() || input.new_run.is_some() {
            return Err(St3Error::new(
                "ambiguous-ask-owner",
                "an update names the work it is about in its request, not with a step or new run",
            ));
        }
        let structured = canonical_request(input)?;
        let about = structured["about"].as_str().unwrap_or_default().to_owned();
        self.connection.batched(|tx| {
            if !declaration_live(tx, &input.actor).map_err(internal)? {
                return Err(St3Error::new("missing-ask-owner", "the agent posting an update must have a live declaration"));
            }
            if !person_asked(tx, &input.person, &input.actor, &about).map_err(internal)? {
                return Err(St3Error::new("update-not-asked", format!(
                    "{} did not ask for `{about}`: an update is about the person's own run or step, or their message to you",
                    input.person)));
            }
            self.append_update_tx(tx, &input.actor, &input.person, &input.title, &input.reason, &input.idempotency_key, &structured)
        }).map_err(internal)?
    }

    fn tell_ask_ended(&self, tx: &Transaction<'_>, ask: &ClaimRecord) -> Result<()> {
        let fields = &ask.body["fields"];
        let requester = ask.actor.as_deref().unwrap_or_default();
        let person = fields["person"].as_str().unwrap_or_default();
        let title = fields["title"].as_str().unwrap_or_default();
        let why = if !requester.starts_with("daemon/") && !declaration_live(tx, requester)? {
            format!("{requester} was stopped or retired")
        } else if fields["origin_step"].is_string() {
            "the step that asked moved on".to_owned()
        } else {
            "the run it belonged to ended".to_owned()
        };
        if requester.starts_with("agent/") {
            let key = format!("st3-ask-ended:{}", ask.id);
            let message = format!("message/{}", &hex::encode(Sha256::digest(key.as_bytes()))[..16]);
            let content = format!(
                "Your ask to {person}, \"{title}\" ({}), was cancelled because {why}. {person} will not answer it. If you still need the answer, ask again with `st work ask`.",
                ask.subject
            );
            append_claim_tx(tx, &self.origin, &message, "message.sent", Some("daemon/runtime"), &json!({"fields": {
                "from": "daemon/runtime", "to": requester, "content": content, "status": "sent",
                "title": format!("Ask cancelled: {title}"), "in_reply_to": null,
                "tags": [format!("st3-ask-ended:{}", ask.subject)]}}), std::slice::from_ref(&ask.id), None)?;
        }
        if person.starts_with("person/") {
            let update = json!({"version": 1, "type": "update", "about": ask.subject});
            self.append_update_tx(tx, "daemon/runtime", person,
                &format!("Cancelled: {title}"),
                &format!("{requester} asked you \"{title}\". That ask ended because {why}, so nothing waits on your answer now."),
                &format!("st3-ask-ended:{}", ask.id), &update)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_update_tx(
            &self,
            tx: &Transaction<'_>,
            actor: &str,
            person: &str,
            title: &str,
            reason: &str,
            idempotency_key: &str,
            structured: &Value,
        ) -> Result<StepRunView, St3Error> {
        let about = structured["about"].as_str().unwrap_or_default();
        {
            let identity = serde_json::to_string(&(actor, person, about, idempotency_key)).map_err(internal)?;
            let hash = hex::encode(Sha256::digest(identity.as_bytes()));
            let generation = format!("update-{}", &hash[..32]);
            let subject = format!("step-run/{generation}/update");
            if let Some(existing) = request(tx, &subject).map_err(internal)? {
                if existing.body["fields"]["title"] != title || existing.body["fields"]["reason"] != reason
                    || existing.body["fields"]["request"] != *structured {
                    return Err(St3Error::new("idempotency-conflict", "this update key already names another update"));
                }
                return step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the update is no longer retained"));
            }
            let mission_id = format!("person-update/{}", &hash[..32]);
            let kdl = format!("version 2\nmission {mission_id:?} state=\"ready\" {{ goal {:?}; step \"update\" {{ assigned-to {:?}; goal {:?}; }} }}", title, person, reason);
            let mut intent = crate::graph::parse_internal_intent(&kdl, &self.origin)?;
            let mission = intent.missions.remove(&mission_id).ok_or_else(|| St3Error::new("internal", "the update mission could not be parsed"))?;
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.person-asked", Some(actor),
                &with_request(json!({"fields": {"run": format!("mission-run/person-update/{}", &hash[..32]),
                    "generation": format!("run-generation/{generation}"), "person": person, "title": title,
                    "reason": reason, "key": idempotency_key, "attempt": 1, "status": "ready", "mission_spec": mission}}), structured),
                &[], None).map_err(claim_append_error)?;
            project(tx, &claim)?;
            step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the update could not be projected"))
        }
    }

    fn ask_person_in_new_run(&self, input: &PersonAskRequest) -> Result<StepRunView, St3Error> {
        let name = input
            .new_run
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| St3Error::new("missing-ask-owner", "specify --step or --new-run"))?;
        if input.step.is_some() {
            return Err(St3Error::new(
                "ambiguous-ask-owner",
                "choose either --step or --new-run",
            ));
        }
        let structured = canonical_request(input)?;
        self.connection.batched(|tx| {
            if !declaration_live(tx, &input.actor).map_err(internal)? {
                return Err(St3Error::new("missing-ask-owner", "the requester must have a live declaration"));
            }
            let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM step_runs WHERE lease_owner=?1 AND status IN ('claimed','working'))",
                [&input.actor], |row| row.get(0)).map_err(internal)?;
            if active { return Err(St3Error::new("ambiguous-ask-owner", "use --step while you hold claimed work")); }
            let desired = current_desired_row(tx, &input.actor).map_err(internal)?.unwrap();
            // A top-level seat's ask survives its re-declaration, so asking again with the same key
            // after a restart or harness change finds the same ask instead of raising another.
            let declaration = desired.owner_run.is_some().then_some(&desired.claim_id);
            let key = serde_json::to_string(&(&input.actor, name, &desired.owner_run, &desired.owner_generation, declaration, &input.idempotency_key)).map_err(internal)?;
            let hash = hex::encode(Sha256::digest(key.as_bytes()));
            let generation = format!("ask-{}", &hash[..32]);
            let subject = format!("step-run/{generation}/ask");
            if let Some(existing) = request(tx, &subject).map_err(internal)? {
                if existing.body["fields"]["person"] != input.person || existing.body["fields"]["title"] != input.title || existing.body["fields"]["reason"] != input.reason
                    || existing.body["fields"]["request"] != structured {
                    return Err(St3Error::new("idempotency-conflict", "this ask key already names another question"));
                }
                return step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the ask is no longer retained"));
            }
            let mission_id = format!("person-ask/{}", &hash[..32]);
            let kdl = format!("version 2\nmission {mission_id:?} state=\"ready\" {{ goal {:?}; step \"ask\" {{ assigned-to {:?}; goal {:?}; }} }}", input.title, input.person, input.reason);
            let mut intent = crate::graph::parse_internal_intent(&kdl, &self.origin)?;
            let mission = intent.missions.remove(&mission_id).ok_or_else(|| St3Error::new("internal", "the person mission could not be parsed"))?;
            let run = format!("mission-run/person-ask/{}", &hash[..32]);
            let waiting_since = input.legacy_request.as_ref().map(|id| tx.query_row("SELECT accepted_at_unix_ms FROM claims WHERE id=?1 AND kind='attention.requested'", [id], |row| row.get::<_, String>(0))).transpose().map_err(internal)?;
            let evidence = input.legacy_request.iter().cloned().collect::<Vec<_>>();
            let claim = append_claim_tx(tx, &self.origin, &subject, "work.person-asked", Some(&input.actor),
                &with_request(json!({"fields": {"run": run, "generation": format!("run-generation/{generation}"),
                    "person": input.person, "title": input.title, "reason": input.reason, "key": input.idempotency_key,
                    "attempt": 1, "status": "ready", "mission_spec": mission, "owner_run": desired.owner_run,
                    "owner_generation": desired.owner_generation, "requester_declaration": desired.claim_id, "waiting_since": waiting_since, "legacy_request": input.legacy_request}}), &structured), &evidence, None).map_err(claim_append_error)?;
            project(tx, &claim)?;
            step(tx, &subject).map_err(internal)?.ok_or_else(|| St3Error::new("missing-step-run", "the ask could not be projected"))
        }).map_err(internal)?
    }
}


#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::person_work_tests_shared_seats::*;
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::person_work_tests_shared_seats::{TEST_FLEET, gateway_decision, receive_exchange};
    use super::*;
    pub use st3_delivery::store::person_work::tests::*;

    #[test]
    fn empty_person_ask_reconciliation_does_not_wait_for_the_writer() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap();
        let (held, is_held) = std::sync::mpsc::sync_channel(1);
        let (release, released) = std::sync::mpsc::sync_channel(1);
        let (done, result) = std::sync::mpsc::sync_channel(1);
        let completed = std::thread::scope(|scope| {
            let store = &store;
            let writer = scope.spawn(move || {
                store.hold_writer_for_test(|| {
                    held.send(()).unwrap();
                    released.recv().unwrap();
                });
            });
            is_held.recv().unwrap();
            let reconcile = scope.spawn(move || {
                done.send(store.reconcile_person_asks()).unwrap();
            });
            let completed = result.recv_timeout(std::time::Duration::from_secs(2));
            // Always release the real writer before joining, including on a regression.
            release.send(()).unwrap();
            writer.join().unwrap();
            reconcile.join().unwrap();
            completed
        });
        assert!(
            !completed.expect("empty reconciliation submitted a writer job").unwrap(),
            "empty reconciliation must not report a change"
        );
    }
}
