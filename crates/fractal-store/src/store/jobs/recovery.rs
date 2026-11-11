//! History only. Authentication, open-window proof and filesystem observations
//! belong to the inactive trusted recovery adapter, never to this projection.
use super::*;
use fractal_jobs::{launch::LaunchIntent, recovery::{Released, Resolution, resolve}};

#[derive(Clone, Debug)]
pub struct RecoveryFacts {
    pub job: JobRow,
    pub intent: Option<LaunchIntent>,
    pub released: Option<Released>,
}

pub(super) fn recorded(tx: &Transaction<'_>, id: &str) -> Result<(Option<LaunchIntent>, Option<Released>)> {
    let mut statement = tx.prepare("SELECT event FROM job_history WHERE job_id=?1
        AND refusal IS NULL AND json_extract(event,'$.action') IN ('launch-intent','launch-released')
        ORDER BY ordinal")?;
    let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
    let (mut intent, mut released) = (None, None);
    for row in rows {
        match serde_json::from_str::<JobRecord>(&row?)? {
            JobRecord::LaunchIntent { intent: value, .. } => intent = Some(value),
            JobRecord::LaunchReleased { released: value, .. } => released = Some(value),
            _ => unreachable!(),
        }
    }
    Ok((intent, released))
}

/// Same transaction and staleness fence as job_by_id. Historical release facts
/// do not prove current authority; authenticate again before recovery side effects.
pub fn recovery_facts(tx: &Transaction<'_>, id: &str) -> Result<Option<RecoveryFacts>> {
    let Some(job) = job_by_id(tx, id)? else { return Ok(None); };
    let (intent, released) = recorded(tx, id)?;
    Ok(Some(RecoveryFacts { job, intent, released }))
}

pub(super) fn apply(
    tx: &Transaction<'_>, id: &str, at: &str, record: &JobRecord,
    current: &mut Option<(JobHostBinding, JobState)>,
) -> Result<()> {
    let (existing, state) = current.as_mut().context("job-not-submitted")?;
    let (expected_intent, expected_release) = recorded(tx, id)?;
    let next = match record {
        JobRecord::LaunchReleased { binding, intent, released } => {
            ensure!(binding == existing, "job-binding-conflict");
            ensure!(expected_intent.as_ref() == Some(intent), "job-launch-intent-mismatch");
            ensure!(expected_release.is_none(), "job-release-already-recorded");
            ensure!(!released.identity.boot_id.is_empty(), "job-release-boot-identity-required");
            ensure!(released.holder_pid > 0 && !released.pin.member_key.is_empty(), "job-release-identity-required");
            ensure!(released.pin.host == binding.execution_host
                && released.pin.admitted_claim == binding.authority_declaration, "job-recovery-pin-mismatch");
            ensure!(*state == JobState::Launching, "job-release-not-launching");
            transition(*state, Event::Started)?
        }
        JobRecord::Recovered { binding, intent, pin, resolution, usage } => {
            ensure!(binding == existing, "job-binding-conflict");
            ensure!(*intent == expected_intent, "job-launch-intent-mismatch");
            ensure!(matches!(resolution, Resolution::KeepRunning) || intent.is_none()
                || usage.is_some(), "job-recovery-sample-required");
            ensure!(pin.host == binding.execution_host && !pin.member_key.is_empty()
                && pin.admitted_claim == binding.authority_declaration, "job-recovery-pin-mismatch");
            if let Some(released) = &expected_release {
                ensure!(*pin == released.pin, "job-recovery-pin-mismatch");
            }
            if !matches!(resolution, Resolution::UnknownAfterHostLoss) {
                ensure!(expected_release.is_some(), "job-release-proof-required");
            }
            resolve(*state, resolution)?
        }
        _ => unreachable!(),
    };
    tx.execute("UPDATE jobs SET state=?2,queued=0,updated_at=?3 WHERE id=?1",
        params![id, serde_json::to_string(&next)?, at])?;
    *state = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_jobs::{launch::ExecutionGeneration, recovery::RecoveryPin};
    use std::num::NonZeroU64;
    #[test]
    fn release_and_recovery_replay_keep_admission_and_reject_a_second_launch() {
        let mut db = super::super::tests::database();
        let tx = db.transaction().unwrap();
        let id = job_id("person/fixture", "recovery");
        let submitted = super::super::tests::submission(id.as_str(), "recovery");
        let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
        let pin = RecoveryPin { host: binding.execution_host.clone(), member_key: "same-key".into(), admitted_claim: binding.authority_declaration.clone() };
        let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: "/receipt".into() };
        let released = Released { holder_pid: 42, pin: pin.clone(),
            identity: fractal_jobs::recovery::ProcessIdentity { boot_id: "boot".into(), start_ticks: 7 } };
        for (index, record) in [submitted.clone(), JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() },
            JobRecord::LaunchReleased { binding: binding.clone(), intent: intent.clone(), released: released.clone() },
            JobRecord::Recovered { binding: binding.clone(), intent: Some(intent.clone()), pin: pin.clone(), resolution: Resolution::KeepRunning, usage: None },
            JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() }].into_iter().enumerate()
        {
            super::super::tests::insert(&tx, id.as_str(), &format!("replay-{index}"), index as u64, &record);
        }
        flush(&tx).unwrap();
        rebuild(&tx).unwrap();
        let facts = recovery_facts(&tx, id.as_str()).unwrap().unwrap();
        assert_eq!(facts.job.binding, *binding);
        assert_eq!(facts.job.state, JobState::Running);
        assert_eq!(facts.intent, Some(intent));
        assert_eq!(facts.released, Some(released));
        assert!(history_by_job(&tx, id.as_str()).unwrap().last().unwrap().refusal.is_some());
    }
    #[test]
    fn uncertain_generation_cannot_claim_release_or_replace_its_intent() {
        let mut db = super::super::tests::database();
        let tx = db.transaction().unwrap();
        let id = job_id("person/fixture", "uncertain");
        let submitted = super::super::tests::submission(id.as_str(), "uncertain");
        let JobRecord::Submitted { binding, .. } = &submitted else { unreachable!() };
        let pin = RecoveryPin { host: binding.execution_host.clone(), member_key: "key".into(), admitted_claim: binding.authority_declaration.clone() };
        let intent = LaunchIntent { generation: ExecutionGeneration(NonZeroU64::new(1).unwrap()), outcome_file: "/receipt".into() };
        let records = [
            submitted.clone(),
            JobRecord::LaunchIntent { binding: binding.clone(), intent: intent.clone() },
            JobRecord::Recovered { binding: binding.clone(), intent: Some(intent.clone()), pin: pin.clone(), resolution: Resolution::KeepRunning, usage: None },
            JobRecord::Recovered { binding: binding.clone(), intent: None, pin: pin.clone(), resolution: Resolution::UnknownAfterHostLoss, usage: None },
            JobRecord::Recovered { binding: binding.clone(), intent: Some(intent), pin, resolution: Resolution::UnknownAfterHostLoss,
                usage: Some(fractal_jobs::recovery::RecoveryUsage::CountersUnavailable(fractal_jobs::launch::CountersUnavailable::CgroupVanished)) },
        ];
        for (index, record) in records.into_iter().enumerate() {
            super::super::tests::insert(&tx, id.as_str(), &format!("uncertain-{index}"), index as u64, &record);
        }
        flush(&tx).unwrap();
        let history = history_by_job(&tx, id.as_str()).unwrap();
        assert!(history[2].refusal.as_ref().unwrap().contains("release-proof-required"));
        assert!(history[3].refusal.as_ref().unwrap().contains("intent-mismatch"));
        assert_eq!(job_by_id(&tx, id.as_str()).unwrap().unwrap().state,
            JobState::UnknownAfterHostLoss { cancel_requested: false });
    }
}
