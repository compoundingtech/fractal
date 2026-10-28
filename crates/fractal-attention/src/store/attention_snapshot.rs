//! The part of `store::attention_snapshot` that moved up from fractal-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::LoopSpec;
pub use fractal_seats::store::attention_snapshot::*;
pub use fractal_seats::store::attention_snapshot::{CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, agent, conversation_agent, native_prompt_gone_key, person_attention_item, plan_approval, recorded, resolve_native_harness_at, screen_prompt_delivered_key, screen_prompt_gone_key};
pub use fractal_seats::store::attention_snapshot::{screen_prompt_undelivered_key};

pub trait AttentionSnapshotStore {
    /// The mission context of a person ask a mission step made: its mission, and the step that
    /// waits on the answer with its goal. `None` for a standalone ask or an update, which belong
    /// to no mission of their own.
    fn person_ask_context(&self, subject: &str) -> Result<Option<Value>>;
    /// Whether `agent` is declared and not stopped or retired: the same test that lets an agent
    /// own a fault.
    fn agent_is_live(&self, agent: &str) -> Result<bool>;
    fn record_runtime_failure(
            &self,
            episode: &str,
            input: &AttentionRequest,
            condition: &str,
        ) -> Result<AttentionRequestView, St3Error>;
    fn pending_operational_failures(
            &self,
            actor: &str,
            origin: &str,
        ) -> Result<Vec<AttentionRequestView>>;
    fn recover_operational_failure(
            &self,
            episode: &str,
            reason: &str,
            key: &str,
        ) -> Result<AttentionRequestView, St3Error>;
    /// Whether a record with the idempotency key `key` exists.
    fn operation_recorded(&self, key: &str) -> Result<bool>;
    /// The person's answer to the screen prompt `claim` recorded, and whether st pressed its keys.
    fn screen_prompt_answer(&self, claim: &str) -> Result<(Option<String>, bool)>;
    /// The person's answer to the native prompt or denial `episode` recorded, if any.
    fn prompt_answer(&self, episode: &str) -> Result<PromptAnswer>;
}

impl AttentionSnapshotStore for Store {

    fn person_ask_context(&self, subject: &str) -> Result<Option<Value>> {
        let connection = self.readers.get();
        let Some(ask) = person_work::request(&connection, subject)? else {
            return Ok(None);
        };
        let fields = &ask.body["fields"];
        let (Some(origin), Some(view)) = (
            fields["origin_step"].as_str(),
            person_work::step(&connection, subject)?,
        ) else {
            return Ok(None);
        };
        let Some(blocked) = person_work::step(&connection, origin)? else {
            return Ok(None);
        };
        // Runs are stored without their `mission-run/` prefix, and missions without `mission/`.
        let mission: Option<String> = connection
            .query_row(
                "SELECT mission_id FROM mission_runs WHERE id=?1",
                [view.run.strip_prefix("mission-run/").unwrap_or(&view.run)],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|id| {
                if id.starts_with("mission/") {
                    id
                } else {
                    format!("mission/{id}")
                }
            });
        Ok(Some(json!({
            "mission_id": mission,
            "blocked": {
                "step_run_id": origin,
                "step": blocked.step,
                "goal": blocked.goals.join("\n"),
                "attempt": blocked.attempt,
            },
        })))
    }

    fn agent_is_live(&self, agent: &str) -> Result<bool> {
        let connection = self.readers.get();
        Ok(agent.starts_with("agent/")
            && current_desired_row(&connection, agent)?.is_some_and(|row| row.kind == "agent")
            && person_work::declaration_live(&connection, agent)?)
    }

    fn record_runtime_failure(
            &self,
            episode: &str,
            input: &AttentionRequest,
            condition: &str,
        ) -> Result<AttentionRequestView, St3Error> {
        self.record_failure(episode, input, condition)
    }

    fn pending_operational_failures(
            &self,
            actor: &str,
            origin: &str,
        ) -> Result<Vec<AttentionRequestView>> {
        let connection = self.readers.get();
        let mut failures = Vec::new();
        for failure in self.operational_failures()? {
            if failure.status == "pending"
                && failure.actor == actor
                && connection.query_row(
                    "SELECT origin=?2 FROM claims WHERE id=?1",
                    params![failure.request, origin],
                    |row| row.get::<_, bool>(0),
                )?
            {
                failures.push(failure);
            }
        }
        Ok(failures)
    }

    fn recover_operational_failure(
            &self,
            episode: &str,
            reason: &str,
            key: &str,
        ) -> Result<AttentionRequestView, St3Error> {
        let current = self
            .operational_failure(episode)
            .map_err(internal)?
            .ok_or_else(|| {
                St3Error::new("missing-operational-failure", "the episode does not exist")
            })?;
        let source = current
            .targets
            .first()
            .ok_or_else(|| St3Error::new("missing-failure-source", "the episode has no source"))?;
        self.append_claim(&ClaimInput {
            subject: source.clone(),
            kind: "operational.recovered".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("failure".into(), json!(current.request)),
                ("reason".into(), json!(reason)),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key.into()),
        })?;
        self.operational_failure(episode)
            .map_err(internal)?
            .ok_or_else(|| St3Error::new("internal", "the recovery was not recorded"))
    }

    fn operation_recorded(&self, key: &str) -> Result<bool> {
        let connection = self.readers.get();
        recorded(&connection, key)
    }

    fn screen_prompt_answer(&self, claim: &str) -> Result<(Option<String>, bool)> {
        let connection = self.readers.get();
        Ok((
            native_prompt_answer(&connection, claim)?,
            recorded(&connection, &screen_prompt_delivered_key(claim))?,
        ))
    }

    fn prompt_answer(&self, episode: &str) -> Result<PromptAnswer> {
        let connection = self.readers.get();
        Ok(PromptAnswer {
            answer: native_prompt_answer(&connection, episode)?,
        })
    }
}

