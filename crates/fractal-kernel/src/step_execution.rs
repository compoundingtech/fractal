//! Historical agent invocations of a generic mission step, independent of its product.
use serde::{Deserialize, Serialize};

/// One durable work claim, including invocations that failed before reporting usage.
/// Optional metadata is evidence, not a reconstruction from the seat's current declaration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepExecution {
    /// The immutable `work.claimed` claim ID (also the invocation's accounting key).
    pub id: String,
    pub step_subject: String,
    pub attempt: u32,
    pub agent: String,
    pub incarnation_id: Option<String>,
    pub declared_model: Option<String>,
    pub observed_models: Vec<String>,
    /// Only explicit fallback evidence; a model mismatch is not a fallback cause.
    pub fallback: Option<bool>,
    pub started_at_unix_ms: u128,
    pub ended_at_unix_ms: Option<u128>,
    pub duration_ms: Option<u128>,
    pub status: Option<String>,
    pub trace_id: Option<String>,
    /// Runtime claim ID carrying native `exit_evidence`, never a synthetic exit code.
    pub exit_evidence: Option<String>,
    /// `None` means no response usage evidence, not a free invocation.
    pub usage: Option<StepExecutionUsage>,
}

/// Response totals priced by the same generic accounting path as fleet spend.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepExecutionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_write_1h_tokens: u64,
    pub cached_tokens: u64,
    pub total_tokens: u64,
    /// Known priced portion; `unpriced_tokens` prevents treating this as the full bill.
    pub cost_microusd: u64,
    pub reported_cost_microusd: u64,
    pub unpriced_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_historical_metadata_stays_unknown() {
        let execution: StepExecution = serde_json::from_str(r#"{"id":"claim/old","attempt":1}"#).unwrap();
        assert_eq!(execution.declared_model, None);
        assert_eq!(execution.trace_id, None);
        assert_eq!(execution.fallback, None);
        assert_eq!(execution.usage, None);
        assert!(execution.observed_models.is_empty());
    }
}
