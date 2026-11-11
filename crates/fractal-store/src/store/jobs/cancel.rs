//! History, not stopping authority. The inactive daemon authenticates each audit
//! before recording it; a replicated record must never execute a stop.
use super::*;
use fractal_jobs::cancel::resolve;

pub(super) fn apply(
    tx: &Transaction<'_>, id: &str, at: &str, record: &JobRecord,
    current: &mut Option<(JobHostBinding, JobState)>,
) -> Result<()> {
    let (existing, state) = current.as_mut().context("job-not-submitted")?;
    let (binding, intent, audit) = match record {
        JobRecord::StopStarted { binding, intent, audit }
        | JobRecord::StopResolved { binding, intent, audit, .. } => (binding, intent, audit),
        _ => unreachable!(),
    };
    ensure!(binding == existing, "job-binding-conflict");
    let (recorded_intent, _) = recovery::recorded(tx, id)?;
    ensure!(recorded_intent.as_ref() == Some(intent), "job-launch-intent-mismatch");
    let requester: String = tx.query_row("SELECT requester FROM jobs WHERE id=?1", [id], |r| r.get(0))?;
    ensure!(audit.validate(&requester).is_ok(), "job-stop-unauthorized");
    match record {
        JobRecord::StopStarted { .. } => {
            ensure!(matches!(*state, JobState::CancelRequested | JobState::RecoveringCancelRequested),
                "job-stop-not-requested");
        }
        JobRecord::StopResolved { resolution, .. } => {
            let started: String = tx.query_row("SELECT event FROM job_history WHERE job_id=?1
                AND refusal IS NULL AND json_extract(event,'$.action')='stop-started'
                ORDER BY ordinal DESC LIMIT 1", [id], |r| r.get(0))
                .optional()?.context("job-stop-audit-required")?;
            let JobRecord::StopStarted { intent: expected, audit: expected_audit, .. } =
                serde_json::from_str(&started)? else { unreachable!() };
            ensure!(expected == *intent && expected_audit == *audit, "job-stop-audit-mismatch");
            let next = resolve(*state, resolution)?;
            tx.execute("UPDATE jobs SET state=?2,queued=0,updated_at=?3 WHERE id=?1",
                params![id, serde_json::to_string(&next)?, at])?;
            *state = next;
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::{cancel::{StopAudit, StopResolution, StopUncertain}, launch::{ExecutionGeneration, LaunchIntent},
        recovery::RecoveryUsage};
    use std::num::NonZeroU64;
    #[test]
    fn cancellation_audit_uncertainty_and_sample_survive_replay() {
        let mut db = super::super::tests::database();
        let tx = db.transaction().unwrap();
        let id = job_id("person/fixture", "cancel");
        let submitted = super::super::tests::submission(id.as_str(), "cancel");
        let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
        let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: "/receipt".into() };
        let audit = StopAudit::Requester { principal: "person/fixture".into() };
        let usage = RecoveryUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished);
        let records = [
            submitted.clone(),
            JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() },
            JobRecord::Transition { binding: binding.clone(), event: Event::CancelRequest },
            // A raw acknowledgement cannot bypass retained usage and audit.
            JobRecord::Transition { binding: binding.clone(), event: Event::WholeTreeStopped },
            JobRecord::StopResolved { binding: binding.clone(), intent: intent.clone(), audit: audit.clone(),
                resolution: StopResolution::WholeTreeStopped { usage: usage.clone() } },
            JobRecord::StopStarted { binding: binding.clone(), intent: intent.clone(), audit: audit.clone() },
            JobRecord::StopResolved { binding: binding.clone(), intent: intent.clone(), audit: audit.clone(),
                resolution: StopResolution::StopUncertain { reason: StopUncertain::EmptyTimeout } },
            JobRecord::StopResolved { binding: binding.clone(), intent: intent.clone(), audit: audit.clone(),
                resolution: StopResolution::WholeTreeStopped { usage: usage.clone() } },
            JobRecord::StopResolved { binding: binding.clone(), intent: intent.clone(), audit,
                resolution: StopResolution::WholeTreeStopped { usage } },
        ];
        for (index, record) in records.iter().enumerate() {
            super::super::tests::insert(&tx, id.as_str(), &format!("cancel-{index}"), index as u64, record);
        }
        flush(&tx).unwrap();
        rebuild(&tx).unwrap();
        assert_eq!(job_by_id(&tx, id.as_str()).unwrap().unwrap().state, JobState::Cancelled);
        let history = history_by_job(&tx, id.as_str()).unwrap();
        assert!(history[3].refusal.as_ref().unwrap().contains("job-stop-proof-required"));
        assert!(history[4].refusal.as_ref().unwrap().contains("job-stop-audit-required"));
        assert_eq!(history[6].state, Some(JobState::CancelRequested));
        assert!(history[7..].iter().all(|row| row.refusal.is_none()));
        let encoded = serde_json::to_string(records.last().unwrap()).unwrap();
        assert!(encoded.contains("cgroup-vanished"));
    }
}
