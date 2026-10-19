//! The part of `store::attention` that moved up from st3-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_seats::store::attention::*;

/// Why `owner` has no human review waiting, in words for the person who tried to answer it:
/// its newest request was answered (how, and by whom), no longer matches the owner, or st never
/// asked one.
fn human_review_refusal_tx(connection: &Connection, owner: &str) -> Result<String> {
    let request = connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hf97b8354cf37 owner=st3_attention::store::attention TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT request.id, request.store_index, request.batch_id, request.subject,
                        request.kind, request.origin, request.actor, request.body,
                        request.predecessors, request.accepted_at_unix_ms
                 FROM claims request
                 WHERE request.kind='gate.requested'
                   AND json_extract(request.body, '$.fields.owner')=?1
                   AND json_extract(request.body, '$.fields.reviewer') IS NOT NULL
                 ORDER BY CANONICAL_DESC(request) LIMIT 1",
            ),
            [owner],
            claim_from_row,
        )
        .optional()?;
    let Some(request) = request else {
        return Ok("st has asked no person to review it".into());
    };
    if let Some(answer) = human_review_answer_tx(connection, &request.id)? {
        let fields = answer.body.get("fields").unwrap_or(&answer.body);
        let decision = fields.get("decision").and_then(Value::as_str).unwrap_or(
            match fields.get("verdict").and_then(Value::as_str) {
                Some("pass") => "approved",
                Some("feedback") => "changes-requested",
                _ => "rejected",
            },
        );
        return Ok(format!(
            "it was already answered: {decision} by {}",
            answer.actor.as_deref().unwrap_or("its reviewer")
        ));
    }
    Ok(match human_review_currency(connection, request)? {
        Err(why) => format!("its review is no longer current: {why}"),
        Ok(review) => format!("its review waits on {}", review.reviewer),
    })
}

pub trait AttentionStore {
    /// Run one refresh, answering the requests made before it began only if it publishes.
    fn answer_agent_roster_requests<T>(&self, refresh: impl FnOnce() -> Result<T>) -> Result<T>;
    /// Current generation steps for all runs in two SQL scans. Collection reads
    /// never hydrate each run and each step through their detail endpoints.
    fn mission_run_summaries(&self) -> Result<Vec<MissionRunView>>;
    fn mission_run_summaries_for_missions(
            &self,
            ids: &[String],
        ) -> Result<Vec<MissionRunView>>;
    fn mission_run_summaries_for_ids(&self, ids: Option<&[String]>) -> Result<Vec<MissionRunView>>;
    fn discard_desired_owned_by(&self, owner_run: &str) -> Result<usize>;
    /// Why `owner` has no pending human review: answered (how, by whom), stale (which of
    /// generation, revision, definition, attempt or state moved on), or never asked.
    fn human_review_refusal(&self, owner: &str) -> Result<String>;
    fn resolve_attention_automatically(
            &self,
            subject: &str,
            reason: &str,
            idempotency_key: &str,
        ) -> Result<AttentionRequestView, St3Error>;
    fn attention_requests(
            &self,
            person: Option<&str>,
            include_resolved: bool,
        ) -> Result<Vec<AttentionRequestView>>;
    /// Pending requests that `actor` raised from `origin`, so that host can close its own alerts.
    fn pending_attention_requests_raised_by(
            &self,
            actor: &str,
            origin: &str,
        ) -> Result<Vec<AttentionRequestView>>;
    /// The runs that wait on a person, with the open human gates among them: each gate's
    /// mission, the step it holds when it holds one, and the person it waits on.
    fn human_attention(&self) -> Result<HumanAttention>;
}

impl AttentionStore for Store {

    fn answer_agent_roster_requests<T>(&self, refresh: impl FnOnce() -> Result<T>) -> Result<T> {
        self.smalltalk.agent_roster_attempt.fetch_or(1, std::sync::atomic::Ordering::AcqRel);
        struct Attempt<'a>(&'a SmalltalkRuntime, bool);
        impl Drop for Attempt<'_> {
            fn drop(&mut self) {
                self.0.agent_roster_attempt.store(if self.1 { 0 } else { 2 }, std::sync::atomic::Ordering::Release);
                // Failure (including an unwind) must also wake readers waiting for publication.
                self.0.agent_roster_published.send_modify(|_| {});
            }
        }
        let mut attempt = Attempt(&self.smalltalk, false);
        let requested = &self.smalltalk.agent_roster_requested_at;
        let pending = requested.swap(0, std::sync::atomic::Ordering::AcqRel);
        let result = refresh();
        attempt.1 = result.is_ok();
        if result.is_err() && pending != 0 {
            // Still unanswered: keep the oldest request time unless a newer request is waiting.
            let _ = requested.compare_exchange(0, pending, std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed);
        }
        if result.is_ok() {
            self.smalltalk.agent_roster_overdue_warned.store(false, std::sync::atomic::Ordering::Release);
        }
        result
    }

    fn mission_run_summaries(&self) -> Result<Vec<MissionRunView>> {
        self.mission_run_summaries_for_ids(None)
    }

    fn mission_run_summaries_for_missions(
            &self,
            ids: &[String],
        ) -> Result<Vec<MissionRunView>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.mission_run_summaries_for_ids(Some(ids))
    }

    fn mission_run_summaries_for_ids(&self, ids: Option<&[String]>) -> Result<Vec<MissionRunView>> {
        let connection = self.readers.get();
        let placeholders = ids.map(|ids| vec!["?"; ids.len()].join(","));
        let sql = placeholders.as_ref().map(|placeholders| MISSION_RUN_HEADER_SELECT.replace(
            "ORDER BY mission_runs.created_at_unix_ms, mission_runs.id",
            &format!("WHERE mission_runs.mission_id IN ({placeholders}) ORDER BY mission_runs.created_at_unix_ms, mission_runs.id"),
        )).unwrap_or_else(|| MISSION_RUN_HEADER_SELECT.to_owned());
        let mut headers = connection
            .prepare(/* sqlite-large-value: allow-for-now lv-h66a0bce27842 owner=st3_attention::store::attention TODO=trace-statement-and-value-representation */ &sql)?
            .query_map(
                rusqlite::params_from_iter(
                    ids.into_iter()
                        .flatten()
                        .map(|id| id.trim_start_matches("mission/")),
                ),
                mission_run_header_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let by_generation = headers
            .iter()
            .enumerate()
            .map(|(index, run)| (run.generation.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let steps_sql = format!(
            "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                    lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints,
                    generation_id
             FROM step_runs {} ORDER BY created_at_unix_ms, step_path",
            placeholders.as_ref().map(|p| format!("WHERE generation_id IN (SELECT current_generation_id FROM mission_runs WHERE mission_id IN ({p}))")).unwrap_or_default(),
        );
        let mut steps = connection.prepare(/* sqlite-large-value: allow-for-now lv-h8a8538a99cae owner=st3_attention::store::attention TODO=trace-statement-and-value-representation */ &steps_sql)?;
        for row in steps.query_map(
            rusqlite::params_from_iter(
                ids.into_iter()
                    .flatten()
                    .map(|id| id.trim_start_matches("mission/")),
            ),
            |row| {
                let step = step_run_from_row(row)?;
                let generation: String = row.get(21)?;
                Ok((generation, step))
            },
        )? {
            let (generation, step) = row?;
            if let Some(index) = by_generation.get(&format!("run-generation/{generation}")) {
                headers[*index].steps.push(step);
            }
        }
        Ok(headers)
    }

    fn discard_desired_owned_by(&self, owner_run: &str) -> Result<usize> {
        let connection = self.connection.write();
        connection
            .execute(/* sqlite-large-value: allow-for-now lv-h9d47d23a105a owner=st3_attention::store::attention TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM desired WHERE owner_run=?1 AND subject NOT LIKE 'resource/uri/%'", [owner_run])
            .map_err(Into::into)
    }

    fn human_review_refusal(&self, owner: &str) -> Result<String> {
        let connection = self.readers.get();
        human_review_refusal_tx(&connection, owner)
    }

    fn resolve_attention_automatically(
            &self,
            subject: &str,
            reason: &str,
            idempotency_key: &str,
        ) -> Result<AttentionRequestView, St3Error> {
        let subject = if subject.starts_with("attention/") {
            subject.to_owned()
        } else {
            format!("attention/{subject}")
        };
        let current = self
            .attention_request(&subject)
            .map_err(internal)?
            .ok_or_else(|| {
                St3Error::new(
                    "missing-attention-request",
                    format!("attention request `{subject}` does not exist"),
                )
            })?;
        self.append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: "attention.resolved".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("request".into(), Value::String(current.request)),
                ("outcome".into(), Value::String("resolved".into())),
                ("reason".into(), Value::String(reason.into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(idempotency_key.into()),
        })?;
        self.attention_request(&subject)
            .map_err(internal)?
            .ok_or_else(|| St3Error::new("internal", "the attention resolution was not stored"))
    }

    fn attention_requests(
            &self,
            person: Option<&str>,
            include_resolved: bool,
        ) -> Result<Vec<AttentionRequestView>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hb13c5fa65f54 owner=st3_attention::store::attention TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT DISTINCT subject FROM claims
             WHERE kind='attention.requested'
               AND (?1 IS NULL OR json_extract(body, '$.fields.reviewer')=?1)
             ORDER BY CANONICAL_ASC(claims)",
        ))?;
        let subjects = statement
            .query_map([person], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let requests = subjects
            .into_iter()
            .filter_map(|subject| attention_request_view_tx(&connection, &subject).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(requests
            .into_iter()
            .filter(|request| include_resolved || request.status == "pending")
            .collect())
    }

    fn pending_attention_requests_raised_by(
            &self,
            actor: &str,
            origin: &str,
        ) -> Result<Vec<AttentionRequestView>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf9332f2649e7 owner=st3_attention::store::attention TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT request.subject FROM claims request
             WHERE request.kind='attention.requested'
               AND request.actor=?1
               AND request.origin=?2
               AND NOT EXISTS (
                 SELECT 1 FROM claims resolution
                 WHERE resolution.subject=request.subject
                   AND resolution.kind='attention.resolved'
               )
             ORDER BY CANONICAL_ASC(request)",
        ))?;
        let subjects = statement
            .query_map([actor, origin], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        smallclaims::touched::note_read(|| "kind:attention.requested".to_owned());
        for subject in &subjects {
            smallclaims::touched::note_read(|| subject.clone());
        }
        subjects
            .iter()
            .filter_map(|subject| attention_request_view_tx(&connection, subject).transpose())
            .collect()
    }

    fn human_attention(&self) -> Result<HumanAttention> {
        self.human_attention_at(now_ms())
    }
}

