//! The native settlement boundary shared by terminal authority and client-v0.
use super::*;
pub use crate::store::harness_questions::HarnessQuestion;
use fractal_schema::harness_questions::Answer;

/// A native adapter's capability refusal. No adapter is installed by default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HarnessQuestionAdapterError {
    Missing,
    Unavailable,
}

/// Settles the exact native call, never terminal text. The harness owns timeout bits.
pub trait HarnessQuestionSettleSink: Send + Sync {
    fn availability(&self, question: &HarnessQuestion) -> Result<(), HarnessQuestionAdapterError>;
    fn settle(
        &self,
        question: &HarnessQuestion,
        answers: &BTreeMap<String, Answer>,
    ) -> Result<BTreeMap<String, Answer>, HarnessQuestionAdapterError>;
}

pub(super) fn capability(state: &AppState, session: &client_v0::ClientSession) -> Value {
    let person = !session.actor.starts_with("agent/")
        && session.authority_actor.starts_with("person/")
        && session.authority_actor.matches('/').count() == 1;
    let availability = if state.harness_question_sink.is_none() {
        "unavailable"
    } else if person && session.allows("terminal.control") {
        "granted"
    } else {
        "ungranted"
    };
    json!({"id":"harness.answer", "version":0, "state":availability})
}

/// Terminal input and native answers share selected live local-runtime authority.
pub fn live_input_session(
    state: &AppState,
    subject: &str,
    incarnation: &str,
) -> Result<LiveSession, ApiError> {
    let session = live_session(state, subject, Some(incarnation))?;
    if !session.terminal {
        return Err(ApiError::bad(St3Error::new(
            "unsupported-capability",
            "session input requires a terminal session",
        )));
    }
    Ok(session)
}
