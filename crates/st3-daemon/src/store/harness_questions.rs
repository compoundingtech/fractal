//! Questions a seat's harness asked natively: what st records of each (ids, option counts and
//! answer positions, never text), how it ended, and the one-day life of a resolved question.
//! `docs/st3/harness-questions.md` is the contract.
use super::*;
use smallclaims::store::checkpoint::claims_of_kind_in_order;
use st3_schema::harness_questions::{
    Answer, OBSERVED, Question, RESOLVED, Receipt, Status, actor_key, answers, questions, subject,
};

/// A native question as the harness shows it, without its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestionObservation {
    pub agent: String,
    pub session: String,
    pub native_id: String,
    pub runtime_incarnation: String,
    pub questions: Vec<Question>,
}

/// How a question ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuestionResolution {
    pub status: Status,
    /// The content revision it settled: the ID of the question's newest observation.
    pub revision: String,
    pub answers: BTreeMap<String, Answer>,
    /// The `harness.answer` request that settled it, when one did.
    pub receipt: Option<Receipt>,
    pub resolved_at_unix_ms: u128,
}

/// A question as st holds it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HarnessQuestion {
    pub id: String,
    /// The ID of the newest observation. Settling the question leaves it unchanged.
    pub revision: String,
    pub agent: String,
    pub session: String,
    pub native_id: String,
    pub runtime_incarnation: String,
    pub questions: Vec<Question>,
    pub observed_at_unix_ms: u128,
    pub resolution: Option<QuestionResolution>,
}

impl HarnessQuestion {
    /// Whether `actor`'s request with idempotency key `key` settled this question.
    pub fn settled_by(&self, actor: &str, key: &str) -> Option<&Receipt> {
        let receipt = self.resolution.as_ref()?.receipt.as_ref()?;
        (receipt.actor_key == actor_key(actor, key)).then_some(receipt)
    }
}

fn field<T: for<'de> serde::Deserialize<'de>>(claim: &ClaimRecord, name: &str) -> Result<T> {
    serde_json::from_value(claim.body.pointer(&format!("/fields/{name}")).cloned().unwrap_or(Value::Null))
        .with_context(|| format!("{} `{}` has no valid {name}", claim.kind, claim.id))
}

/// The question `subject`: its newest observation and its first resolution in canonical order.
/// With `at`, a question resolved at least a day before it is gone, as it is once a checkpoint
/// drops it.
pub(crate) fn question_at(
    connection: &Connection,
    subject: &str,
    at: Option<u128>,
) -> Result<Option<HarnessQuestion>> {
    let Some(observed) = claims_of_kind_in_order(connection, subject, OBSERVED)?.pop() else {
        return Ok(None);
    };
    let resolution = match claims_of_kind_in_order(connection, subject, RESOLVED)?.first() {
        Some(claim) => {
            if at.is_some_and(|at| checkpoint_rules::question_expired(claim.accepted_at_unix_ms, at)) {
                return Ok(None);
            }
            let fields = claim.body["fields"].as_object().cloned().unwrap_or_default();
            Some(QuestionResolution {
                status: field(claim, "status")?,
                revision: field(claim, "revision")?,
                answers: answers(&fields.into_iter().collect())
                    .map_err(|error| anyhow::anyhow!("{}: {error}", claim.id))?,
                receipt: field(claim, "receipt")?,
                resolved_at_unix_ms: claim.accepted_at_unix_ms,
            })
        }
        None => None,
    };
    let fields = observed.body["fields"].as_object().cloned().unwrap_or_default();
    Ok(Some(HarnessQuestion {
        id: subject.to_owned(),
        revision: observed.id.clone(),
        agent: field(&observed, "agent")?,
        session: field(&observed, "session")?,
        native_id: field(&observed, "native_id")?,
        runtime_incarnation: field(&observed, "runtime_incarnation")?,
        questions: questions(&fields.into_iter().collect())
            .map_err(|error| anyhow::anyhow!("{}: {error}", observed.id))?,
        observed_at_unix_ms: observed.accepted_at_unix_ms,
        resolution,
    }))
}

fn json_fields(value: Value) -> BTreeMap<String, Value> {
    match value {
        Value::Object(fields) => fields.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

impl Store {
    /// The question `subject` as of `at`, or `None` when st holds no such question or it was
    /// resolved at least a day before `at`.
    pub fn harness_question(&self, subject: &str, at: u128) -> Result<Option<HarnessQuestion>> {
        question_at(&self.readers.get(), subject, Some(at))
    }

    /// The question `subject` as long as st holds its claims, resolved a day ago or not.
    fn harness_question_held(&self, subject: &str) -> Result<Option<HarnessQuestion>, St3Error> {
        question_at(&self.readers.get(), subject, None).map_err(internal)
    }

    /// Record what the harness shows of a question. The same content again changes nothing, so
    /// its revision holds; new content is a new revision. A resolved question takes no new
    /// observation.
    pub fn observe_harness_question(
        &self,
        observation: &QuestionObservation,
    ) -> Result<HarnessQuestion, St3Error> {
        self.observe_harness_question_after_read(observation, || {})
    }

    fn observe_harness_question_after_read(
        &self,
        observation: &QuestionObservation,
        after_read: impl FnOnce(),
    ) -> Result<HarnessQuestion, St3Error> {
        let subject = subject(&observation.agent, &observation.session, &observation.native_id);
        let current = self.harness_question_held(&subject)?;
        // Fence the observation we inspected, not a head that may already contain a
        // concurrent resolution or replacement observation.
        let expected = current.as_ref().map(|question| question.revision.clone());
        after_read();
        if let Some(current) = current {
            if current.resolution.is_some() {
                return Err(St3Error::new(
                    "harness-question-resolved",
                    "a resolved question takes no new observation",
                )
                .with_detail("question", subject));
            }
            if current.runtime_incarnation == observation.runtime_incarnation
                && current.questions == observation.questions
            {
                return Ok(current);
            }
        } else if self.latest_claim(&subject, None).map_err(internal)?.is_some() {
            // Only a question resolved and dropped leaves nothing to fold; it stays ended.
            return Err(St3Error::new("harness-question-resolved", "the question already ended")
                .with_detail("question", subject));
        }
        self.append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: OBSERVED.into(),
            actor: None,
            fields: json_fields(json!({
                "agent": observation.agent,
                "session": observation.session,
                "native_id": observation.native_id,
                "runtime_incarnation": observation.runtime_incarnation,
                "questions": observation.questions,
            })),
            evidence: Vec::new(),
            expected_subject: Some(expected),
            idempotency_key: None,
        })?;
        self.harness_question_held(&subject)?
            .ok_or_else(|| St3Error::new("internal", "the observed question did not fold"))
    }

    /// Record how a question ended, once. The resolution must settle the question's current
    /// revision and answer only its questions with their positions; each answer carries the
    /// harness's own `timed_out`. A question that already ended returns its first resolution
    /// and `false`.
    pub fn resolve_harness_question(
        &self,
        subject: &str,
        status: Status,
        revision: &str,
        answers: &BTreeMap<String, Answer>,
        receipt: Option<&Receipt>,
    ) -> Result<(HarnessQuestion, bool), St3Error> {
        let current = self
            .harness_question_held(subject)?
            .ok_or_else(|| {
                St3Error::new("unknown-harness-question", "st holds no such question")
                    .with_detail("question", subject)
            })?;
        if current.resolution.is_some() {
            return Ok((current, false));
        }
        if current.revision != revision {
            return Err(St3Error::new("stale-subject", "the question changed")
                .with_detail("question", subject)
                .with_detail("revision", current.revision));
        }
        for (id, answer) in answers {
            let question = current.questions.iter().find(|question| &question.id == id);
            if question.is_none_or(|question| {
                answer.positions.iter().any(|position| *position >= question.options)
            }) {
                return Err(St3Error::new(
                    "invalid-harness-question",
                    "a resolution answers only the question's own options",
                )
                .with_detail("question", subject));
            }
        }
        let mut fields = json!({"status": status, "revision": revision, "answers": answers});
        if let Some(receipt) = receipt {
            fields["receipt"] = json!(receipt);
        }
        let written = self.append_claim(&ClaimInput {
            subject: subject.to_owned(),
            kind: RESOLVED.into(),
            actor: None,
            fields: json_fields(fields),
            evidence: Vec::new(),
            // Settles exactly the revision it answered: a new observation or another
            // resolution since then moves the head.
            expected_subject: Some(Some(revision.to_owned())),
            idempotency_key: None,
        });
        match written {
            Ok(_) => {}
            Err(error) if error.code == "stale-subject" => {
                let current = self.harness_question_held(subject)?;
                if let Some(current) = current.filter(|current| current.resolution.is_some()) {
                    return Ok((current, false));
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        let resolved = self
            .harness_question_held(subject)?
            .ok_or_else(|| St3Error::new("internal", "the resolved question did not fold"))?;
        Ok((resolved, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u128 = st3_schema::harness_questions::RESOLVED_RETENTION_MS;

    fn observation() -> QuestionObservation {
        QuestionObservation {
            agent: "agent/ada".into(),
            session: "tester".into(),
            native_id: "call-1".into(),
            runtime_incarnation: "inc-1".into(),
            questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
        }
    }

    fn answer(position: u8) -> BTreeMap<String, Answer> {
        BTreeMap::from([("q0".into(), Answer { positions: vec![position], timed_out: false })])
    }

    #[test]
    fn the_same_observation_keeps_its_revision_and_new_content_moves_it() {
        let store = Store::open_memory("alder").unwrap();
        let first = store.observe_harness_question(&observation()).unwrap();
        assert_eq!(store.observe_harness_question(&observation()).unwrap().revision, first.revision);
        let mut changed = observation();
        changed.questions[0].options = 3;
        let second = store.observe_harness_question(&changed).unwrap();
        assert_ne!(second.revision, first.revision);
        assert_eq!(second.id, first.id);
    }

    #[test]
    fn a_resolution_between_observation_read_and_append_refuses_new_content() {
        let store = Store::open_memory("operator").unwrap();
        let question = store.observe_harness_question(&observation()).unwrap();
        let mut changed = observation();
        changed.questions[0].options = 3;
        let error = store
            .observe_harness_question_after_read(&changed, || {
                store
                    .resolve_harness_question(
                        &question.id,
                        Status::Answered,
                        &question.revision,
                        &answer(1),
                        None,
                    )
                    .unwrap();
            })
            .unwrap_err();
        assert_eq!(error.code, "stale-subject");
        let held = store.harness_question_held(&question.id).unwrap().unwrap();
        assert_eq!(held.revision, question.revision);
        assert_eq!(held.questions, question.questions);
        assert_eq!(held.resolution.unwrap().revision, question.revision);
    }

    #[test]
    fn a_question_resolves_once_and_settling_leaves_its_revision() {
        let store = Store::open_memory("alder").unwrap();
        let question = store.observe_harness_question(&observation()).unwrap();
        let receipt = Receipt {
            actor_key: actor_key("person/alex", "tester-operator-key"),
            request_digest: "b".repeat(64),
            action_id: "action/tester".into(),
        };
        let (resolved, new) = store
            .resolve_harness_question(&question.id, Status::Answered, &question.revision, &answer(1), Some(&receipt))
            .unwrap();
        assert!(new);
        assert_eq!(resolved.revision, question.revision);
        assert!(resolved.settled_by("person/alex", "tester-operator-key").is_some());
        assert!(resolved.settled_by("person/operator", "tester-operator-key").is_none());
        let (again, new) = store
            .resolve_harness_question(&question.id, Status::AnsweredElsewhere, &question.revision, &answer(0), None)
            .unwrap();
        assert!(!new);
        assert_eq!(again.resolution, resolved.resolution);
        assert_eq!(
            store.observe_harness_question(&observation()).unwrap_err().code,
            "harness-question-resolved"
        );
    }

    #[test]
    fn a_resolution_answers_the_current_revision_with_its_own_options() {
        let store = Store::open_memory("alder").unwrap();
        let question = store.observe_harness_question(&observation()).unwrap();
        assert_eq!(
            store
                .resolve_harness_question(&question.id, Status::Answered, &question.revision, &answer(2), None)
                .unwrap_err()
                .code,
            "invalid-harness-question"
        );
        let mut changed = observation();
        changed.runtime_incarnation = "inc-2".into();
        store.observe_harness_question(&changed).unwrap();
        assert_eq!(
            store
                .resolve_harness_question(&question.id, Status::Answered, &question.revision, &answer(1), None)
                .unwrap_err()
                .code,
            "stale-subject"
        );
    }

    #[test]
    fn a_resolved_question_is_gone_a_day_after_its_resolution() {
        let store = Store::open_memory("alder").unwrap();
        store.set_write_clock_at(10 * DAY).unwrap();
        let question = store.observe_harness_question(&observation()).unwrap();
        let (resolved, _) = store
            .resolve_harness_question(&question.id, Status::Expired, &question.revision, &BTreeMap::new(), None)
            .unwrap();
        let at = resolved.resolution.as_ref().unwrap().resolved_at_unix_ms;
        assert!(store.harness_question(&question.id, at + DAY - 1).unwrap().is_some());
        assert!(store.harness_question(&question.id, at + DAY).unwrap().is_none());
        // A pending question has no end of life.
        let mut other = observation();
        other.native_id = "call-2".into();
        let pending = store.observe_harness_question(&other).unwrap();
        assert!(store.harness_question(&pending.id, 1000 * DAY).unwrap().is_some());
    }
}
