//! Derived person work and operational episodes. Legacy attention claims are audit data only.
use super::*;


/// Derive one person step using the same source fences as the full reader.
pub fn person_attention_item(
    connection: &Connection,
    subject: &str,
    as_of: u128,
) -> Result<Option<AttentionItemView>> {
    let Some(view) = person_work::step(connection, subject)? else {
        return Ok(None);
    };
    if !matches!(view.status.as_str(), "pending" | "ready")
        || view
            .assigned_to
            .as_deref()
            .is_none_or(|a| {
                !a.get(..7)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("person/"))
            })
    {
        return Ok(None);
    }
    let ask = person_work::request(connection, subject)?;
    if let Some(ask) = &ask {
        if !person_work::current(connection, ask, as_of)? {
            return Ok(None);
        }
    } else if view.status != "ready"
        || !person_work::run_live(connection, &view.run, Some(&view.generation), false)?
    {
        return Ok(None);
    }
    let person = view.assigned_to.as_deref().unwrap_or_default();
    let episode = ask.as_ref().map(|ask| ask.id.clone()).unwrap_or_else(|| {
        format!(
            "{}:{}:{}",
            view.generation, view.attempt, view.readiness_epoch
        )
    });
    let activated: Option<String> = connection.query_row(
        "SELECT activated_at_unix_ms FROM step_runs WHERE subject=?1",
        [subject],
        |row| row.get(0),
    )?;
    let since = ask.as_ref().map_or(
        activated
            .and_then(|at| at.parse().ok())
            .unwrap_or(view.created_at_unix_ms),
        |ask| {
            ask.body["fields"]["waiting_since"]
                .as_str()
                .and_then(|value| value.parse().ok())
                .unwrap_or(ask.accepted_at_unix_ms)
        },
    );
    let request = ask
        .as_ref()
        .and_then(|ask| ask.body["fields"].get("request"))
        .filter(|request| request.is_object())
        .cloned();
    let (label, response): (&str, &[&str]) = match request.as_ref().and_then(|r| r["type"].as_str())
    {
        Some("decision" | "choice") => ("done", &["--answer", "ANSWER_ID"]),
        // An update clears when the person opens it or presses read.
        Some("update") => ("read", &["--answer", "read"]),
        Some(_) => ("done", &["--text", "FEEDBACK"]),
        None => ("done", &["--summary", "RESPONSE"]),
    };
    Ok(Some(AttentionItemView {
        episode,
        priority: "normal".into(),
        kind: "person-step".into(),
        review_mode: None,
        subject: subject.to_owned(),
        person: person.into(),
        conversation: match ask
            .as_ref()
            .and_then(|ask| ask.actor.as_deref())
            .and_then(agent)
        {
            Some(actor) => Some(actor),
            None => conversation_agent(connection, None, Some(&view.run))?,
        },
        requester_id: ask.and_then(|ask| ask.actor),
        launch_id: None,
        variant_id: None,
        message_id: None, prompt_call: None, answers: Vec::new(),
        title: view
            .title
            .unwrap_or_else(|| "A step needs your response".into()),
        detail: view.goals.join("\n"),
        mission: None,
        mission_run: Some(view.run),
        step: Some(subject.to_owned()),
        targets: vec![subject.to_owned()],
        requested_at_unix_ms: since,
        actions: vec![attention_action(
            label,
            &[
                "st",
                "work",
                "done",
                subject,
                "--as",
                person,
                response[0],
                response[1],
            ],
        )],
        request,
    }))
}

/// `subject` when it names an agent: only an agent has a conversation an item can belong to.
pub fn agent(subject: &str) -> Option<String> {
    subject.starts_with("agent/").then(|| subject.to_owned())
}

/// The agent whose conversation an item about `step` or `run` belongs to: the agent assigned to
/// the step, else the agent that requested its run, else none. A person's run has no agent.
pub fn conversation_agent(
    connection: &Connection,
    step: Option<&str>,
    run: Option<&str>,
) -> Result<Option<String>> {
    let mut run = run.map(str::to_owned);
    if let Some(step) = step
        && let Some(view) = person_work::step(connection, step)?
    {
        if let Some(agent) = view.assigned_to.as_deref().and_then(agent) {
            return Ok(Some(agent));
        }
        run.get_or_insert(view.run);
    }
    let Some(run) = run else { return Ok(None) };
    Ok(
        mission_run_header_tx(connection, run.strip_prefix("mission-run/").unwrap_or(&run))
            .optional()?
            .and_then(|header| agent(&header.requester)),
    )
}

/// The idempotency key of the record that a native prompt opened by `observation` is gone.
pub fn native_prompt_gone_key(observation: &str) -> String {
    format!("native-prompt-gone:{observation}")
}

pub fn native_prompt_gone_operation(observation: &str) -> String {
    smallclaims::store::operation_id_for_key(&native_prompt_gone_key(observation))
}

/// The idempotency key of the person's answer to the native prompt `observation` opened.
pub fn native_prompt_answer_key(observation: &str) -> String {
    format!("native-prompt-answer:{observation}")
}

/// Decode the shared diagnostic payload for both recent and durable answer lookups.
pub fn native_prompt_answer_from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Option<String>> {
    let status = row.get::<_, Option<String>>(offset)?;
    // A question's answer is its choices, which the reason holds.
    match status.as_deref() {
        Some("answer") => row.get(offset + 1),
        _ => Ok(status),
    }
}

/// Fold the native prompt's terminal-only release into the shared harness view.
/// Driver heartbeats can restate the old blocked file after Esc/No; their
/// unchanged episode must not resurrect a resolved hold in sender status/cards.
pub fn resolve_native_harness_at(
    connection: &Connection,
    subject: &str,
    index: u64,
    view: &mut crate::model::CurrentHarnessView,
) -> Result<()> {
    if view.driver.as_deref() != Some("claude")
        || view.blocked_on.as_deref() != Some("human")
        || !matches!(view.ask.as_deref(), Some("permission" | "question" | "review"))
    {
        return Ok(());
    }
    let Some(claim) = claim_by_id_tx(connection, &view.claim)? else { return Ok(()); };
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    let Some((ownership, transition)) = fields["ownership_sequence"].as_u64()
        .zip(fields["transition_sequence"].as_u64()) else { return Ok(()); };
    // A Smalltalk answer is a request for the permission hook, not proof that
    // Claude received it. The hook may already have exhausted its polling wait.
    if let Some((NativePromptState::Gone, at)) = native_prompt_sequence_resolution(connection, subject, ownership, transition, index)? {
        view.state = "idle".into();
        view.blocked_on = None;
        view.ask = None;
        view.reason = None;
        view.since_unix_ms = at;
        view.observed_at_unix_ms = view.observed_at_unix_ms.max(at);
    }
    Ok(())
}

/// An episode's release stays durable even after more than sixteen heartbeats.
/// Ownership/transition sequences identify that exact driver episode, so older
/// answers cannot release a successor prompt, even with the same tool/reason.
pub fn native_prompt_sequence_resolution(
    connection: &Connection,
    subject: &str,
    ownership: u64,
    transition: u64,
    index: u64,
) -> Result<Option<(NativePromptState, u128)>> {
    let mut observations = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7a8ee6803462 owner=st3_store::store::attention_snapshot TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT claims.id FROM claims INDEXED BY claims_harness_episode_index
         JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='harness.observed'
           AND claims.store_index<=?2
           AND json_extract(claims.body,'$.fields.ownership_sequence')=?3
           AND json_extract(claims.body,'$.fields.transition_sequence')=?4
         ORDER BY CANONICAL_DESC(claims)",
    ))?;
    // Native closure is terminal for this episode. A delayed answer request
    // cannot supersede Gone, even when it arrives later in canonical order.
    // Operation identity distinguishes closure from an answer: question choices
    // are answer payloads too, not just the permission words "allow" and "deny".
    let mut resolutions = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h41335a012b43 owner=st3_store::store::attention_snapshot TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT json_extract(body, '$._operation.id')=?1,
           json_extract(body, '$.fields.status'), json_extract(body, '$.fields.reason'),
           accepted_at_unix_ms
         FROM claims INDEXED BY claims_operation_index
         WHERE json_extract(body, '$._operation.id') IS NOT NULL
           AND json_extract(body, '$._operation.id') IN (?1,?2)
           AND store_index<=?3
         ORDER BY (json_extract(body, '$._operation.id')=?1) DESC,
           CANONICAL_DESC(claims) LIMIT 1",
    ))?;
    let mut requested_answer = None;
    for claim in observations.query_map(params![subject, index, ownership, transition], |row| row.get::<_, String>(0))? {
        smallclaims::read_budget::check()?;
        let claim = claim?;
        let gone = native_prompt_gone_operation(&claim);
        let answer = smallclaims::store::operation_id_for_key(&native_prompt_answer_key(&claim));
        let resolved: Option<(bool, Option<String>, String)> = resolutions
            .query_row(/* sqlite-large-value: allow-for-now lv-hf7276efc8832 owner=st3_store::store::attention_snapshot TODO=trace-statement-and-value-representation */ params![gone, answer, index], |row| {
                Ok((row.get(0)?, native_prompt_answer_from_row(row, 1)?, row.get(3)?))
            })
            .optional()?;
        if let Some((gone, answer, at)) = resolved {
            let state = match (gone, answer) {
                (false, Some(answer)) => NativePromptState::Answered { answer },
                _ => NativePromptState::Gone,
            };
            let resolution = (state, at.parse()?);
            if matches!(resolution.0, NativePromptState::Gone) {
                return Ok(Some(resolution));
            }
            requested_answer.get_or_insert(resolution);
        }
    }
    Ok(requested_answer)
}

/// What a waiting prompt hook needs to know: whether the seat's prompt is still open, answered
/// (and how), or gone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum NativePromptState {
    /// Not yet published, or still waiting on a person.
    Open,
    /// The person recorded an answer for a live permission hook to apply.
    /// This is not proof the hook received it or the native dialog closed.
    Answered { answer: String },
    /// Answered in the terminal, replaced or ended: nothing will arrive.
    Gone,
}

#[cfg(test)]
mod person_steps_plan_tests {
    

}

#[cfg(test)]
mod native_prompt_tests {
    

}

#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_snapshot_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::attention_snapshot_tests_shared_seats::step_back_in_time;

    

}
