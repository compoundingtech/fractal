//! Observations from the managed pi-family native channel.
//!
//! The CLI owns stdio, mailbox delivery and binary replacement. This module records validated
//! native-session observations and shares the measured state and context normalizers.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::{harness_context, harness_state};

/// Record the managed channel's observations in its st-owned outbox.
pub struct EventObserver {
    state: harness_state::Writer,
    context: harness_context::Writer,
    timeline: crate::harness_timeline::Writer,
    last_heartbeat: std::time::Instant,
    agent_dir: std::path::PathBuf,
    session_dir: std::path::PathBuf,
    identity: String,
    ownership_seq: u64,
    resume_generation: Option<crate::residency::Generation>,
    expected_native_session: Option<String>,
    owner: String,
    runtime: String,
    driver: &'static str,
    native_session: Option<String>,
}
impl EventObserver {
    pub fn new(
        agent_dir: &Path,
        session_dir: &Path,
        identity: &str,
        driver: &'static str,
        session: &str,
        seq: u64,
        runtime_id: &str,
    ) -> Result<Self> {
        let harness = match driver {
            "pi" => harness_context::Harness::Pi,
            "omp" => harness_context::Harness::Omp,
            _ => anyhow::bail!("unsupported managed channel"),
        };
        let resume_generation = if driver == "omp" {
            crate::contracts::env(crate::omp_session::CHANNEL_RESUME_GENERATION)
                .map(|value| value.parse::<u64>().map(crate::residency::Generation)
                    .context("OMP channel resume generation is invalid"))
                .transpose()?
        } else { None };
        let expected_native_session = if driver == "omp" {
            crate::contracts::env(crate::omp_session::CHANNEL_EXPECTED_NATIVE_SESSION)
                .filter(|value| !value.is_empty())
        } else { None };
        anyhow::ensure!(
            resume_generation.is_some() == expected_native_session.is_some(),
            "OMP channel has an incomplete mandatory resume fence"
        );
        let mut state =
            harness_state::Writer::new(agent_dir, identity, driver, Some(runtime_id.into()))
                .with_ownership(session, seq);
        state.interrupt();
        Ok(Self {
            state,
            context: harness_context::Writer::new_paths(agent_dir, identity, harness)?
                .with_session(session),
            timeline: crate::harness_timeline::Writer::new(agent_dir, driver, session),
            last_heartbeat: std::time::Instant::now(),
            agent_dir: agent_dir.into(),
            session_dir: session_dir.into(),
            identity: identity.into(),
            ownership_seq: seq,
            resume_generation,
            expected_native_session,
            owner: session.into(),
            runtime: runtime_id.into(),
            driver,
            native_session: None,
        })
    }
    /// Restore an authenticated native ID; durable confirmation alone consumes the cold fence.
    pub fn with_native_session(mut self, native_session: Option<String>) -> Result<Self> {
        if self.driver == "omp" && native_session.is_some()
            && harness_state::with_current_ownership(&self.agent_dir, &self.owner, self.ownership_seq, || {
                crate::omp_session::has_confirmed_channel_binding(
                    &self.session_dir, &self.identity, &self.runtime, &self.owner)
            })?
        {
            self.resume_generation = None;
            self.expected_native_session = None;
        }
        self.native_session = native_session;
        Ok(self)
    }
    pub fn observe(&mut self, frame: &Value) -> Result<()> {
        if matches!(frame["type"].as_str(), Some("session" | "ready")) {
            let native = frame["sessionId"].as_str().filter(|id| !id.is_empty());
            if self.driver == "omp" {
                let native = native.context("OMP channel has no native session id")?;
                if frame["type"] == "session" {
                    harness_state::with_current_ownership(&self.agent_dir, &self.owner, self.ownership_seq, || {
                        crate::omp_session::record_channel_binding(&self.session_dir, &self.identity,
                            &self.runtime, &self.owner, native, self.resume_generation,
                            self.expected_native_session.as_deref())
                    })?;
                } else {
                    crate::omp_session::confirm_channel_binding(&self.session_dir, &self.agent_dir,
                        &self.identity, &self.runtime, &self.owner, self.ownership_seq, native,
                        self.resume_generation)?;
                    self.resume_generation = None;
                    self.expected_native_session = None;
                }
            }
            self.native_session = native.map(str::to_owned);
        }
        if matches!(frame["type"].as_str(), Some("question.open" | "question.closed")) {
            if self.driver != "omp" || !crate::omp_questions::enabled() {
                return Ok(());
            }
            let observation: crate::omp_questions::Observation = serde_json::from_value(frame.clone())?;
            observation.validate()?;
            anyhow::ensure!(self.native_session.as_deref() == Some(observation.session()),
                "question observation belongs to another native session");
            return harness_state::with_current_ownership(
                &self.agent_dir, &self.owner, self.ownership_seq, || {
                    anyhow::ensure!(crate::omp_session::bound_native_session(
                        &self.session_dir, &self.identity, &self.runtime, &self.owner,
                    )?.as_deref() == Some(observation.session()),
                        "question observation has no confirmed native session");
                    crate::harness_events::write_question(&self.agent_dir, &self.owner, &observation)
                },
            );
        }
        if let Some(inventory) = crate::harness_inventory::observation(
            frame, self.driver, self.native_session.as_deref(), &self.runtime,
        )? {
            let mut payload = serde_json::to_value(inventory)?;
            payload["incarnation"] = Value::String(self.owner.clone());
            crate::harness_events::write_snapshot(
                &self.agent_dir, "harness-inventory", &serde_json::to_vec(&payload)?,
            )?;
        }
        // A subagent's runs go to the ledger the seat's driver records on the seat. Only a bound
        // native session vouches for them.
        if frame["type"] == "subagent" {
            return match self.native_session.as_deref() {
                Some(parent) if self.driver == "omp" => {
                    crate::subagents::observe_omp(&self.agent_dir, frame, parent)
                }
                _ => Ok(()),
            };
        }
        if let Some(fields) = todo_observation(
            frame, self.driver, self.native_session.as_deref(), &self.runtime,
        ) {
            let mut payload = serde_json::to_value(fields)?;
            payload["incarnation"] = Value::String(self.owner.clone());
            crate::harness_events::write_snapshot(
                &self.agent_dir, "harness-todo", &serde_json::to_vec(&payload)?,
            )?;
        }
        if let Some(observation) = state_observation(frame) {
            self.state.observe(observation)?;
        }
        if let Some(context) = context_frame(frame) {
            write_context(&mut self.context, context)?;
        }
        crate::harness_timeline::observe_channel_frame(&mut self.timeline, frame)
    }
    pub fn heartbeat(&mut self) -> Result<()> {
        if self.last_heartbeat.elapsed() >= harness_state::HARNESS_STATE_REFRESH {
            self.state.heartbeat()?;
            self.last_heartbeat = std::time::Instant::now();
        }
        Ok(())
    }
}

/// Normalize only structured snapshots from the currently bound native session. Transport
/// supplied identity is never accepted as claim provenance.
pub fn todo_observation(
    frame: &Value,
    driver: &str,
    native_session: Option<&str>,
    runtime: &str,
) -> Option<std::collections::BTreeMap<String, Value>> {
    if driver != "omp" || frame["type"] != "todo"
        || native_session.is_none() || frame["session"].as_str() != native_session
    {
        return None;
    }
    let mut fields = std::collections::BTreeMap::from([
        ("harness".into(), Value::String(driver.into())),
        ("session_id".into(), Value::String(native_session?.into())),
        ("incarnation_id".into(), Value::String(runtime.into())),
    ]);
    for key in ["observed_at", "source_op", "phases", "totals", "truncated"] {
        fields.insert(key.into(), frame.get(key)?.clone());
    }
    fractal_schema::registry().validate_claim(
        "agent/channel", "harness.todo.observed", &fields,
    ).ok()?;
    Some(fields)
}

/// The observed-state frame the shipped extension emits on the harness's own turn boundaries.
/// Only positively recognized words become observations: an unrecognized state word is dropped
/// like any other unknown frame, so a newer asset cannot make this channel record something it
/// cannot vouch for. pi offers no waiting-on-a-human signal, so pi frames never carry
/// `blockedOn`; the omp extension does (`tool_approval_requested`/`_resolved`), and its optional
/// axes are parsed here for both channels — a frame without them decodes exactly as before.
fn state_observation(frame: &Value) -> Option<harness_state::Observation> {
    if frame.get("type").and_then(Value::as_str) != Some("state") {
        return None;
    }
    let state = match frame.get("state").and_then(Value::as_str)? {
        "active" => harness_state::Activity::Active,
        "idle" => harness_state::Activity::Idle,
        _ => return None,
    };
    let blocked_on = match frame.get("blockedOn").and_then(Value::as_str) {
        Some("human") => harness_state::BlockedOn::Human,
        _ => harness_state::BlockedOn::None,
    };
    let mut observation =
        harness_state::Observation::new(state, blocked_on, harness_state::InputBuffer::Unknown);
    observation.background_jobs = frame.get("backgroundJobs").and_then(Value::as_u64);
    if blocked_on == harness_state::BlockedOn::Human
        && let Some(ask) = frame.get("ask").and_then(Value::as_str)
    {
        observation = observation.with_ask(parse_ask(ask));
    }
    if let Some(reason) = frame.get("reason").and_then(Value::as_str) {
        observation = observation.with_reason(reason);
    }
    Some(observation)
}

/// One `type: "context"` frame as the shipped extension emits it: a reading, a compaction edge, or
/// both. Both halves are optional, and a frame carrying neither is not a frame — it is dropped
/// like any other unrecognized one.
type ContextFrame = (
    Option<harness_context::Reading>,
    Option<harness_context::Compaction>,
);

/// Decode a context frame (HC-R02, HC-R03, HC-R12).
///
/// The withholding discipline lives here as much as in the asset: a number this decoder cannot
/// read as a finite number is `None`, never zero and never the previous value. `usedPercent` is
/// taken exactly as the harness published it — pi and omp report a float that runs well above 100
/// on an overrun (585.6% measured), and st2 neither clamps it nor computes one of its own from a
/// window it would have had to guess at.
fn context_frame(frame: &Value) -> Option<ContextFrame> {
    if frame.get("type").and_then(Value::as_str) != Some("context") {
        return None;
    }
    let reading = frame.get("reading").and_then(Value::as_object).map(|body| {
        let tokens = |key: &str| body.get(key).and_then(token_count);
        harness_context::Reading {
            used_tokens: tokens("usedTokens"),
            window_tokens: tokens("windowTokens"),
            used_percent: body.get("usedPercent").and_then(Value::as_f64),
            model: body
                .get("model")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            cost_usd: body.get("costUsd").and_then(Value::as_f64),
            // pi and omp both withhold this in v1: obtaining it means summing every message's
            // usage, which is a producer-side accumulator whose correctness depends on having seen
            // every message. tokenlens owns lifetime accounting, and a half-observed sum would be
            // a worse answer than none.
            session_total_tokens: None,
            // Neither harness reports account-scoped rate limits.
            rate_limits: harness_context::RateLimits::default(),
            account: None,
            plan: None,
        }
    });
    let compaction = frame
        .get("compaction")
        .and_then(Value::as_object)
        .map(|body| {
            let edge = harness_context::Compaction::new(compaction_trigger(
                body.get("trigger").and_then(Value::as_str),
            ));
            // The harness's own session store answers the count for both of these harnesses, so
            // the counter is durable across restarts. A frame that could not read it leaves the
            // count absent, and st2 falls back to incrementing its own — a weaker,
            // incarnation-scoped answer rather than a wrong one.
            match body.get("count").and_then(Value::as_u64) {
                Some(count) => edge.with_count(count),
                None => edge,
            }
        });
    (reading.is_some() || compaction.is_some()).then_some((reading, compaction))
}

/// A token count off the wire, tolerating a fractional one.
///
/// Both harnesses round today — pi's fallback estimator is `Math.ceil(chars / 4)` and every
/// measured reading was integral — so this is deliberately not a decode of anything observed. It
/// exists because the failure if one ever stops rounding is silent in the worst direction: a plain
/// integer parse would return `None` for `1234.75`, and the producer would WITHHOLD a reading the
/// harness actually had. Withholding is reserved for a harness saying it does not know (HC-R03);
/// spending it on a JSON number shape would make the record lie about which of those happened. The
/// percent leg already parses as a float because pi and omp genuinely emit one there; this extends
/// the same tolerance to the operands rather than leaving them stricter for no measured reason.
///
/// A negative or non-finite value is not a token count and is withheld.
fn token_count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && *number >= 0.0)
            .map(|number| number.round() as u64)
    })
}

/// The trigger word, over the record's closed vocabulary and additive-tolerant on read: a word
/// this version does not recognize — and an edge that carries none at all, which is omp's case and
/// three of the five harnesses' — decodes as `unknown`, never as a definite trigger.
fn compaction_trigger(word: Option<&str>) -> harness_context::CompactionTrigger {
    use harness_context::CompactionTrigger as Trigger;
    match word {
        Some("manual") => Trigger::Manual,
        Some("auto") => Trigger::Auto,
        Some("threshold") => Trigger::Threshold,
        Some("overflow") => Trigger::Overflow,
        Some("idle") => Trigger::Idle,
        _ => Trigger::Unknown,
    }
}

/// Land one context frame.
///
/// A frame carrying both halves lands as ONE write, and that is load-bearing rather than an
/// optimization: a compaction edge always writes while a reading whose percent is withheld has no
/// bucket, so an edge written alone would publish the stale pre-compaction numbers beside it and
/// the null reading proving the window was emptied would not appear until the heartbeat came due.
/// pi hands us exactly that pair — measured inside its own `session_compact` handler,
/// `getContextUsage()` already reports `{tokens: null, percent: null}` there.
fn write_context(
    writer: &mut harness_context::Writer,
    (reading, compaction): ContextFrame,
) -> Result<bool> {
    match (reading, compaction) {
        (Some(reading), Some(compaction)) => writer.compacted_with(compaction, reading),
        (Some(reading), None) => writer.observe(reading),
        (None, Some(compaction)) => writer.compacted(compaction),
        (None, None) => Ok(false),
    }
}

/// The machine-readable ask word on a blocked frame. An unrecognized word decodes as unknown —
/// indeterminate, never silently reclassified — matching the record's own decode rule.
fn parse_ask(word: &str) -> harness_state::Ask {
    match word {
        "permission" => harness_state::Ask::Permission,
        "question" => harness_state::Ask::Question,
        "review" => harness_state::Ask::Review,
        _ => harness_state::Ask::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn todo_frame() -> Value {
        serde_json::json!({
            "type":"todo", "session":"native-a", "observed_at":"2026-10-03T15:00:00Z",
            "source_op":"block", "phases":[{"name":"Review","tasks":[
                {"content":"Review claim","status":"blocked","blocker":"Await review"}
            ]}], "totals":{"pending":0,"in_progress":0,"completed":0,"blocked":1},
            "truncated":false
        })
    }

    #[test]
    fn harness_todo_normalizer_requires_current_binding_and_valid_snapshot() {
        let frame = todo_frame();
        assert!(todo_observation(&frame, "omp", None, "runtime-a").is_none());
        assert!(todo_observation(&frame, "omp", Some("native-b"), "runtime-a").is_none());
        assert!(todo_observation(&frame, "pi", Some("native-a"), "runtime-a").is_none());
        let fields = todo_observation(&frame, "omp", Some("native-a"), "runtime-a").unwrap();
        assert_eq!(fields["incarnation_id"], "runtime-a");
        assert_eq!(fields["phases"][0]["tasks"][0]["blocker"], "Await review");
        let mut malformed = frame;
        malformed["phases"][0]["tasks"][0]["status"] = "invented".into();
        assert!(todo_observation(&malformed, "omp", Some("native-a"), "runtime-a").is_none());
    }

    #[test]
    fn harness_todo_managed_observer_spools_and_fences_superseded_owner() {
        let root = tempfile::tempdir().unwrap();
        crate::harness_events::enable(root.path(), "runtime-a").unwrap();
        let seq = harness_state::claim(root.path(), "agent/example", "omp", "provider-a").unwrap();
        let cold_observer = || {
            let mut observer = EventObserver::new(
                root.path(), &root.path().join("sessions"), "agent/example", "omp", "provider-a", seq, "runtime-a",
            ).unwrap();
            observer.resume_generation = Some(crate::residency::Generation(2));
            observer.expected_native_session = Some("native-a".into());
            observer
        };
        let mut observer = cold_observer();
        assert!(observer.observe(&serde_json::json!({"type":"session","sessionId":"native-wrong"})).is_err());
        let mut incomplete = cold_observer();
        incomplete.resume_generation = None;
        assert!(incomplete.observe(&serde_json::json!({"type":"session","sessionId":"native-a"})).is_err());
        observer.observe(&serde_json::json!({"type":"session","sessionId":"native-a"})).unwrap();
        assert_eq!(crate::omp_session::bound_native_session(&root.path().join("sessions"),
            "agent/example", "runtime-a", "provider-a").unwrap(), None);
        observer = cold_observer().with_native_session(Some("native-a".into())).unwrap();
        assert!(observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-wrong"})).is_err());
        observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-a"})).unwrap();
        assert_eq!(crate::omp_session::bound_native_session(&root.path().join("sessions"),
            "agent/example", "runtime-a", "provider-a").unwrap().as_deref(), Some("native-a"));
        observer.observe(&todo_frame()).unwrap();
        let events = crate::harness_events::pending(root.path(), 100).unwrap();
        let event = events.iter().find(|event| event.kind == "harness-todo").unwrap();
        assert_eq!(event.runtime_incarnation, "runtime-a");
        assert_eq!(event.payload["session_id"], "native-a");
        let prepared = serde_json::json!({"fields":event.payload});
        assert_eq!(crate::harness_events::prepare_publication(
            root.path(), event.sequence, "harness.todo.observed:", &prepared,
        ).unwrap(), prepared);
        assert_eq!(crate::harness_events::prepare_publication(
            root.path(), event.sequence, "harness.todo.observed:", &serde_json::json!({"changed":true}),
        ).unwrap(), prepared);
        observer.observe(&serde_json::json!({"type":"session","sessionId":"native-b"})).unwrap();
        assert_eq!(crate::omp_session::bound_native_session(&root.path().join("sessions"),
            "agent/example", "runtime-a", "provider-a").unwrap(), None);
        assert!(observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-a"})).is_err());
        assert_eq!(crate::omp_session::bound_native_session(&root.path().join("sessions"),
            "agent/example", "runtime-a", "provider-a").unwrap(), None);
        observer = cold_observer().with_native_session(Some("native-b".into())).unwrap();
        observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-b"})).unwrap();
        assert_eq!(crate::omp_session::bound_native_session(&root.path().join("sessions"),
            "agent/example", "runtime-a", "provider-a").unwrap().as_deref(), Some("native-b"));
        crate::harness_events::enable(root.path(), "runtime-b").unwrap();
        harness_state::claim(root.path(), "agent/example", "omp", "provider-b").unwrap();
        assert!(cold_observer().with_native_session(Some("native-b".into())).is_err());
        assert!(observer.observe(&serde_json::json!({"type":"session","sessionId":"native-c"})).is_err());
        assert!(observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-b"})).is_err());
        let mut current_todo = todo_frame();
        current_todo["session"] = "native-b".into();
        assert!(observer.observe(&current_todo).is_err());
        assert_eq!(crate::harness_events::pending(root.path(), 100).unwrap()
            .iter().filter(|event| event.kind == "harness-todo").count(), 1);
    }

    #[test]
    fn omp_subagent_frames_reach_the_ledger_only_from_a_bound_session() {
        let root = tempfile::tempdir().unwrap();
        crate::harness_events::enable(root.path(), "runtime-a").unwrap();
        let seq = harness_state::claim(root.path(), "agent/example", "omp", "provider-a").unwrap();
        let mut observer = EventObserver::new(
            root.path(), &root.path().join("sessions"), "agent/example", "omp", "provider-a", seq, "runtime-a",
        ).unwrap();
        let start = serde_json::json!({"type":"subagent","event":"start","id":"0-Review","name":"task"});
        observer.observe(&start).unwrap();
        assert!(crate::subagents::read(root.path()).running.is_empty());
        observer.observe(&serde_json::json!({"type":"session","sessionId":"native-a"})).unwrap();
        observer.observe(&serde_json::json!({"type":"ready","sessionId":"native-a"})).unwrap();
        observer.observe(&start).unwrap();
        let ledger = crate::subagents::read(root.path());
        assert_eq!(ledger.session_id.as_deref(), Some("native-a"));
        assert_eq!(ledger.running["0-Review"].subagent_type.as_deref(), Some("task"));
        observer.observe(&serde_json::json!({"type":"subagent","event":"end","id":"0-Review"})).unwrap();
        let ledger = crate::subagents::read(root.path());
        assert!(ledger.running.is_empty());
        assert_eq!(ledger.ended[0].outcome, "completed");
    }

    #[test]
    fn omp_channel_binds_native_session_before_readiness() {
        let temp = tempfile::tempdir().unwrap();
        let agent_dir = temp.path().join("agent");
        let state = temp.path().join("state");
        crate::harness_events::enable(&agent_dir, "runtime-next").unwrap();
        let ownership_seq =
            harness_state::claim(&agent_dir, "h.worker", "omp", "runtime-next").unwrap();
        let mut observer = EventObserver::new(
            &agent_dir, &state, "h.worker", "omp", "runtime-next", ownership_seq, "runtime-next",
        ).unwrap();
        observer.resume_generation = Some(crate::residency::Generation(2));
        observer.expected_native_session = Some("session-exact".into());
        observer.observe(&json!({"type":"session","sessionId":"session-exact"})).unwrap();

        let binding: Value = serde_json::from_slice(
            &std::fs::read(state.join("binding.pending.json")).unwrap(),
        ).unwrap();
        assert_eq!(binding["nativeSessionId"], "session-exact");
        assert_eq!(binding["runtimeIncarnation"], "runtime-next");
        assert_eq!(binding["resumeGeneration"], 2);
        assert_eq!(binding["ready"], false);
        assert!(!state.join("binding.json").exists());

        observer.observe(&json!({"type":"ready","sessionId":"session-exact"})).unwrap();
        let binding: Value =
            serde_json::from_slice(&std::fs::read(state.join("binding.json")).unwrap()).unwrap();
        assert_eq!(binding["ready"], true);
        assert!(!state.join("binding.pending.json").exists());
    }

    #[test]
    fn omp_channel_refuses_a_different_resumed_native_session() {
        let temp = tempfile::tempdir().unwrap();
        let agent_dir = temp.path().join("agent");
        let state = temp.path().join("state");
        crate::harness_events::enable(&agent_dir, "runtime-next").unwrap();
        let ownership_seq =
            harness_state::claim(&agent_dir, "h.worker", "omp", "runtime-next").unwrap();
        let mut observer = EventObserver::new(
            &agent_dir, &state, "h.worker", "omp", "runtime-next", ownership_seq, "runtime-next",
        ).unwrap();
        observer.resume_generation = Some(crate::residency::Generation(2));
        observer.expected_native_session = Some("session-exact".into());
        let error = observer.observe(&json!({"type":"session","sessionId":"session-other"}))
            .unwrap_err();

        assert!(error.to_string().contains("does not match required resume"));
        assert!(!state.join("binding.json").exists());
        assert!(!state.join("binding.pending.json").exists());
    }

    /// Only the two words pi's own turn boundaries can vouch for become observations. Everything
    /// else — other frame types, unknown state words, missing fields — is dropped, so a newer
    /// extension asset cannot push this channel into recording something it cannot prove.
    #[test]
    fn only_recognized_state_frames_become_observations() {
        let active = state_observation(&json!({"type":"state","state":"active"})).unwrap();
        assert_eq!(active.state, harness_state::Activity::Active);
        assert_eq!(active.blocked_on, harness_state::BlockedOn::None);
        assert_eq!(active.input_buffer, harness_state::InputBuffer::Unknown);
        assert_eq!(
            state_observation(&json!({"type":"state","state":"idle"}))
                .unwrap()
                .state,
            harness_state::Activity::Idle
        );

        for frame in [
            json!({"type":"state","state":"child"}),
            json!({"type":"state","state":"unknown"}),
            json!({"type":"state"}),
            json!({"type":"delivered","state":"active"}),
            json!({"state":"active"}),
        ] {
            assert_eq!(state_observation(&frame), None, "frame: {frame}");
        }
    }

    /// The omp extension's approval frames carry the blocked-on-human axis pi never emits. The
    /// optional axes must decode for either channel's frames, an unrecognized ask word decodes
    /// unknown (never silently reclassified), and a blocked frame without the extras decodes as
    /// before.
    #[test]
    fn blocked_frames_carry_the_human_axes() {
        let blocked = state_observation(&json!({
            "type":"state","state":"active","blockedOn":"human",
            "ask":"permission","reason":"bash"
        }))
        .unwrap();
        assert_eq!(blocked.blocked_on, harness_state::BlockedOn::Human);
        assert_eq!(blocked.ask, harness_state::Ask::Permission);
        assert_eq!(blocked.reason.as_deref(), Some("bash"));

        let question = state_observation(&json!({
            "type":"state","state":"active","blockedOn":"human",
            "ask":"question","reason":"Which deployment target?"
        }))
        .unwrap();
        assert_eq!(question.blocked_on, harness_state::BlockedOn::Human);
        assert_eq!(question.ask, harness_state::Ask::Question);
        assert_eq!(question.reason.as_deref(), Some("Which deployment target?"));

        let unknown_ask = state_observation(&json!({
            "type":"state","state":"active","blockedOn":"human",
            "ask":"sacrifice"
        }))
        .unwrap();
        assert_eq!(unknown_ask.blocked_on, harness_state::BlockedOn::Human);
        assert_eq!(unknown_ask.ask, harness_state::Ask::Unknown);

        let plain = state_observation(&json!({"type":"state","state":"idle"})).unwrap();
        assert_eq!(plain.blocked_on, harness_state::BlockedOn::None);
        assert_eq!(plain.ask, harness_state::Ask::None);
    }

    /// Land one context frame through a real writer and read the record back.
    fn record_after(
        frames: &[Value],
        harness: harness_context::Harness,
    ) -> harness_context::Observed {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agents").join("h").join("h.worker");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let mut writer = harness_context::Writer::new(&agent_dir, "h.worker", harness).unwrap();
        for frame in frames {
            if let Some(context) = context_frame(frame) {
                write_context(&mut writer, context).unwrap();
            }
        }
        harness_context::read(&harness_context::harness_context_path(&agent_dir))
            .expect("a record must have been written")
    }

    /// HC-R13, pinned to pi 0.84.2. The payload is verbatim from the credential-free pi lab: one
    /// `message_end` for an assistant message, with `getContextUsage()` and the message's own
    /// `usage` side by side.
    ///
    /// The teeth are the numerator's MEANING, not merely its value. pi's `tokens` is the last
    /// assistant message's `totalTokens` — input + output + cacheRead + cacheWrite — and the
    /// fixture carries `input` too, so a producer that started publishing the prompt figure (which
    /// is what omp's identically-shaped call returns) fails here rather than silently publishing a
    /// differently-meaning number under the same field name. The percent is carried raw at 585.625:
    /// pi reports a float that runs far above 100 when a turn overruns the window, and a producer
    /// or reader that clamped it would hide exactly the saturation this record exists to show.
    #[test]
    fn the_pi_0_84_2_fixture_pins_total_tokens_as_the_numerator() {
        const MEASURED: &str = crate::pi_session::MEASURED_CONTEXT_VERSION;
        assert_eq!(
            MEASURED, "0.84.2",
            "the fixture below was captured on this build"
        );
        // Verbatim `event.message.usage` from the same event, carried so the assertion below can
        // name the number this producer must NOT publish.
        let message_usage = json!({
            "input": 23300, "output": 25, "cacheRead": 100, "cacheWrite": 0, "reasoning": 0,
            "totalTokens": 23425,
            "cost": {"input": 0.0699, "output": 0.000375, "cacheRead": 0.00003,
                     "cacheWrite": 0.0, "total": 0.070305}
        });
        // Verbatim `ctx.getContextUsage()` on that event, as the extension forwards it.
        let frame = json!({"type": "context", "reading": {
            "usedTokens": 23425, "windowTokens": 4000, "usedPercent": 585.625,
            "model": "fake-1", "costUsd": 0.070305
        }});

        let record = record_after(&[frame], harness_context::Harness::Pi);

        assert_eq!(record.harness, harness_context::Harness::Pi);
        assert_eq!(
            record.used_tokens,
            message_usage["totalTokens"].as_u64(),
            "pi {MEASURED}: the numerator is the assistant message's totalTokens"
        );
        assert_ne!(
            record.used_tokens,
            message_usage["input"].as_u64(),
            "pi {MEASURED}: publishing the prompt figure would be omp's arithmetic under pi's tag"
        );
        assert_eq!(record.window_tokens, Some(4000));
        assert_eq!(
            record.used_percent,
            Some(585.625),
            "carried raw, never clamped"
        );
        assert_eq!(record.model.as_deref(), Some("fake-1"));
        assert_eq!(record.cost_usd, message_usage["cost"]["total"].as_f64());
        // Neither pi nor omp carries these in v1: a lifetime sum would need a producer-side
        // accumulator whose correctness depends on having seen every message, and neither reports
        // account-scoped rate limits at all.
        assert_eq!(record.session_total_tokens, None);
        assert_eq!(record.rate_limits, harness_context::RateLimits::default());
    }

    /// HC-R13 and HC-T03's second version-coupled constant, pinned to omp 18.0.9 (the same probe
    /// run reproduces it on 18.0.3, which is why the launch gate can admit the whole 18.0 minor).
    ///
    /// omp's call has pi's exact shape and a DIFFERENT meaning: `tokens` settles to the last
    /// assistant message's prompt figure — `input` plus what pi's decomposition calls `cacheRead`
    /// — never to `totalTokens`. Measured in a controlled lab whose fake provider reported prompt
    /// tokens of 900, 9,900 and 22,500, and again on a real-credential run where `tokens` read
    /// 2,065 against that message's `totalTokens` of 2,071. So omp under-reports relative to pi by
    /// output plus cache write, and this test fails if `tokens` ever stops meaning prompt-only.
    #[test]
    fn the_omp_18_0_9_fixture_pins_prompt_input_as_the_numerator() {
        const MEASURED: [&str; 2] = crate::omp_session::MEASURED_CONTEXT_VERSIONS;
        assert_eq!(
            MEASURED,
            ["18.0.9", "18.0.3"],
            "the fixture below was captured on these builds"
        );
        let message_usage = json!({
            "input": 22400, "cacheRead": 100, "cacheWrite": 0, "output": 25,
            "totalTokens": 22525,
            "cost": {"total": 0.067605}
        });
        let frame = json!({"type": "context", "reading": {
            "usedTokens": 22500, "windowTokens": 4000, "usedPercent": 562.5,
            "model": "fake-1", "costUsd": 0.067605
        }});

        let record = record_after(&[frame], harness_context::Harness::Omp);

        assert_eq!(record.harness, harness_context::Harness::Omp);
        let prompt =
            message_usage["input"].as_u64().unwrap() + message_usage["cacheRead"].as_u64().unwrap();
        assert_eq!(
            record.used_tokens,
            Some(prompt),
            "omp {MEASURED:?}: the numerator is the assistant message's prompt tokens"
        );
        assert_ne!(
            record.used_tokens,
            message_usage["totalTokens"].as_u64(),
            "omp {MEASURED:?}: publishing totalTokens would be pi's arithmetic under omp's tag, \
             over-reporting the window by output plus cache write"
        );
        assert_eq!(record.window_tokens, Some(4000));
        assert_eq!(
            record.used_percent,
            Some(562.5),
            "carried raw, never clamped"
        );
        assert_eq!(record.cost_usd, Some(0.067605));
        assert_eq!(record.session_total_tokens, None);
    }

    /// pi's honest unknown, end to end (HC-R03). Measured inside pi's own `session_compact`
    /// handler: `getContextUsage()` already reports `{tokens: null, percent: null}` there while
    /// `contextWindow` stays populated, and `getEntries()` already counts the new entry.
    ///
    /// Two things must hold and both are silent failures. The nulls must REPLACE the previous
    /// reading rather than being carried forward — an agent whose window was just emptied must not
    /// still read 90% full. And they must land in the SAME write as the edge: a compaction edge
    /// always writes while a withheld percent has no bucket, so an edge written on its own would
    /// publish the stale pre-compaction numbers beside it and the truth would wait for the
    /// heartbeat.
    #[test]
    fn a_pi_compaction_withholds_the_reading_it_emptied_in_the_same_write() {
        let before = json!({"type": "context", "reading": {
            "usedTokens": 3625, "windowTokens": 4000, "usedPercent": 90.625,
            "model": "fake-1", "costUsd": 0.010905
        }});
        // Verbatim: reason "overflow", and `getEntries()` filtered to compactions already reads 3.
        let compacted = json!({"type": "context",
            "reading": {"usedTokens": null, "windowTokens": 4000, "usedPercent": null,
                        "model": "fake-1", "costUsd": 0.010905},
            "compaction": {"trigger": "overflow", "count": 3}});

        let full = record_after(std::slice::from_ref(&before), harness_context::Harness::Pi);
        assert_eq!(full.used_percent, Some(90.625));
        assert_eq!(full.compactions, 0);

        let record = record_after(&[before, compacted], harness_context::Harness::Pi);

        assert_eq!(
            record.used_tokens, None,
            "a withheld count is never the previous one"
        );
        assert_eq!(record.used_percent, None, "nor is a withheld percent");
        assert_eq!(
            record.window_tokens,
            Some(4000),
            "pi still knows its denominator"
        );
        assert_eq!(
            record.last_compaction_trigger,
            Some(harness_context::CompactionTrigger::Overflow),
            "pi is the only v1 producer that names its trigger"
        );
        assert_eq!(
            record.compactions, 3,
            "the count is the harness's own durable one, not st2 counting edges"
        );
        assert!(record.last_compaction_ms.is_some());
        // Withholding occupancy says nothing about cost, which the harness did not retract.
        assert_eq!(record.cost_usd, Some(0.010905));
    }

    /// omp's compaction edge carries no `reason` and no `willRetry` — pi 0.84.2 has both — so the
    /// trigger is `unknown`, a legitimate v1 value for three of the five harnesses. omp does name
    /// its auto-compaction "idle" and "threshold" internally, but those words are not projected
    /// onto the event and inventing one would be a claim no capture supports. Unlike pi, omp's
    /// `getContextUsage()` still answers inside the handler, so a real reading rides along.
    #[test]
    fn an_omp_compaction_yields_unknown_because_the_event_names_no_reason() {
        let compacted = json!({"type": "context",
            "reading": {"usedTokens": 8100, "windowTokens": 4000, "usedPercent": 202.5,
                        "model": "fake-1", "costUsd": null},
            "compaction": {"trigger": null, "count": 1}});

        let record = record_after(&[compacted], harness_context::Harness::Omp);

        assert_eq!(
            record.last_compaction_trigger,
            Some(harness_context::CompactionTrigger::Unknown)
        );
        assert_eq!(record.compactions, 1);
        assert_eq!(record.used_tokens, Some(8100));
        assert_eq!(record.cost_usd, None);
    }

    /// A count the extension could not read leaves st2 counting edges itself. That is a weaker
    /// answer — incarnation-scoped rather than harness-durable — and the point is that it is
    /// weaker rather than wrong: the edge still lands, with a trigger.
    #[test]
    fn an_unreadable_durable_count_degrades_to_counting_edges_not_to_losing_them() {
        let edge = json!({"type": "context", "compaction": {"trigger": "manual"}});

        let record = record_after(&[edge.clone(), edge], harness_context::Harness::Pi);

        assert_eq!(record.compactions, 2);
        assert_eq!(
            record.last_compaction_trigger,
            Some(harness_context::CompactionTrigger::Manual)
        );
    }

    /// The decoder's own fail-closed rules. A trigger word this version does not know decodes as
    /// `unknown` and never as a definite one; a frame with neither half is not a frame; and every
    /// other frame type is left to the other decoders.
    #[test]
    fn context_frames_decode_conservatively_or_not_at_all() {
        assert_eq!(
            compaction_trigger(Some("sacrifice")),
            harness_context::CompactionTrigger::Unknown
        );
        assert_eq!(
            compaction_trigger(None),
            harness_context::CompactionTrigger::Unknown
        );
        for word in ["manual", "auto", "threshold", "overflow", "idle"] {
            assert_eq!(compaction_trigger(Some(word)).as_str(), word);
        }

        for frame in [
            json!({"type": "context"}),
            json!({"type": "context", "reading": "nonsense"}),
            json!({"type": "state", "state": "idle"}),
            json!({"reading": {"usedTokens": 1}}),
        ] {
            assert_eq!(context_frame(&frame), None, "frame: {frame}");
        }

        // A reading whose every leg is withheld is still a reading: "the harness told us it does
        // not know" is an observation, and dropping it would leave a stale number looking current.
        let withheld = context_frame(&json!({"type": "context", "reading": {}})).unwrap();
        assert_eq!(withheld.0, Some(harness_context::Reading::default()));
        assert_eq!(withheld.1, None);
    }

    /// Withholding must mean "the harness said it does not know", never "the number arrived in a
    /// JSON shape this decoder was strict about". Both harnesses round today — pi's fallback
    /// estimator is `Math.ceil(chars / 4)` — so a fractional count is not something measured; the
    /// point is that if one ever stops rounding, a strict integer parse would silently discard a
    /// real reading and the record would be indistinguishable from an honest withheld one.
    #[test]
    fn a_fractional_token_count_is_a_reading_not_a_withheld_value() {
        assert_eq!(token_count(&json!(23425)), Some(23425));
        assert_eq!(token_count(&json!(1234.75)), Some(1235));
        assert_eq!(token_count(&json!(0)), Some(0));
        // Not counts: a negative, a non-finite, and a non-number are withheld.
        assert_eq!(token_count(&json!(-1)), None);
        assert_eq!(token_count(&json!("23425")), None);
        assert_eq!(token_count(&Value::Null), None);

        let (reading, _) = context_frame(&json!({"type": "context", "reading": {
            "usedTokens": 1234.75, "windowTokens": 4000, "usedPercent": 30.86
        }}))
        .unwrap();
        assert_eq!(reading.unwrap().used_tokens, Some(1235));
    }

    /// A context frame must never be able to take the channel down or stall the inbox, and the two
    /// axes must stay independent: the numeric record is not consulted by the categorical one and
    /// does not consult it.
    #[test]
    fn context_frames_and_state_frames_are_independent_axes_on_one_wire() {
        let context = json!({"type": "context", "reading": {"usedTokens": 10, "usedPercent": 1.0}});
        let state = json!({"type": "state", "state": "idle"});

        assert!(
            state_observation(&context).is_none(),
            "a context frame is not an observation"
        );
        assert!(
            context_frame(&state).is_none(),
            "a state frame is not a reading"
        );
    }

    /// HC-R13's version pin for pi. pi ships no runtime gate, so the only thing coupling this
    /// repository to a pi build is the flake check that type-checks and runtime-smokes the shipped
    /// asset. If that tarball moves without the fixture moving, the fixture would keep claiming a
    /// measurement of a build nothing in the tree uses any more — the exact silent drift HC-T03
    /// asks a fixture to bound.
    #[test]
    fn the_measured_pi_release_is_the_one_the_extension_gate_pins() {
        let flake = include_str!("../../../flake.nix");
        let pin = format!(
            "piVersion = \"{}\";",
            crate::pi_session::MEASURED_CONTEXT_VERSION
        );
        assert!(
            flake.contains(&pin),
            "flake.nix must pin the pi release the harness-context fixture measured ({pin})"
        );
    }

}

#[cfg(test)]
mod background_job_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn background_job_wire_count_is_positive_evidence_and_unknown_is_not_zero() {
        for (value, expected) in [(json!(2), Some(2)), (json!(0), Some(0)), (Value::Null, None), (json!(-1), None), (json!("0"), None)] {
            let frame = json!({"type":"state","state":"idle","backgroundJobs":value});
            assert_eq!(state_observation(&frame).unwrap().background_jobs, expected);
        }
    }
}
