//! Authoritative execution-host evidence, captured before any destructive retirement.
use serde::{Deserialize, Serialize};
use super::{Coverage, EndReason, MemoryEvents, cgroup::IoDeviceStat};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Evidence {
    pub pre_retirement_populated: Option<bool>,
    pub learning_coverage: Coverage,
    pub memory_events: Option<MemoryEvents>,
    pub io_stat: Option<Vec<IoDeviceStat>>,
    pub retirement_generation: Option<String>,
    pub retirement_confirmed: bool,
    pub end_reason: EndReason,
}

pub(in crate::store) fn record_execution(
    transaction: &rusqlite::Transaction<'_>,
    execution: &super::super::shell_resources::Execution,
) -> anyhow::Result<Option<super::RecordOutcome>> {
    let request = &execution.command.lane;
    let (Some(key), Some(revision), Some(result)) = (
        request.profile.as_ref(),
        request.profile_estimate.as_ref().and_then(|estimate| estimate.seed_revision.as_ref()),
        execution.result.as_ref(),
    ) else { return Ok(None); };
    let evidence = &result.evidence;
    let Some(generation) = &evidence.retirement_generation else {
        return Ok(Some(super::RecordOutcome::Rejected(super::Rejection::MissingRetirementEvidence)));
    };
    super::record_retirement_tx(transaction, &super::RetirementSample {
        host: request.host.clone(), key: key.clone(), semantic_revision: revision.clone(),
        execution: execution.command.id.clone(), generation: generation.clone(),
        retirement_confirmed: evidence.retirement_confirmed,
        pre_retirement_populated: evidence.pre_retirement_populated,
        end_reason: if execution.cancel { EndReason::Cancelled }
            else if execution.timed_out { EndReason::TimedOut } else { evidence.end_reason },
        coverage: evidence.learning_coverage, memory_peak_bytes: result.memory_peak_bytes,
        memory_events: evidence.memory_events,
        // Shared filesystem deltas and block write traffic are not attributable allocation.
        disk_growth_bytes: None,
    }).map(Some)
}
