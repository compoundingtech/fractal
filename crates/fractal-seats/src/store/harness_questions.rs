//! The part of `store::harness_questions` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::store::checkpoint::claims_of_kind_in_order;
use fractal_schema::harness_questions::{
    Answer, OBSERVED, Question, RESOLVED, Receipt, Status, actor_key, answers, questions, subject,
};
pub use fractal_missions::store::harness_questions::*;
pub use fractal_missions::store::harness_questions::{HarnessQuestion, QuestionResolution, question_at};

/// A native question as the harness shows it, without its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestionObservation {
    pub agent: String,
    pub session: String,
    /// Raw native call identity for `observe_harness_question`; the digest variant requires
    /// the already-hashed identity emitted by the native outbox.
    pub native_id: String,
    pub runtime_incarnation: String,
    pub questions: Vec<Question>,
}

fn json_fields(value: Value) -> BTreeMap<String, Value> {
    match value {
        Value::Object(fields) => fields.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

pub trait HarnessQuestionsStore {
    /// The question `subject`, including its indefinitely retained resolution when it ended.
    fn harness_question(&self, subject: &str, at: u128) -> Result<Option<HarnessQuestion>>;
    /// The question `subject` as long as st holds its claims.
    fn harness_question_held(&self, subject: &str) -> Result<Option<HarnessQuestion>, St3Error>;
    /// Record what the harness shows of a question. The same content again changes nothing, so
    /// its revision holds; new content is a new revision. A resolved question takes no new
    /// observation.
    fn observe_harness_question(
            &self,
            observation: &QuestionObservation,
        ) -> Result<HarnessQuestion, St3Error>;
    /// Ingest an already-hashed native identity from the text-free native outbox.
    fn observe_harness_question_digest(
            &self,
            observation: &QuestionObservation,
        ) -> Result<HarnessQuestion, St3Error>;
    fn observe_harness_question_after_read(
            &self,
            observation: &QuestionObservation,
            after_read: impl FnOnce(),
        ) -> Result<HarnessQuestion, St3Error>;
    /// Record how a question ended, once. The resolution must settle the question's current
    /// revision and answer only its questions with their positions; each answer carries the
    /// harness's own `timed_out`. A question that already ended returns its first resolution
    /// and `false`.
    fn resolve_harness_question(
            &self,
            subject: &str,
            status: Status,
            revision: &str,
            answers: &BTreeMap<String, Answer>,
            receipt: Option<&Receipt>,
        ) -> Result<(HarnessQuestion, bool), St3Error>;
}

impl HarnessQuestionsStore for Store {

    fn harness_question(&self, subject: &str, at: u128) -> Result<Option<HarnessQuestion>> {
        question_at(&self.readers.get(), subject, Some(at))
    }

    fn harness_question_held(&self, subject: &str) -> Result<Option<HarnessQuestion>, St3Error> {
        question_at(&self.readers.get(), subject, None).map_err(internal)
    }

    fn observe_harness_question(
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
        let native_id = fractal_schema::harness_questions::native_id_digest(&observation.native_id);
        observe_identity(self, observation, &native_id, after_read)
    }

    fn observe_harness_question_digest(
            &self,
            observation: &QuestionObservation,
        ) -> Result<HarnessQuestion, St3Error> {
        observe_identity(self, observation, &observation.native_id, || {})
    }

    fn resolve_harness_question(
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

fn observe_identity(
    store: &Store,
    observation: &QuestionObservation,
    native_id: &str,
    after_read: impl FnOnce(),
) -> Result<HarnessQuestion, St3Error> {
    let subject = subject(&observation.agent, &observation.session, native_id);
    let current = store.harness_question_held(&subject)?;
    let expected = current.as_ref().map(|question| question.revision.clone());
    after_read();
    if let Some(current) = current {
        if current.resolution.is_some() {
            return Err(St3Error::new(
                "harness-question-resolved", "a resolved question takes no new observation",
            ).with_detail("question", subject));
        }
        if current.runtime_incarnation == observation.runtime_incarnation
            && current.questions == observation.questions
        {
            return Ok(current);
        }
    }
    store.append_claim(&ClaimInput {
        subject: subject.clone(),
        kind: OBSERVED.into(),
        actor: None,
        fields: json_fields(json!({
            "agent": observation.agent, "session": observation.session, "native_id": native_id,
            "runtime_incarnation": observation.runtime_incarnation, "questions": observation.questions,
        })),
        evidence: Vec::new(),
        expected_subject: Some(expected),
        idempotency_key: None,
    })?;
    store.harness_question_held(&subject)?
        .ok_or_else(|| St3Error::new("internal", "the observed question did not fold"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use fractal_missions::store::harness_questions_tests_shared_seats::*;
    use fractal_missions::store::harness_questions_tests_shared_seats::{DAY, answer};

    fn observation() -> QuestionObservation {
        QuestionObservation {
            agent: "agent/ada".into(),
            session: "tester".into(),
            native_id: "call-1".into(),
            runtime_incarnation: "inc-1".into(),
            questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }],
        }
    }

    #[test]
    fn native_path_and_command_ids_are_hashed_before_storage() {
        let store = Store::open_memory("operator").unwrap();
        for native in ["/home/ada/secret.txt", "cmd --token=x"] {
            let mut observed = observation();
            observed.native_id = native.into();
            let question = store.observe_harness_question(&observed).unwrap();
            assert_eq!(question.native_id, fractal_schema::harness_questions::native_id_digest(native));
            assert_eq!(question.id, subject(&observed.agent, &observed.session, &question.native_id));
            for claim in store.claims_for(&question.id, None).unwrap() {
                assert!(!claim.body.to_string().contains(native));
            }
            assert_eq!(store.observe_harness_question(&observed).unwrap(), question);
        }
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
    fn an_expired_question_keeps_its_answer_and_receipt_forever_and_cannot_resurrect() {
        let store = Store::open_memory("operator").unwrap();
        store.set_write_clock_at(10 * DAY).unwrap();
        let question = store.observe_harness_question(&observation()).unwrap();
        let receipt = Receipt {
            actor_key: actor_key("person/alex", "request-1"),
            request_digest: "a".repeat(64),
            action_id: "action/tester".into(),
        };
        let (resolved, _) = store
            .resolve_harness_question(&question.id, Status::Expired, &question.revision, &answer(1), Some(&receipt))
            .unwrap();
        assert_eq!(store.harness_question(&question.id, u128::MAX).unwrap(), Some(resolved.clone()));
        assert_eq!(resolved.resolution.as_ref().unwrap().answers, answer(1));
        assert_eq!(resolved.settled_by("person/alex", "request-1"), Some(&receipt));
        assert_eq!(store.observe_harness_question(&observation()).unwrap_err().code, "harness-question-resolved");
        // A pending question has no end of life.
        let mut other = observation();
        other.native_id = "call-2".into();
        let pending = store.observe_harness_question(&other).unwrap();
        assert!(store.harness_question(&pending.id, 1000 * DAY).unwrap().is_some());
    }

    // A seat's current harness, its status history and its native questions fold replicated
    // runtime, harness and question claims. Two writers record them, several in one
    // millisecond, and resolve different revisions of one question: every delivery order,
    // batching and repetition must read the same.
    fn harness_fixture() -> &'static (crate::model::ReplicationExchange, String) {
        static FIXTURE: std::sync::OnceLock<(crate::model::ReplicationExchange, String)> =
            std::sync::OnceLock::new();
        FIXTURE.get_or_init(|| {
            let alder = Store::open_memory("alder").unwrap();
            let birch = Store::open_memory("birch").unwrap();
            let relay = Store::open_memory("cedar").unwrap();
            let seat = observation().agent;
            let state = |store: &Store, kind: &str, fields: Value| {
                store
                    .append_claim(&ClaimInput {
                        subject: seat.clone(),
                        kind: kind.into(),
                        actor: Some(seat.clone()),
                        fields: serde_json::from_value(fields).unwrap(),
                        evidence: Vec::new(),
                        expected_subject: None,
                        idempotency_key: None,
                    })
                    .unwrap();
            };
            alder.set_write_clock_at(10 * DAY).unwrap();
            state(&alder, "runtime.observed", json!({"status": "running", "runtime_id": "runtime-1", "incarnation_id": "inc-1"}));
            state(&alder, "harness.observed", json!({"state": "working", "driver": "claude", "incarnation_id": "inc-1"}));
            let first = alder.observe_harness_question(&observation()).unwrap();
            fractal_store::store::tests_shared::receive_and_project(
                &birch,
                "alder",
                &fractal_store::store::tests_shared::exchange_from(&alder, &ReplicationInventory::default()),
            );
            // Both writers in the same later millisecond.
            for store in [&alder, &birch] {
                store.set_write_clock_at(10 * DAY + 5_000).unwrap();
            }
            state(&birch, "harness.observed", json!({"state": "idle", "driver": "claude", "incarnation_id": "inc-1"}));
            state(&alder, "harness.observed", json!({"state": "blocked", "driver": "claude", "incarnation_id": "inc-1"}));
            let mut changed = observation();
            changed.questions[0].options = 3;
            let second = birch.observe_harness_question(&changed).unwrap();
            birch
                .resolve_harness_question(&second.id, Status::Answered, &second.revision, &answer(2), None)
                .unwrap();
            alder
                .resolve_harness_question(&first.id, Status::Expired, &first.revision, &answer(1), None)
                .unwrap();
            for (name, source) in [("alder", &alder), ("birch", &birch)] {
                let exchange =
                    fractal_store::store::tests_shared::exchange_from(source, &ReplicationInventory::default());
                fractal_store::store::tests_shared::receive_and_project(&relay, name, &exchange);
            }
            (
                fractal_store::store::tests_shared::exchange_from(&relay, &ReplicationInventory::default()),
                first.id,
            )
        })
    }

    fn harness_snapshot(store: &Store) -> Value {
        let (_, question) = harness_fixture();
        let seat = observation().agent;
        let connection = store.readers.get();
        let index = store.index().unwrap();
        json!({
            "harness": fractal_store::store::current_harness_at(&connection, &seat, None).unwrap(),
            "history": fractal_store::store::seat_status::history_at(&connection, &seat, 20 * DAY, index).unwrap(),
            "question": store.harness_question(question, u128::MAX).unwrap(),
        })
    }

    fn deliver_harness(target: &Store, order: &[usize], width: usize, copies: usize) {
        let (exchange, _) = harness_fixture();
        for chunk in order.chunks(width) {
            let envelopes = chunk
                .iter()
                .flat_map(|&index| std::iter::repeat_n(exchange.envelopes[index].clone(), copies))
                .collect();
            let admission = fractal_store::store::tests_shared::receive_and_project(
                target,
                "cedar",
                &fractal_store::store::tests_shared_seats::exchange_of("cedar", envelopes),
            );
            assert_eq!((admission.invalid, admission.unknown), (0, 0));
        }
    }

    fn harness_reference() -> &'static Value {
        static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        REFERENCE.get_or_init(|| {
            let target = Store::open_memory("dogwood").unwrap();
            let count = harness_fixture().0.envelopes.len();
            deliver_harness(&target, &(0..count).collect::<Vec<_>>(), usize::MAX, 1);
            harness_snapshot(&target)
        })
    }

    #[test]
    fn the_harness_fixture_resolves_the_question_once() {
        let reference = harness_reference();
        assert!(reference["harness"].is_object());
        assert!(reference["question"]["resolution"].is_object());
    }

    #[test]
    fn harness_folds_survive_redelivery_restart_and_replay() {
        let (exchange, _) = harness_fixture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let target = Store::open(&path, "dogwood").unwrap();
        deliver_harness(&target, &(0..exchange.envelopes.len()).rev().collect::<Vec<_>>(), 1, 1);
        assert_eq!(&harness_snapshot(&target), harness_reference());
        fractal_store::store::tests_shared::receive_and_project(&target, "cedar", exchange);
        assert_eq!(&harness_snapshot(&target), harness_reference());
        drop(target);
        let target = Store::open(&path, "dogwood").unwrap();
        assert_eq!(&harness_snapshot(&target), harness_reference());
        target.replay_replication_graph().unwrap();
        assert_eq!(&harness_snapshot(&target), harness_reference());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

        #[test]
        fn harness_folds_are_pure_under_shuffled_split_and_duplicate_delivery(
            ranks in proptest::collection::vec(proptest::prelude::any::<u64>(), 32),
            width in 1usize..5,
            copies in 1usize..3,
        ) {
            let count = harness_fixture().0.envelopes.len();
            proptest::prop_assert!(count <= ranks.len());
            let mut shuffled = (0..count).collect::<Vec<_>>();
            shuffled.sort_by_key(|&index| (ranks[index], index));
            let target = Store::open_memory("dogwood").unwrap();
            deliver_harness(&target, &shuffled, width, copies);
            proptest::prop_assert_eq!(&harness_snapshot(&target), harness_reference());
        }
    }
}
