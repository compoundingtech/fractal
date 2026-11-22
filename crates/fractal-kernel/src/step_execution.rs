//! Historical agent invocations of a generic mission step, independent of its product.
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Exact durable launch/turn receipt identity; absent on historical unbound claims.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
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
    /// Known priced portion; either unpriced counter prevents treating this as the full bill.
    pub cost_microusd: u64,
    pub reported_cost_microusd: u64,
    pub unpriced_tokens: u64,
    /// Responses without a price, including those whose total token count was not reported.
    pub unpriced_responses: u64,
}

/// Declared waits clipped to one physical execution interval. A wait automatically ends
/// at its declared deadline, so a missing resume cannot stop execution accounting forever.
pub fn execution_wait<'a>(
    events: impl Iterator<Item = (&'a str, &'a Value, u128)>,
    attempt: u32,
    start: u128,
    end: u128,
) -> (u128, Option<u128>) {
    let mut paused: Option<(u128, u128)> = None;
    let mut waited = 0_u128;
    for (kind, body, at) in events {
        if at > end { continue; }
        let fields = body.get("fields").unwrap_or(body);
        if fields["attempt"].as_u64() != Some(u64::from(attempt)) { continue; }
        if let Some((began, deadline)) = paused
            && at >= deadline
        {
            waited = waited.saturating_add(deadline.saturating_sub(began.max(start)));
            paused = None;
        }
        let clock = (kind == "work.progress").then(|| fields["clock"].as_str()).flatten();
        let closes = matches!(kind, "work.claimed" | "work.submitted" | "work.failed" | "work.released")
            || clock == Some("running") || clock == Some("paused");
        if closes && let Some((began, deadline)) = paused.take() {
            waited = waited.saturating_add(at.min(deadline).saturating_sub(began.max(start)));
        }
        if clock == Some("paused")
            && let Some(deadline) = fields["wait_until_unix_ms"].as_u64().map(u128::from)
            && deadline > at
        {
            paused = Some((at, deadline));
        }
    }
    if let Some((began, deadline)) = paused {
        waited = waited.saturating_add(end.min(deadline).saturating_sub(began.max(start)));
    }
    (waited, paused.filter(|(_, deadline)| *deadline > end).map(|(_, deadline)| deadline))
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
