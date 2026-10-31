//! Person asks are runtime steps. The immutable asking claim supplies their ownership fence;
//! step_runs supplies their state. No separate attention record is created.
use super::*;
use crate::model::PersonRenameRequest;

use crate::person_request::StructuredRequest;

#[cfg(test)]
mod reconcile_candidates_tests;
#[cfg(test)]
mod revision_provenance_tests;

/// The structured request an ask claim carries. An ask from a newer build whose request this
/// build cannot read is refused rather than answered as free text.
pub fn structured_request(ask: &ClaimRecord) -> Result<Option<StructuredRequest>, St3Error> {
    let Some(request) = ask.body["fields"].get("request").filter(|r| !r.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value::<StructuredRequest>(request.clone())
        .ok()
        .filter(|request| request.version == crate::person_request::REQUEST_VERSION)
        .map(Some)
        .ok_or_else(|| {
            St3Error::new(
                "unsupported-person-request",
                "this ask's request needs a newer st to answer",
            )
        })
}

/// How long a person has to answer an ask that names no deadline, unless `[asks]
/// default_deadline` says otherwise.
pub const DEFAULT_ASK_DEADLINE_MS: u64 = 24 * 60 * 60 * 1000;

/// The `meta` key holding when this node first counted ask deadlines, the first reconcile pass
/// after `[asks] deadlines` was turned on. An ask made before then carries none, so its deadline
/// counts from then rather than from when it was asked.
pub const ASK_DEADLINES_SINCE: &str = "person_ask_deadlines_since_unix_ms";

/// When this node started counting ask deadlines; zero before it has.
pub fn ask_deadlines_since(connection: &Connection) -> Result<u128> {
    Ok(connection
        .query_row("SELECT value FROM meta WHERE key=?1", [ASK_DEADLINES_SINCE], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0))
}

/// The `meta` key holding this node's `[asks] default_deadline`, so that transaction code, such
/// as a person's home, reads the same default the reconciler resolves asks with.
pub const ASK_DEFAULT_DEADLINE: &str = "person_ask_default_deadline_ms";

/// When this node started counting ask deadlines, and its default deadline.
pub fn ask_deadline_policy(connection: &Connection) -> Result<(u128, u64)> {
    let default = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [ASK_DEFAULT_DEADLINE],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ASK_DEADLINE_MS);
    Ok((ask_deadlines_since(connection)?, default))
}

/// Records, once, that this node counts ask deadlines from `now`. Returns whether it had not.
pub fn start_counting_ask_deadlines(connection: &Connection, now: u128) -> Result<bool> {
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h3beacc1959d7 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,?2)",
        params![ASK_DEADLINES_SINCE, now.to_string()],
    )? > 0)
}

/// Forgets when this node started counting, on a pass that sees deadlines off: an ask made
/// while they were off then counts from when they are next turned on, not from the first time.
/// Returns whether it had been counting.
pub fn stop_counting_ask_deadlines(connection: &Connection) -> Result<bool> {
    Ok(connection.execute(/* sqlite-large-value: allow-for-now lv-hdb4786de6904 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [ASK_DEADLINES_SINCE])? > 0)
}

/// Which of an ask's responses stand once a deadline has applied its default. Members resolve
/// independently, so a person may answer on another member before the deadline while the
/// ask's own member applies the default: a person's answer accepted before the cutoff (the
/// ask's deadline, or when the default was applied for an ask made without one) wins, and one
/// accepted at or after it does not. Every member holds the same claims with the same accept
/// times, so every member keeps the same responses. Without a deadline response all stand.
pub fn standing_responses<T>(
    responses: Vec<T>,
    by_deadline: impl Fn(&T) -> bool,
    is_person_answer: impl Fn(&T) -> bool,
    accepted_at: impl Fn(&T) -> u128,
    deadline: Option<u128>,
) -> (Vec<T>, bool) {
    let Some(applied) = responses.iter().find(|response| by_deadline(response)) else {
        return (responses, false);
    };
    let cutoff = deadline.unwrap_or_else(|| accepted_at(applied));
    let answered_in_time = responses
        .iter()
        .any(|response| is_person_answer(response) && accepted_at(response) < cutoff);
    let standing = responses
        .into_iter()
        .filter(|response| {
            if by_deadline(response) {
                !answered_in_time
            } else {
                !is_person_answer(response) || accepted_at(response) < cutoff
            }
        })
        .collect();
    (standing, answered_in_time)
}

/// When an ask resolves by itself if the person has not answered: the deadline it was asked
/// with, or, for an ask made before deadlines, `default_ms` after it was asked or after this
/// node began counting (`since`), whichever is later. An update waits for nobody and has none.
pub fn ask_deadline(ask: &ClaimRecord, since: u128, default_ms: u64) -> Option<u128> {
    if is_update(ask) {
        return None;
    }
    if let Some(deadline) = ask.body["fields"]["deadline_unix_ms"].as_u64() {
        return Some(u128::from(deadline));
    }
    Some(ask.accepted_at_unix_ms.max(since) + u128::from(default_ms))
}

/// The answer id that applies at an ask's deadline, if it names one.
pub fn ask_default_answer(ask: &ClaimRecord) -> Option<&str> {
    ask.body["fields"]["default_answer"].as_str()
}

impl Store {
    /// Sets how long a person has to answer an ask that names no deadline. It is kept in `meta`
    /// too, written only when it changes, for the asks that carry no deadline of their own.
    pub fn set_default_ask_deadline_ms(&self, deadline_ms: u64) -> Result<()> {
        self.default_ask_deadline_ms
            .store(deadline_ms, std::sync::atomic::Ordering::Relaxed);
        if ask_deadline_policy(&self.readers.get())?.1 == deadline_ms {
            return Ok(());
        }
        self.connection
            .batched(|tx| {
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-hcb7506bd8ba2 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
                    params![ASK_DEFAULT_DEADLINE, deadline_ms.to_string()],
                )
                .map(drop)
            })
            .map_err(anyhow::Error::msg)??;
        self.ask_deadline_policy_changed();
        Ok(())
    }

    /// The ask deadline policy in `meta` changed. A person's home shows deadlines by it, and
    /// no claim tells the attention list refresher, so wake it: its next refresh sees the
    /// publication's policy differ and folds again.
    pub fn ask_deadline_policy_changed(&self) {
        if let Some(wake) = self.fractal.attention_list.refresher.get() {
            wake.notify_one();
        }
    }

    pub fn default_ask_deadline_ms(&self) -> u64 {
        self.default_ask_deadline_ms
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turns ask deadlines on or off (`[asks] deadlines`).
    pub fn set_ask_deadlines(&self, enabled: bool) {
        self.ask_deadlines
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn ask_deadlines(&self) -> bool {
        self.ask_deadlines.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub fn validate_delegated_answer(
    ask: &ClaimRecord,
    fields: &BTreeMap<String, Value>,
) -> Result<(), St3Error> {
    if fields.get("status").and_then(Value::as_str) != Some("completed")
        || fields
            .get("summary")
            .and_then(Value::as_str)
            .is_none_or(|summary| summary.trim().is_empty())
    {
        return Err(St3Error::new(
            "delegation-refused",
            "a completion needs completed status and a summary",
        ));
    }
    if let Some(request) = structured_request(ask)? {
        let submitted = fields.get("answer").ok_or_else(|| {
            St3Error::new("delegation-refused", "the structured answer is missing")
        })?;
        let input = crate::person_request::AnswerInput {
            id: submitted
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            text: submitted
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        if request.answer(Some(&input), fields["summary"].as_str().unwrap())? != *submitted {
            return Err(St3Error::new(
                "delegation-refused",
                "the answer does not match the offered structured answer",
            ));
        }
    } else if fields.contains_key("answer") {
        return Err(St3Error::new(
            "delegation-refused",
            "a free-text ask has no named answer",
        ));
    }
    Ok(())
}

const STEP_QUERY: &str = "SELECT subject, run_id, step_path, definition_hash, status, attempt,
 assignee, available_to, agentless, title, goals, worker_reported, lease_owner,
 lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms,
 created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
 FROM step_runs WHERE subject=?1";

pub fn step(connection: &Connection, subject: &str) -> Result<Option<StepRunView>> {
    Ok(connection
        .query_row(/* sqlite-large-value: allow-for-now lv-he9da72925547 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ STEP_QUERY, [subject], step_run_from_row)
        .optional()?)
}

pub fn request(connection: &Connection, subject: &str) -> Result<Option<ClaimRecord>> {
    Ok(connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-h43a8ecb87729 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id, store_index, batch_id, subject, kind,
        origin, actor, body, predecessors, accepted_at_unix_ms FROM claims
        WHERE subject=?1 AND kind='work.person-asked' ORDER BY CANONICAL_ASC(claims) LIMIT 1",
            ),
            [subject],
            claim_from_row,
        )
        .optional()?)
}

pub fn declaration_live(connection: &Connection, subject: &str) -> Result<bool> {
    let row = current_desired_row(connection, subject)?;
    let Some(row) = row else { return Ok(false) };
    let body: Value = serde_json::from_str(&row.body)?;
    if row.kind == "stop"
        || body
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop")
    {
        return Ok(false);
    }
    if let Some(run) = row.owner_run.as_deref() {
        if !run_live(connection, run, row.owner_generation.as_deref(), false)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn run_live(
    connection: &Connection,
    run: &str,
    generation: Option<&str>,
    failure: bool,
) -> Result<bool> {
    Ok(run_liveness(connection, run, generation, failure)?.is_ok())
}

/// Whether `run` still matters, or why not: it and each run above it exist, are open and on
/// the generation their parent step belongs to, each parent step is still open, and a
/// subscription or schedule that delivered it still runs. `failure` keeps a failed run or
/// parent step live, for the fault that names it.
pub fn run_liveness(
    connection: &Connection,
    run: &str,
    generation: Option<&str>,
    failure: bool,
) -> Result<std::result::Result<(), String>> {
    let mut run = run.strip_prefix("mission-run/").unwrap_or(run).to_owned();
    let mut expected_generation = generation.map(str::to_owned);
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(run.clone()) {
            return Ok(Err(format!(
                "mission run `mission-run/{run}` is its own ancestor"
            )));
        }
        let header = mission_run_header_tx(connection, &run).optional()?;
        let Some(header) = header else {
            return Ok(Err(format!("mission run `mission-run/{run}` is gone")));
        };
        if let Some(expected) = expected_generation
            .as_ref()
            .filter(|expected| *expected != &header.generation)
        {
            return Ok(Err(format!(
                "mission run `{}` moved on from {expected} to {}",
                header.subject, header.generation
            )));
        }
        if is_terminal_run_state(&header.status) && !(failure && header.status == "failed") {
            return Ok(Err(format!(
                "mission run `{}` is {}",
                header.subject, header.status
            )));
        }
        if let Some(parent) = header.parent_step_run.as_deref() {
            let Some(parent) = step(connection, parent)? else {
                // Subscription and schedule deliveries use their declaration as the parent,
                // normalized with a step-run prefix. They do not create a synthetic work step.
                let subject = parent.strip_prefix("step-run/").unwrap_or(parent);
                let Some(owner) = current_desired_row(connection, subject)? else {
                    return Ok(Err(format!("`{subject}`, which started it, is gone")));
                };
                let body: Value = serde_json::from_str(&owner.body)?;
                if !matches!(owner.kind.as_str(), "subscription" | "schedule") {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, is not a step, subscription or schedule"
                    )));
                }
                if body
                    .get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop")
                {
                    return Ok(Err(format!("`{subject}`, which started it, is stopped")));
                }
                let Some(owner_run) = owner.owner_run else {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, has no owning run"
                    )));
                };
                let owner_header =
                    mission_run_header_tx(connection, owner_run.trim_start_matches("mission-run/"))
                        .optional()?;
                if owner_header
                    .is_none_or(|owner| owner.root_mission_run != header.root_mission_run)
                {
                    return Ok(Err(format!(
                        "`{subject}`, which started it, now belongs to another run"
                    )));
                }
                run = owner_run.trim_start_matches("mission-run/").into();
                expected_generation = owner.owner_generation;
                continue;
            };
            if matches!(parent.status.as_str(), "completed" | "cancelled")
                || (parent.status == "failed" && !failure)
            {
                return Ok(Err(format!(
                    "its parent step `{}` is {}",
                    parent.subject, parent.status
                )));
            }
            run = parent.run.trim_start_matches("mission-run/").into();
            expected_generation = Some(parent.generation);
        } else {
            let root = mission_run_header_tx(
                connection,
                header.root_mission_run.trim_start_matches("mission-run/"),
            )
            .optional()?;
            return Ok(match root {
                None => Err(format!(
                    "its root mission run `{}` is gone",
                    header.root_mission_run
                )),
                Some(root)
                    if is_terminal_run_state(&root.status)
                        && !(failure && root.status == "failed") =>
                {
                    Err(format!(
                        "its root mission run `{}` is {}",
                        root.subject, root.status
                    ))
                }
                Some(_) => Ok(()),
            });
        }
    }
}

pub fn current(connection: &Connection, ask: &ClaimRecord, as_of: u128) -> Result<bool> {
    if ask.accepted_at_unix_ms > as_of {
        return Ok(false);
    }
    let fields = &ask.body["fields"];
    let Some(view) = step(connection, &ask.subject)? else {
        return Ok(false);
    };
    let requester = ask.actor.as_deref().unwrap_or_default();
    // A daemon asks for its own policy, not for a seat, so no declaration fences it. An update
    // waits for nobody, so it stays until the person reads it, even after its poster stops.
    let unfenced = requester.starts_with("daemon/") || is_update(ask);
    if !matches!(view.status.as_str(), "pending" | "ready")
        || !run_live(connection, &view.run, Some(&view.generation), false)?
    {
        return Ok(false);
    }
    let requester_live = unfenced || declaration_live(connection, requester)?;
    let retiring = !requester_live && super::rollouts::retiring_ask_live(connection, ask)?;
    if !requester_live && !retiring {
        return Ok(false);
    }
    if unfenced {
        return Ok(true);
    }
    let mut declarations = connection.prepare(/* sqlite-large-value: allow-for-now lv-hbc46e8c53f31 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='intent.desired' ORDER BY CANONICAL_ASC(claims)"))?;
    let ask_key = canonical::claim_key(connection, &ask.id)?;
    for declaration in declarations.query_map([requester], claim_from_row)? {
        let declaration = declaration?;
        if canonical::claim_key(connection, &declaration.id)? <= ask_key {
            continue;
        }
        if !retiring
            && (declaration.body["kind"] == "stop"
                || declaration.body["desired"]
                    .get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| children.len() == 1 && children[0]["name"] == "stop"))
        {
            return Ok(false);
        }
    }
    // A top-level seat's ask outlives its restarts and harness changes: only stopping or retiring
    // the seat (above) or cancelling the ask ends it, so a person never loses it to a restart. A
    // mission's seat lives and dies with its declaration, and so does its ask.
    if let Some(declaration) = fields["requester_declaration"].as_str()
        && !fields["owner_run"].is_null()
        && current_desired_row(connection, requester)?.is_none_or(|row| row.claim_id != declaration)
    {
        return Ok(false);
    }
    if let Some(owner) = fields["owner_run"].as_str() {
        if !run_live(
            connection,
            owner,
            fields["owner_generation"].as_str(),
            false,
        )? {
            return Ok(false);
        }
    }
    if let Some(origin) = fields["origin_step"].as_str() {
        let Some(origin) = step(connection, origin)? else {
            return Ok(false);
        };
        if origin.attempt as u64 != fields["origin_attempt"].as_u64().unwrap_or(0)
            || origin.status != "waiting-person"
            || origin.blocked_reason.as_deref() != Some(ask.subject.as_str())
            || origin.generation != view.generation
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn is_update(ask: &ClaimRecord) -> bool {
    ask.body["fields"]["request"]["type"] == "update"
}

/// Adds person responses to a step view: the responses to asks this attempt made, as
/// constraints and as data, or a person ask's own response as data.
pub fn enrich_responses(
    connection: &Connection,
    view: &mut StepRunView,
) -> rusqlite::Result<()> {
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h65cae3d401d5 owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT resolution.subject, resolution.actor, resolution.body, resolution.accepted_at_unix_ms,
          json_extract(request.body,'$.fields.origin_step')=?1, resolution.kind,
          json_extract(request.body,'$.fields.deadline_unix_ms') FROM claims resolution JOIN claims request
        ON request.subject=resolution.subject AND request.kind='work.person-asked'
        WHERE resolution.kind IN ('work.person-done','work.person-cancelled')
          -- Only asks this step made, or the step's own ask: an index walk, not every answer.
          AND resolution.subject IN (
            SELECT subject FROM claims WHERE kind='work.person-asked'
              AND json_extract(body,'$.fields.origin_step')=?1
            UNION SELECT ?1)
          AND ((json_extract(request.body,'$.fields.origin_step')=?1
                AND json_extract(request.body,'$.fields.origin_attempt')=?2)
            OR (resolution.subject=?1 AND json_extract(resolution.body,'$.fields.attempt')=?2))
        ORDER BY CANONICAL_ASC(resolution)",
    ))?;
    let responses = query
        .query_map(params![view.subject, view.attempt], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Each ask's responses, in canonical order, keeping only those that stand after a deadline.
    let mut asks: Vec<(String, Option<u128>, Vec<_>)> = Vec::new();
    for (ask, actor, body, accepted, from_origin, kind, deadline) in responses {
        let Ok(body) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        let response = (actor, body, accepted, from_origin, kind);
        match asks.iter_mut().find(|(subject, _, _)| *subject == ask) {
            Some((_, _, responses)) => responses.push(response),
            None => asks.push((ask, deadline.and_then(|deadline| u128::try_from(deadline).ok()), vec![response])),
        }
    }
    for (ask, deadline, responses) in asks {
        let (standing, overridden) = standing_responses(
            responses,
            |(_, body, _, _, _)| body["fields"]["resolved_by"] == "deadline",
            |(_, body, _, _, kind)| kind == "work.person-done" && body["fields"]["resolved_by"] != "deadline",
            |(_, _, accepted, _, _)| accepted.parse().unwrap_or(0),
            deadline,
        );
        if overridden && standing.iter().any(|(_, _, _, from_origin, _)| *from_origin) {
            view.constraints.push(format!(
                "Person response to {ask}: the person answered before its deadline, so their answer replaces the default the deadline applied"
            ));
        }
        for (actor, body, accepted, from_origin, _) in standing {
            enrich_response(view, ask.clone(), actor, &body, accepted, from_origin);
        }
    }
    Ok(())
}

/// Adds one standing response to the view, as a constraint for its asker and as data.
fn enrich_response(
    view: &mut StepRunView,
    ask: String,
    actor: Option<String>,
    body: &Value,
    accepted: String,
    from_origin: bool,
) {
    {
        let fields = &body["fields"];
        let Some(summary) = fields["summary"].as_str() else {
            return;
        };
        if from_origin {
            let mut constraint = format!("Person response: {summary}");
            if let Some(sender_class) = fields["sender_class"].as_str() {
                constraint.push_str(" [sender_class: ");
                constraint.push_str(sender_class);
                constraint.push(']');
            }
            if let Some(grant) = fields["assistant_grant_id"].as_str() {
                constraint.push_str(" [assistant_grant_id: ");
                constraint.push_str(grant);
                constraint.push(']');
            }
            view.constraints.push(constraint);
        }
        view.person_answers.push(crate::model::PersonAnswerView {
            acted_for: fields
                .get("acted_for")
                .and_then(Value::as_str)
                .map(str::to_owned),
            sender_class: fields
                .get("sender_class")
                .and_then(Value::as_str)
                .map(str::to_owned),
            assistant_grant_id: fields
                .get("assistant_grant_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            assistant_owner: fields.get("assistant_owner").and_then(Value::as_str).map(str::to_owned),
            assistant_session_actor: fields.get("assistant_session_actor").and_then(Value::as_str).map(str::to_owned),
            ask,
            status: fields["status"].as_str().unwrap_or("completed").into(),
            summary: summary.into(),
            respondent: actor.unwrap_or_default(),
            answered_at_unix_ms: accepted.parse().unwrap_or(0),
            answer: fields
                .get("answer")
                .filter(|answer| answer.is_object())
                .cloned(),
            evidence: serde_json::from_value(body["evidence"].clone()).unwrap_or_default(),
            resolved_by: fields
                .get("resolved_by")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
}


/// Reassignment captures only the current open episode. The original ask remains immutable,
/// including its requester and originally addressed person.
pub(super) fn reassign_person_steps_tx(
    tx: &Transaction<'_>,
    origin: &str,
    input: &PersonRenameRequest,
) -> Result<Vec<String>, St3Error> {
    let as_of = now_ms();
    let mut query = tx.prepare(
        "SELECT subject FROM step_runs WHERE assignee=?1 AND status IN ('ready','pending') ORDER BY subject",
    ).map_err(internal)?;
    let subjects = query.query_map([&input.old_person], |row| row.get::<_, String>(0))
        .map_err(internal)?.collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
    let mut reassigned = Vec::new();
    for subject in subjects {
        let Some(mut view) = step(tx, &subject).map_err(internal)? else { continue };
        apply_effective_step_state(tx, &mut view, as_of).map_err(internal)?;
        if !matches!(view.status.as_str(), "ready" | "pending")
            || !run_live(tx, &view.run, Some(&view.generation), false).map_err(internal)?
        {
            continue;
        }
        let ask = request(tx, &subject).map_err(internal)?;
        if let Some(ask) = &ask
            && !current(tx, ask, as_of).map_err(internal)?
        {
            continue;
        }
        let episode = ask.as_ref().map(|ask| ask.id.clone()).unwrap_or_else(|| {
            format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch)
        });
        let evidence = ask.map(|ask| vec![ask.id]).unwrap_or_default();
        let claim = append_claim_tx(tx, origin, &subject, "work.person-reassigned", Some(&input.actor),
            &json!({"fields": {"previous_person": input.old_person, "person": input.new_person,
                "run": view.run, "generation": view.generation, "attempt": view.attempt, "episode": episode}}),
            &evidence, None).map_err(claim_append_error)?;
        project(tx, &claim)?;
        reassigned.push(subject);
    }
    Ok(reassigned)
}

/// Shared projection for a one-step runtime run, used by asks and spontaneous work.
pub fn project_minimal_run(
    tx: &Transaction<'_>,
    claim: &ClaimRecord,
    mission: &MissionSpec,
    run: &str,
    generation: &str,
    root: &str,
    reason: &str,
) -> Result<(), St3Error> {
    let at = claim.accepted_at_unix_ms.to_string();
    tx.execute(/* sqlite-large-value: allow-for-now lv-hca41158cfea1 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_revisions(mission_id,revision,state,body,claim_id,created_index)
                VALUES(?1,?2,'ready',?3,?4,?5)", params![mission.id, mission.revision, serde_json::to_string(&mission).map_err(internal)?, claim.id, claim.store_index]).map_err(internal)?;
    // Independent daemons can create the same update revision. Retained revision history
    // survives replay, so its source must follow canonical claim order rather than arrival.
    let previous: String = tx
        .query_row(
            "SELECT claim_id FROM mission_revisions WHERE mission_id=?1 AND revision=?2",
            params![mission.id, mission.revision],
            |row| row.get(0),
        )
        .map_err(internal)?;
    if previous != claim.id
        && canonical::claim_key(tx, &claim.id).map_err(internal)?
            < canonical::claim_key(tx, &previous).map_err(internal)?
    {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h4249b590ecda owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_revisions SET claim_id=?3 WHERE mission_id=?1 AND revision=?2",
            params![mission.id, mission.revision, claim.id],
        )
        .map_err(internal)?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-hc66679f685d8 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_definitions(mission_id,revision,state,claim_id) VALUES(?1,?2,'ready',?3)", params![mission.id, mission.revision, claim.id]).map_err(internal)?;
    // A run-tree rebuild retains this definition too, and a full replay keeps the canonically
    // first claim's. Independent daemons can write different revisions of one internal mission:
    // each words an ended ask's reason from its own view. So the canonically first claim gives
    // the revision as well as the source, over a definition another minimal run wrote; this
    // does not change definition state, and a published definition, of any revision, stays.
    let definition_source: Option<(String, bool)> = tx
        .query_row(
            "SELECT claim_id, EXISTS(SELECT 1 FROM claims
                 WHERE claims.id=mission_definitions.claim_id
                 AND claims.kind IN ('work.person-asked','mission-run.created'))
             FROM mission_definitions WHERE mission_id=?1",
            [&mission.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(internal)?;
    if let Some((previous, replaceable)) = definition_source
        && replaceable
        && previous != claim.id
        && canonical::claim_key(tx, &claim.id).map_err(internal)?
            < canonical::claim_key(tx, &previous).map_err(internal)?
    {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-h5397dfc4c7e7 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_definitions SET revision=?2, claim_id=?3 WHERE mission_id=?1",
            params![mission.id, mission.revision, claim.id],
        )
        .map_err(internal)?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h540720b8262b owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,
                workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
                VALUES(?1,?2,?3,?4,?3,?5,'.',?6,'{}','run','running','normal',?7,?7)",
                params![run, mission.id, mission.revision, generation, root, claim.actor, at]).map_err(internal)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-he893baab2fb0 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO run_generations(id,run_id,revision,status,actor,reason,created_at_unix_ms,updated_at_unix_ms)
                VALUES(?1,?2,?3,'running',?4,?6,?5,?5)", params![generation, run, mission.revision, claim.actor, at, reason]).map_err(internal)?;
    Ok(())
}

pub fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<bool, St3Error> {
    let fields = &claim.body["fields"];
    if claim.kind == "work.person-reassigned" {
        let accepted: bool = tx.query_row(
            /* sqlite-large-value: allow-for-now lv-hfbb687ae03ed owner=fractal_store::store::person_work TODO=trace-statement-and-value-representation */
            &format!("SELECT {} FROM claims reassignment WHERE reassignment.id=?1",
                person_rename::accepted_reassignment_sql("reassignment", "previous_person", "person")),
            [&claim.id], |row| row.get(0)).map_err(internal)?;
        if !accepted { return Ok(true); }
        let Some(view) = step(tx, &claim.subject).map_err(internal)? else {
            return Ok(true);
        };
        let ask = request(tx, &claim.subject).map_err(internal)?;
        let episode = ask.map(|ask| ask.id).unwrap_or_else(|| {
            format!("{}:{}:{}", view.generation, view.attempt, view.readiness_epoch)
        });
        if fields["run"].as_str() != Some(view.run.as_str())
            || fields["generation"].as_str() != Some(view.generation.as_str())
            || fields["attempt"].as_u64() != Some(u64::from(view.attempt))
            || fields["episode"].as_str() != Some(episode.as_str())
        {
            return Ok(true);
        }
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-hf6f5b79d7be9 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */
            "UPDATE step_runs SET assignee=?2,updated_at_unix_ms=?3
             WHERE subject=?1 AND assignee=?4 AND status IN ('ready','pending')",
            params![claim.subject, fields["person"].as_str(), claim.accepted_at_unix_ms.to_string(),
                fields["previous_person"].as_str()],
        ).map_err(internal)?;
        return Ok(true);
    }
    if claim.kind == "work.person-asked" {
        let run = fields["run"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("mission-run/");
        let generation = fields["generation"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("run-generation/");
        if let Some(spec) = fields.get("mission_spec") {
            let mission: MissionSpec = serde_json::from_value(spec.clone()).map_err(internal)?;
            let root = fields["owner_run"]
                .as_str()
                .unwrap_or(run)
                .trim_start_matches("mission-run/");
            project_minimal_run(tx, claim, &mission, run, generation, root, "person ask")?;
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-hfddcac254de6 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,
            attempt,assignee,available_to,agentless,title,goals,created_at_unix_ms,updated_at_unix_ms,constraints)
            VALUES(?1,?2,?3,?4,?5,'ready',1,?6,'[]',0,?7,?8,?9,?9,'[]')",
            params![claim.subject, run, generation, claim.subject.rsplit('/').next(), claim.id,
                fields["person"].as_str(), fields["title"].as_str(), serde_json::to_string(&vec![fields["reason"].as_str().unwrap_or_default()]).map_err(internal)?, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
        if step_owner_is_terminal_tx(tx, &claim.subject)? {
            tx.execute(/* sqlite-large-value: allow-for-now lv-he796c14d0fd2 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='cancelled',blocked_reason='the owning run ended' WHERE subject=?1", [&claim.subject]).map_err(internal)?;
            return Ok(true);
        }
        if let Some(origin) = fields["origin_step"].as_str() {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h2c813e50cf88 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='waiting-person',lease_owner=NULL,lease_incarnation=NULL,
                lease_expires_at_unix_ms=NULL,blocked_reason=?3,updated_at_unix_ms=?4
                WHERE subject=?1 AND attempt=?2 AND status IN ('claimed','working','ready','blocked')",
                params![origin, fields["origin_attempt"].as_u64(), claim.subject, claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-h6462186c014f owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_work_lease_renewals WHERE subject=?1",
                [origin],
            )
            .map_err(internal)?;
        }
        return Ok(true);
    }
    if !matches!(
        claim.kind.as_str(),
        "work.person-done" | "work.person-cancelled"
    ) {
        return Ok(false);
    }
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-ha5f213f5faef owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2,worker_reported=1,updated_at_unix_ms=?3
        WHERE subject=?1 AND attempt=?4 AND status IN ('ready','pending')",
        params![
            claim.subject,
            fields["status"].as_str(),
            claim.accepted_at_unix_ms.to_string(),
            fields["attempt"].as_u64()
        ],
    )
    .map_err(internal)?;
    if let Some(ask) = request(tx, &claim.subject).map_err(internal)? {
        if ask.body["fields"].get("mission_spec").is_some() {
            tx.execute(/* sqlite-large-value: allow-for-now lv-hbbd28d35da4a owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE mission_runs SET status=?2,phase='terminal',updated_at_unix_ms=?3 WHERE id=?1",
                params![ask.body["fields"]["run"].as_str().unwrap_or_default().trim_start_matches("mission-run/"), fields["status"].as_str(), claim.accepted_at_unix_ms.to_string()]).map_err(internal)?;
        }
        if let Some(origin) = ask.body["fields"]["origin_step"].as_str() {
            if !step_owner_is_terminal_tx(tx, origin)? {
                tx.execute(/* sqlite-large-value: allow-for-now lv-h4a39c8232ed5 owner=fractal_store::store::person_work TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status='ready',blocked_reason=NULL,readiness_epoch=readiness_epoch+1,
                    activated_at_unix_ms=?3,updated_at_unix_ms=?3 WHERE subject=?1 AND attempt=?2 AND status='waiting-person' AND blocked_reason=?4",
                    params![origin, ask.body["fields"]["origin_attempt"].as_u64(), claim.accepted_at_unix_ms.to_string(), ask.subject]).map_err(internal)?;
            }
        }
    }
    Ok(true)
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

    pub fn receive(source: &Store, target: &Store) {
        let exchange = source
            .export_replication_exchange_answering(
                TEST_FLEET,
                &target.replication_inventory().unwrap(),
                &[],
            )
            .unwrap();
        receive_exchange(target, &exchange);
    }


}

#[cfg(any(test, feature = "test-support"))]
pub mod reconcile_candidates_tests_shared_seats;

#[cfg(any(test, feature = "test-support"))]
pub mod revision_provenance_tests_shared_missions;

#[cfg(test)]
mod standing_responses_tests {
    use super::standing_responses;

    /// (name, by the deadline, a person's answer, accepted at)
    type Response = (&'static str, bool, bool, u128);

    fn standing(responses: Vec<Response>, deadline: Option<u128>) -> (Vec<&'static str>, bool) {
        let (kept, overridden) = standing_responses(
            responses,
            |response| response.1,
            |response| response.2,
            |response| response.3,
            deadline,
        );
        (kept.into_iter().map(|response| response.0).collect(), overridden)
    }

    #[test]
    fn a_person_answer_before_the_cutoff_beats_the_default_and_a_later_one_does_not() {
        let default = ("default", true, false, 1_000);
        let in_time = ("in-time", false, true, 999);
        let late = ("late", false, true, 1_000);
        let cancelled = ("cancelled", false, false, 2_000);
        // An answer accepted before the deadline stands over the default, in any arrival order.
        assert_eq!(standing(vec![default, in_time], Some(1_000)), (vec!["in-time"], true));
        assert_eq!(standing(vec![in_time, default], Some(1_000)), (vec!["in-time"], true));
        // At or after the deadline it does not, and the default stands alone.
        assert_eq!(standing(vec![default, late], Some(1_000)), (vec!["default"], false));
        // Without a deadline on the ask, the cutoff is when the default was applied.
        assert_eq!(standing(vec![("default", true, false, 1_500), ("late", false, true, 1_500)], None), (vec!["default"], false));
        assert_eq!(standing(vec![("default", true, false, 1_500), in_time], None), (vec!["in-time"], true));
        // A cancellation is not a person's answer and always stands; without a default all do.
        assert_eq!(standing(vec![default, cancelled], Some(1_000)), (vec!["default", "cancelled"], false));
        assert_eq!(standing(vec![late, cancelled], Some(1_000)), (vec!["late", "cancelled"], false));
    }
}
