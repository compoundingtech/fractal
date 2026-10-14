//! A question a seat's harness asked natively, as st records it: question ids, option counts and
//! answer positions, never text. The question text, option labels, free-text answers and notes
//! stay in the harness's own timeline; a client joins them from there.
//! `docs/st3/harness-questions.md` is the contract.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::{ValidationError, error};

pub const FAMILY: &str = "harness-question";
/// The question as the harness shows it. A later observation of the same question is a new
/// content revision; settling it writes [`RESOLVED`] and leaves the revision unchanged.
pub const OBSERVED: &str = "harness-question.observed";
/// The question's terminal status and answers, written once.
pub const RESOLVED: &str = "harness-question.resolved";

/// At most this many questions in one native call.
pub const MAX_QUESTIONS: usize = 16;
/// At most this many options in one question; positions count from zero.
pub const MAX_OPTIONS: u8 = 64;
/// The longest question id, native id or session id st keeps.
pub const MAX_ID_BYTES: usize = 128;
/// How long a resolved question and its answer receipt stay after the resolution, in
/// milliseconds. A checkpoint drops both once its cut is at least this long after the resolution.
pub const RESOLVED_RETENTION_MS: u128 = 24 * 60 * 60 * 1000;

/// One question of a native call. `options` is the number of choices, so an answer names
/// positions below it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub options: u8,
    pub multi: bool,
    pub required: bool,
}

/// How a question ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Answered through `harness.answer`.
    Answered,
    /// Answered in the harness itself.
    AnsweredElsewhere,
    /// The harness gave up waiting.
    Expired,
    /// The harness or its person withdrew the question.
    Cancelled,
}

/// One question's answer: the chosen positions and whether the harness timed the question out.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub positions: Vec<u8>,
    pub timed_out: bool,
}

/// The `harness.answer` request that settled a question: the digest of its actor and
/// idempotency key, the digest of its content, and its action ID. A retry with the same key and
/// content replays the result; the same key with other content is a conflict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub actor_key: String,
    pub request_digest: String,
    pub action_id: String,
}

/// The subject of the question `native_id` that `agent` asked in `session`. Each part is length
/// prefixed, so no two triples share a subject.
pub fn subject(agent: &str, session: &str, native_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"st3-harness-question-v1\0");
    for part in [agent, session, native_id] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{FAMILY}/{}", hex::encode(&digest.finalize()[..16]))
}

/// The digest a receipt stores for `actor`'s idempotency key `key`.
pub fn actor_key(actor: &str, key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"st3-harness-answer-key-v1\0");
    digest.update((actor.len() as u64).to_be_bytes());
    digest.update(actor.as_bytes());
    digest.update(key.as_bytes());
    hex::encode(digest.finalize())
}

fn bounded_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && !value.chars().any(|character| character.is_control() || character.is_whitespace())
}

fn invalid(message: impl Into<String>) -> ValidationError {
    error("invalid-harness-question", message)
}

fn decode<T: for<'de> Deserialize<'de>>(name: &str, value: Option<&Value>) -> Result<T, ValidationError> {
    serde_json::from_value(value.cloned().unwrap_or(Value::Null))
        .map_err(|_| invalid(format!("`{name}` does not have the harness question shape")))
}

/// The questions of an observation, checked: bounded ids, unique ids, bounded option counts.
pub fn questions(fields: &BTreeMap<String, Value>) -> Result<Vec<Question>, ValidationError> {
    let questions: Vec<Question> = decode("questions", fields.get("questions"))?;
    if questions.is_empty() || questions.len() > MAX_QUESTIONS {
        return Err(invalid(format!("a native call has 1 to {MAX_QUESTIONS} questions")));
    }
    let mut ids = BTreeSet::new();
    for question in &questions {
        if !bounded_id(&question.id) || !ids.insert(question.id.as_str()) {
            return Err(invalid("each question needs a unique bounded id"));
        }
        if question.options > MAX_OPTIONS {
            return Err(invalid(format!("a question has at most {MAX_OPTIONS} options")));
        }
    }
    Ok(questions)
}

/// The answers of a resolution, checked: bounded question ids and unique positions below the
/// option limit. Which positions a question allows is checked against its observation.
pub fn answers(fields: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Answer>, ValidationError> {
    let answers: BTreeMap<String, Answer> = decode("answers", fields.get("answers"))?;
    if answers.len() > MAX_QUESTIONS {
        return Err(invalid(format!("a resolution answers at most {MAX_QUESTIONS} questions")));
    }
    for (id, answer) in &answers {
        let unique = answer.positions.iter().collect::<BTreeSet<_>>();
        if !bounded_id(id)
            || unique.len() != answer.positions.len()
            || answer.positions.iter().any(|position| *position >= MAX_OPTIONS)
        {
            return Err(invalid("each answer needs a bounded question id and unique positions"));
        }
    }
    Ok(answers)
}

pub(crate) fn validate(kind: &str, fields: &BTreeMap<String, Value>) -> Result<(), ValidationError> {
    match kind {
        OBSERVED => {
            for name in ["session", "native_id", "runtime_incarnation"] {
                if !fields.get(name).and_then(Value::as_str).is_some_and(bounded_id) {
                    return Err(invalid(format!("`{name}` must be a bounded id")));
                }
            }
            questions(fields).map(|_| ())
        }
        RESOLVED => {
            answers(fields)?;
            if let Some(receipt) = fields.get("receipt") {
                let receipt: Receipt = decode("receipt", Some(receipt))?;
                let digest = |value: &str| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                };
                if !digest(&receipt.actor_key)
                    || !digest(&receipt.request_digest)
                    || !receipt.action_id.starts_with("action/")
                    || receipt.action_id.len() > 256
                {
                    return Err(invalid("a receipt holds two digests and an action id"));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(value: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_subject_names_one_agent_session_and_native_call() {
        let one = subject("agent/run/ada", "tester", "call-1");
        assert!(one.starts_with("harness-question/"));
        assert_eq!(one, subject("agent/run/ada", "tester", "call-1"));
        assert_ne!(one, subject("agent/run/ada", "tester", "call-2"));
        // Length prefixes keep a boundary shift from colliding.
        assert_ne!(subject("a", "bc", "d"), subject("ab", "c", "d"));
    }

    #[test]
    fn an_observation_holds_ids_and_counts_only() {
        let valid = json!({"agent": "agent/run/ada", "session": "tester", "native_id": "call-1",
            "runtime_incarnation": "inc-1",
            "questions": [{"id": "q0", "options": 3, "multi": false, "required": true}]});
        assert!(validate(OBSERVED, &fields(valid.clone())).is_ok());
        for questions in [
            json!([]),
            json!([{"id": "q0", "options": 3, "multi": false, "required": true, "text": "Which?"}]),
            json!([{"id": "q0", "options": 65, "multi": false, "required": true}]),
            json!([{"id": "q0", "options": 1, "multi": false, "required": true},
                   {"id": "q0", "options": 1, "multi": false, "required": true}]),
        ] {
            let mut invalid = valid.clone();
            invalid["questions"] = questions;
            assert_eq!(validate(OBSERVED, &fields(invalid)).unwrap_err().code, "invalid-harness-question");
        }
    }

    #[test]
    fn a_resolution_holds_positions_and_native_timeouts_only() {
        let valid = json!({"status": "answered", "revision": "claim-1",
            "answers": {"q0": {"positions": [1], "timed_out": false}},
            "receipt": {"actor_key": "a".repeat(64), "request_digest": "b".repeat(64),
                "action_id": "action/tester"}});
        assert!(validate(RESOLVED, &fields(valid.clone())).is_ok());
        for answers in [
            json!({"q0": {"positions": [1]}}),
            json!({"q0": {"positions": [1, 1], "timed_out": false}}),
            json!({"q0": {"positions": [1], "timed_out": false, "other": "free text"}}),
            json!({"q0": {"positions": [1], "timed_out": false, "note": "free text"}}),
        ] {
            let mut invalid = valid.clone();
            invalid["answers"] = answers;
            assert!(validate(RESOLVED, &fields(invalid)).is_err());
        }
        let mut invalid = valid;
        invalid["receipt"]["idempotency_key"] = json!("tester-operator-key");
        assert!(validate(RESOLVED, &fields(invalid)).is_err());
    }
}
