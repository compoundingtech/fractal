//! Canonical graph-free execution authority.
use fractal_wire::Error as FractalError;
use crate::internal;
pub mod local;
pub mod assertions;

/// Immutable execution identity admitted by the selected home. Run authority is separate from
/// the executing seat's identity: non-program seats carry no run, and a run is never an agent.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub version: u32,
    pub home: String,
    pub host: String,
    pub epoch: u64,
    pub subject: String,
    pub runtime: String,
    pub incarnation: String,
    pub authorization_revision: String,
    pub run: Option<RunBinding>,
}

impl ExecutionBinding {
    pub fn validate(&self) -> Result<(), FractalError> {
        if self.version != 1
            || self.epoch == 0
            || self.epoch > i64::MAX as u64
            || self.home.is_empty()
            || self.host.is_empty()
            || self.subject.is_empty()
            || self.runtime.is_empty()
            || self.incarnation.is_empty()
            || self.authorization_revision.is_empty()
            || self.run.as_ref().is_some_and(|run| {
                !run.run.starts_with("mission-run/")
                    || run.step_run.is_empty()
                    || run.attempt == 0
            })
        {
            return Err(FractalError::new(
                "invalid-execution-binding",
                "execution authority requires version 1 and an exact admitted identity",
            ));
        }
        Ok(())
    }
}
/// The actor a run credential binds: `run` is `mission-run/RUN`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBinding {
    pub run: String,
    pub step_run: String,
    pub attempt: u32,
}
