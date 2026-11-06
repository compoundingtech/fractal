//! Text-free observations of native OMP asks. This module cannot deliver answers.
use anyhow::{Result, ensure};
use fractal_schema::harness_questions::{MAX_OPTIONS, MAX_QUESTIONS, Question};
use serde::{Deserialize, Serialize};

pub fn enabled() -> bool {
    std::env::var("ST_OMP_WEB_ANSWERS").as_deref() == Ok("1")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Observation {
    #[serde(rename = "question.open")]
    Open { native_id: String, session: String, questions: Vec<Question> },
    #[serde(rename = "question.closed")]
    Closed { native_id: String, session: String, status: CloseStatus },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseStatus { AnsweredElsewhere, Expired, Cancelled }

impl Observation {
    pub fn native_id(&self) -> &str {
        match self { Self::Open { native_id, .. } | Self::Closed { native_id, .. } => native_id }
    }
    pub fn session(&self) -> &str {
        match self { Self::Open { session, .. } | Self::Closed { session, .. } => session }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.native_id().strip_prefix("sha256:").is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }), "native question identity is not a canonical digest");
        ensure!(!self.session().is_empty() && self.session().len() <= 128
            && !self.session().chars().any(|c| c.is_whitespace() || c.is_control()),
            "native question session is not bounded");
        if let Self::Open { questions, .. } = self {
            ensure!(!questions.is_empty() && questions.len() <= MAX_QUESTIONS,
                "native question count is out of bounds");
            for (index, question) in questions.iter().enumerate() {
                ensure!(question.id == format!("q{index}") && question.options <= MAX_OPTIONS
                    && question.required != question.multi,
                    "native question descriptors do not follow positional ask semantics");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn descriptors_are_closed_bounded_and_positional() {
        let open = json!({"type":"question.open", "session":"native-session",
            "native_id":fractal_schema::harness_questions::native_id_digest("call"),
            "questions":[{"id":"q0", "options":2, "multi":false, "required":true}]});
        let decode = |value| serde_json::from_value::<Observation>(value).unwrap();
        assert!(decode(open.clone()).validate().is_ok());
        let mut wrong = open.clone();
        wrong["questions"][0]["id"] = "q1".into();
        assert!(decode(wrong).validate().is_err());
        let mut raw = open.clone();
        raw["native_id"] = "PRIVATE_CALL_CANARY".into();
        assert!(decode(raw).validate().is_err());
        let mut text = open;
        text["questions"][0]["question"] = "PRIVATE_QUESTION_CANARY".into();
        assert!(serde_json::from_value::<Observation>(text).is_err());
    }

    #[test]
    fn text_free_open_and_close_survive_outbox_restart() {
        use crate::{harness_events, harness_state};
        let root = tempfile::tempdir().unwrap();
        harness_events::enable(root.path(), "runtime-one").unwrap();
        let sequence = harness_state::claim(root.path(), "example/question", "omp", "owner").unwrap();
        let mut writer = harness_state::Writer::new(
            root.path(), "example/question", "omp", Some("provider-runtime".into()),
        ).with_ownership("owner", sequence);
        writer.observe(harness_state::Observation::new(
            harness_state::Activity::Active, harness_state::BlockedOn::Human,
            harness_state::InputBuffer::Unknown,
        )).unwrap();
        let native_id = fractal_schema::harness_questions::native_id_digest("PRIVATE_CALL_CANARY");
        let open = Observation::Open { native_id: native_id.clone(), session: "native-session".into(),
            questions: vec![Question { id: "q0".into(), options: 2, multi: false, required: true }] };
        harness_events::write_question(root.path(), "owner", &open).unwrap();
        let close = Observation::Closed { native_id, session: "native-session".into(),
            status: CloseStatus::AnsweredElsewhere };
        harness_events::write_question(root.path(), "owner", &close).unwrap();
        drop(writer);
        harness_events::enable(root.path(), "runtime-one").unwrap();
        let events = harness_events::pending(root.path(), 100).unwrap();
        let questions: Vec<_> = events.iter().filter(|event| event.kind == "harness-question").collect();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].runtime_incarnation, "runtime-one");
        assert_eq!(questions[0].payload["event"]["type"], "question.open");
        assert_eq!(questions[1].payload["event"]["type"], "question.closed");
        for event in &questions {
            let raw = serde_json::to_string(&event.payload).unwrap();
            for forbidden in ["PRIVATE_CALL_CANARY", "PRIVATE_QUESTION_CANARY", "PRIVATE_OPTION_CANARY",
                "\"question\":", "\"label\":", "\"answers\":"] {
                assert!(!raw.contains(forbidden));
            }
        }
        assert!(harness_events::write_question(root.path(), "foreign-owner", &open).is_err());
        assert_eq!(harness_events::pending(root.path(), 100).unwrap().len(), events.len());
        harness_events::acknowledge(root.path(), questions[1].sequence).unwrap();
        assert!(harness_events::pending(root.path(), 100).unwrap().is_empty());
    }
}
