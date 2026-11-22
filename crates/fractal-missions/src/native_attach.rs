//! Typed native attach admission, intentionally not exposed through a transport yet.
//! The selected native process proves possession before a paired person may approve.
use crate::PersonWorkStore;
use fractal_kernel::model::{FractalError, PersonAskRequest, StepRunView};
use fractal_store::store::{Store, native_attach::REQUESTER};

pub trait NativeAttachStore {
    /// Creates or recovers the exact native ask, then binds its immutable episode.
    /// Neither local CLI answers nor the respondent projection authorize commit.
    fn ask_native_attach(&self, preparation_id: &str) -> Result<StepRunView, FractalError>;
}

impl NativeAttachStore for Store {
    fn ask_native_attach(&self, preparation_id: &str) -> Result<StepRunView, FractalError> {
        let prepared = self.native_attach_preparation(preparation_id)?;
        let remaining = prepared.expires_at_unix_ms.saturating_sub(smallclaims::store::now_ms());
        if remaining == 0 {
            return Err(FractalError::new("native-attach-expired", "native attach preparation expired"));
        }
        let view = self.ask_person(&PersonAskRequest {
            legacy_request: None,
            person: prepared.candidate.person.clone(),
            title: "Approve this exact native OMP attachment".into(),
            reason: "Attach an existing session without restarting it or sending an initial message.".into(),
            actor: REQUESTER.into(),
            step: None,
            new_run: None,
            incarnation: None,
            request: Some(prepared.request()),
            default_answer: None,
            priority: None,
            idempotency_key: prepared.ask_key(),
            // Stable terms preserve ask idempotency. The preparation's absolute expiry,
            // independently enforced at commit, can never be extended by this ask.
            deadline_ms: Some(u64::try_from(prepared.expires_at_unix_ms - prepared.prepared_at_unix_ms).unwrap_or(u64::MAX)),
        })?;
        self.bind_native_attach_ask(preparation_id, &view.subject)?;
        Ok(view)
    }
}

#[cfg(test)]
mod tests;
