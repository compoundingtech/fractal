//! A seat running on a model other than the one it declares (a harness's silent fallback).
//!
//! The reconciler raises one non-blocking `operational.failure` per drifting seat, episode
//! `model-drift/<agent>`, whose reason this module writes and reads. A person sees the drift as
//! one aggregated attention item per owning person, derived here from the current per-seat
//! episodes: it changes as seats start or stop drifting and is gone once none does. A read
//! never writes, so no aggregate claim exists and no host has to own one.

use super::*;
use chrono::{TimeZone as _, Utc};

/// The per-seat failure's condition.
pub const MODEL_DRIFT_CONDITION: &str = "harness-model-drift";
/// The per-seat failure's title.
pub const MODEL_DRIFT_TITLE: &str = "Seat runs on a fallback model";
/// The attention kind of the aggregated person item.
pub const MODEL_DRIFT_ATTENTION_KIND: &str = "model-drift";

/// The per-seat failure's episode.
pub fn model_drift_episode(agent: &str) -> String {
    format!("model-drift/{agent}")
}

/// One seat's drift, as its failure's reason carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDrift {
    pub declared: String,
    pub live: String,
    /// When the drift was first detected; kept across reason changes.
    pub since_ms: u128,
}

/// Incarnation-scoped main-loop evidence, never response/subagent spend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelObservation {
    pub model: String,
    pub observed_at_ms: Option<u128>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelDriftState {
    AtDeclared,
    Drifted,
    Unknown,
    Stale,
    Uncomparable,
}

/// Missing evidence is not success. Freshness is supplied by the reader's policy.
pub fn model_drift_state(
    declared: Option<&str>,
    observation: Option<&ModelObservation>,
    now: u128,
    max_age_ms: u128,
) -> ModelDriftState {
    let Some(observation) = observation else { return ModelDriftState::Unknown };
    if observation.observed_at_ms.is_none_or(|at| at > now || now - at > max_age_ms) {
        return ModelDriftState::Stale;
    }
    match declared.and_then(|declared| fractal_kernel::pricing::declared_model_matches(declared, &observation.model)) {
        Some(true) => ModelDriftState::AtDeclared,
        Some(false) => ModelDriftState::Drifted,
        None => ModelDriftState::Uncomparable,
    }
}

/// Canonical declaration reader shared by reconciliation and presentation.
pub fn declared_harness_model(desired: &Value) -> Option<&str> {
    desired.get("children")?.as_array()?.iter()
        .find(|child| child.get("name").and_then(Value::as_str) == Some("harness"))?
        .get("children")?.as_array()?.iter()
        .find(|child| child.get("name").and_then(Value::as_str) == Some("model"))?
        .get("arguments")?.as_array()?.first()?.as_str()
}

impl ModelDrift {
    /// The failure reason: readable, with `since` in a marker so later passes reuse it
    /// without reading history.
    pub fn reason(&self) -> String {
        format!(
            "declared {} but running on {} since {} [drift-since-ms:{}]",
            self.declared,
            self.live,
            clock(self.since_ms),
            self.since_ms
        )
    }

    /// The drift a reason written by [`ModelDrift::reason`] names.
    pub fn parse(reason: &str) -> Option<Self> {
        let (declared, rest) = reason.strip_prefix("declared ")?.split_once(" but running on ")?;
        let (live, rest) = rest.split_once(" since ")?;
        let since_ms = rest
            .rsplit_once("[drift-since-ms:")?
            .1
            .strip_suffix(']')?
            .parse()
            .ok()?;
        Some(Self { declared: declared.into(), live: live.into(), since_ms })
    }
}

/// A UTC wall-clock time a person reads in a title.
fn clock(at_ms: u128) -> String {
    Utc.timestamp_millis_opt(at_ms.min(i64::MAX as u128) as i64)
        .single()
        .map_or_else(|| format!("{at_ms}ms"), |time| time.format("%H:%MZ").to_string())
}

impl Store {
    /// One attention item per person while any seat they own runs on a model other than its
    /// declared one, derived from the current per-seat drift failures (with their declaration
    /// and incarnation fences). It names how many seats drift, from what to what, since when,
    /// and the seat that drifted first, which stands for the item: its host publishes it and
    /// its episode is the item's, so the item's ID changes when that seat recovers first. The
    /// per-seat failures stay faults for their owning agents and never reach a person.
    pub fn model_drift_attention_items(
        &self,
        person: Option<&str>,
        as_of: u128,
    ) -> Result<Vec<AttentionItemView>> {
        struct Seat {
            subject: String,
            episode: String,
            drift: ModelDrift,
        }
        let connection = self.readers.get();
        let mut incarnations = BTreeMap::<String, Option<String>>::new();
        let mut owners: BTreeMap<String, Vec<Seat>> = BTreeMap::new();
        for failure in self.operational_failures()? {
            if failure.status != "pending" || !failure.subject.starts_with("model-drift/") {
                continue;
            }
            let Some(fault) =
                self.operational_failure_item::<false>(&connection, &failure, None, as_of, &mut incarnations)?
            else {
                continue;
            };
            let Some(drift) = ModelDrift::parse(&failure.reason) else {
                continue;
            };
            let subject = fault.item.subject;
            // Bounded: requester headers only, never a run's full view (its steps and wake
            // history), which this read would otherwise rebuild per drifting seat.
            let Some(owner) = agent_person_at(&connection, &subject)? else {
                continue;
            };
            if person.is_some_and(|person| person != owner) {
                continue;
            }
            owners.entry(owner).or_default().push(Seat { subject, episode: fault.item.episode, drift });
        }
        let mut items = Vec::new();
        for (owner, mut seats) in owners {
            seats.sort_by(|left, right| {
                (left.drift.since_ms, &left.subject).cmp(&(right.drift.since_ms, &right.subject))
            });
            let first = &seats[0];
            let since = clock(first.drift.since_ms);
            let shared = seats.iter().all(|seat| {
                seat.drift.declared == first.drift.declared && seat.drift.live == first.drift.live
            });
            let title = match (seats.len(), shared) {
                (1, _) => format!(
                    "{} declared {} runs {} since {since}",
                    first.subject, first.drift.declared, first.drift.live
                ),
                (count, true) => format!(
                    "{count} agents declared {} run {} since {since} (oldest {})",
                    first.drift.declared, first.drift.live, first.subject
                ),
                (count, false) => format!(
                    "{count} agents run a fallback model since {since} (oldest {})",
                    first.subject
                ),
            };
            let lines = seats
                .iter()
                .map(|seat| {
                    format!(
                        "- {}: declared {}, running {} since {}",
                        seat.subject,
                        seat.drift.declared,
                        seat.drift.live,
                        clock(seat.drift.since_ms)
                    )
                })
                .collect::<Vec<_>>();
            let detail = format!(
                "These agents keep working on another model than they declare, usually because the harness fell back after provider rate limits:\n{}\nNothing is stopped. This clears by itself once each agent runs its declared model again or its declaration names the model it runs.",
                lines.join("\n")
            );
            items.push(AttentionItemView {
                episode: first.episode.clone(), priority: "normal".into(), kind: MODEL_DRIFT_ATTENTION_KIND.into(), review_mode: None,
                subject: first.subject.clone(), person: owner, requester_id: None, launch_id: None,
                variant_id: None, message_id: None, prompt_call: None, answers: Vec::new(), conversation: Some(first.subject.clone()),
                title, detail,
                request: None, deadline_unix_ms: None, default_answer: None, mission: None, mission_run: None, step: None,
                targets: seats.iter().map(|seat| seat.subject.clone()).collect(),
                requested_at_unix_ms: first.drift.since_ms,
                actions: vec![crate::model::AttentionActionView { label: "Inspect the oldest agent".into(),
                    argv: vec!["st".into(), "agents".into(), "show".into(), first.subject.clone()] }],
            });
        }
        Ok(items)
    }
}

impl Store {
    /// Reuse the detector's durable episode, but never borrow another incarnation's clock.
    pub fn model_drift_since(
        &self,
        subject: &str,
        incarnation: &str,
        declared: &str,
        live: &str,
    ) -> Result<Option<u128>> {
        let Some(failure) = self.operational_failure(&model_drift_episode(subject))?
            .filter(|failure| failure.status == "pending") else { return Ok(None) };
        let Some(claim) = self.claim_by_id(&failure.request)? else { return Ok(None) };
        if claim.body.pointer("/fields/incarnation").and_then(Value::as_str) != Some(incarnation) {
            return Ok(None);
        }
        Ok(ModelDrift::parse(&failure.reason)
            .filter(|drift| drift.declared == declared && drift.live == live)
            .map(|drift| drift.since_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drift_reason_round_trips() {
        let drift = ModelDrift {
            declared: "anthropic/claude-opus-5-5".into(),
            live: "gpt-6.1-sol".into(),
            since_ms: 1_789_000_000_000,
        };
        assert_eq!(ModelDrift::parse(&drift.reason()), Some(drift));
        assert_eq!(ModelDrift::parse("the harness did not become ready"), None);
    }

    #[test]
    fn detector_clock_is_fenced_by_incarnation_and_model_pair() {
        let store = Store::open_memory("node").unwrap();
        store.append_claim(&ClaimInput {
            subject: "agent/worker".into(), kind: "runtime.observed".into(), actor: None,
            fields: BTreeMap::from([
                ("status".into(), json!("running")),
                ("incarnation_id".into(), json!("one")),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let drift = ModelDrift { declared: "claude-opus-5-5".into(), live: "gpt-6.1-sol".into(), since_ms: 100 };
        store.record_failure(&model_drift_episode("agent/worker"), &AttentionRequest {
            reviewer: "agent/worker".into(), title: MODEL_DRIFT_TITLE.into(), reason: drift.reason(),
            severity: "error".into(), targets: vec!["agent/worker".into()], actor: "test".into(),
            idempotency_key: "drift-test".into(),
        }, MODEL_DRIFT_CONDITION).unwrap();
        assert_eq!(store.model_drift_since("agent/worker", "one", &drift.declared, &drift.live).unwrap(), Some(100));
        assert_eq!(store.model_drift_since("agent/worker", "two", &drift.declared, &drift.live).unwrap(), None);
        assert_eq!(store.model_drift_since("agent/worker", "one", "sonnet", &drift.live).unwrap(), None);
    }

    #[test]
    fn model_drift_distinguishes_missing_stale_and_uncomparable() {
        let mut observation = ModelObservation {
            model: "gpt-6.1-sol".into(), observed_at_ms: Some(100),
        };
        let classify = |observation: Option<&ModelObservation>, declared| {
            model_drift_state(declared, observation, 200, 100)
        };
        assert_eq!(classify(None, Some("gpt-6.1-sol")), ModelDriftState::Unknown);
        assert_eq!(classify(Some(&observation), Some("gpt-6.1-sol")), ModelDriftState::AtDeclared);
        assert_eq!(classify(Some(&observation), Some("anthropic/claude-opus-5-5")), ModelDriftState::Drifted);
        assert_eq!(classify(Some(&observation), None), ModelDriftState::Uncomparable);
        assert_eq!(classify(Some(&observation), Some("unregistered-model")), ModelDriftState::Uncomparable);
        observation.observed_at_ms = Some(99);
        assert_eq!(classify(Some(&observation), Some("gpt-6.1-sol")), ModelDriftState::Stale);
        observation.observed_at_ms = None;
        assert_eq!(classify(Some(&observation), Some("gpt-6.1-sol")), ModelDriftState::Stale);
        observation.observed_at_ms = Some(201);
        assert_eq!(classify(Some(&observation), Some("gpt-6.1-sol")), ModelDriftState::Stale);
    }
}
