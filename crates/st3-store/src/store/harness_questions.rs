//! Questions a seat's harness asked natively: what st records of each (ids, option counts and
//! answer positions, never text), and the indefinitely retained compact record of how it ended.
//! `docs/st3/harness-questions.md` is the contract.
use super::*;
use smallclaims::store::checkpoint::claims_of_kind_in_order;
use st3_schema::harness_questions::{Answer, OBSERVED, Question, RESOLVED, Receipt, Status, actor_key, answers, questions};

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
    /// Canonical `sha256:` digest, never the raw native call identity.
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

pub fn field<T: for<'de> serde::Deserialize<'de>>(claim: &ClaimRecord, name: &str) -> Result<T> {
    serde_json::from_value(claim.body.pointer(&format!("/fields/{name}")).cloned().unwrap_or(Value::Null))
        .with_context(|| format!("{} `{}` has no valid {name}", claim.kind, claim.id))
}

/// The question `subject`: its newest observation and its first resolution in canonical order.
/// The compact record stays visible indefinitely; only redundant observation history expires.
pub fn question_at(
    connection: &Connection,
    subject: &str,
    _at: Option<u128>,
) -> Result<Option<HarnessQuestion>> {
    let Some(observed) = claims_of_kind_in_order(connection, subject, OBSERVED)?.pop() else {
        return Ok(None);
    };
    let resolution = match claims_of_kind_in_order(connection, subject, RESOLVED)?.first() {
        Some(claim) => {
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
