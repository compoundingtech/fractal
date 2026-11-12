//! Person read positions: the reads and the forward-only advance over `store::read_positions`.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use fractal_schema::read_positions as schema;
use fractal_store::store::read_positions as projection;

/// What an advance did: the claim it appended, if it moved the position forward, and the stored
/// position afterwards in Unix milliseconds.
#[derive(Clone, Debug)]
pub struct ReadPositionAdvance {
    pub claim: Option<ClaimRecord>,
    pub at_ms: u64,
}

pub trait ReadPositionsStore {
    /// `person`'s positions ordered by conversation, inside the caller's read snapshot `through`.
    fn read_positions(&self, person: &str, through: u64) -> Result<Vec<Value>>;
    /// The largest local change index of `person`'s positions (or a later repair): a window
    /// pins it, so a change outside its row limit still sends a frame.
    fn read_position_revision(&self, person: &str) -> Result<u64>;
    /// Whether any position changed in `(after, through]`, including a repair.
    fn read_positions_changed(&self, after: u64, through: u64) -> Result<bool>;
    /// Append `read-position.advanced` only when `at_ms` is beyond the stored position; an equal
    /// or older advance completes without a claim. Concurrent advances on other hosts still
    /// converge, since every projection keeps the maximum.
    fn advance_read_position(
        &self,
        person: &str,
        conversation: &str,
        at_ms: u64,
        actor: &str,
        idempotency_key: String,
    ) -> Result<ReadPositionAdvance, FractalError>;
}

impl ReadPositionsStore for Store {
    fn read_positions(&self, person: &str, through: u64) -> Result<Vec<Value>> {
        self.read_snapshot(|_| projection::read_positions_at(&self.readers.get(), person, through))
    }

    fn read_position_revision(&self, person: &str) -> Result<u64> {
        self.read_snapshot(|_| projection::revision(&self.readers.get(), person))
    }

    fn read_positions_changed(&self, after: u64, through: u64) -> Result<bool> {
        projection::changed(&self.readers.get(), after, through)
    }

    fn advance_read_position(
        &self,
        person: &str,
        conversation: &str,
        at_ms: u64,
        actor: &str,
        idempotency_key: String,
    ) -> Result<ReadPositionAdvance, FractalError> {
        let stored = || projection::position_at_ms(&self.readers.get(), person, conversation).map_err(internal);
        if let Some(current) = stored()?
            && current >= at_ms
        {
            return Ok(ReadPositionAdvance { claim: None, at_ms: current });
        }
        let claim = self.append_claim(&ClaimInput {
            subject: schema::subject(person),
            kind: schema::KIND.into(),
            actor: Some(actor.into()),
            fields: BTreeMap::from([
                ("owner".into(), json!(person)),
                ("conversation".into(), json!(conversation)),
                ("at_ms".into(), json!(at_ms)),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some(idempotency_key),
        })?;
        let at_ms = stored()?.map_or(at_ms, |stored| stored.max(at_ms));
        Ok(ReadPositionAdvance { claim: Some(claim), at_ms })
    }
}
