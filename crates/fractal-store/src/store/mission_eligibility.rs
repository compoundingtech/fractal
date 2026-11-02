//! A dependency-admitted mission step with no desired seat has one readiness-scoped fault.
use super::*;

pub const MISSING_AGENT_CONDITION: &str = "mission-no-eligible-agent";

pub fn episode(run: &MissionRunView, step: &StepRunView) -> String {
    format!(
        "{MISSING_AGENT_CONDITION}:{}:{}:{}",
        run.generation, step.step, step.readiness_epoch
    )
}

pub fn blocked(step: &StepRunView) -> bool {
    step.status == "blocked"
        && step
            .blocked_reason
            .as_deref()
            .is_some_and(|r| r.starts_with("no eligible agent is present"))
}

pub fn missing_binding_tx(connection: &Connection, step: &StepRunView) -> Result<bool> {
    let mut deliberate_stop = false;
    for name in step.assigned_to.iter().chain(step.available_to.iter()) {
        smallclaims::touched::note_read(|| name.clone());
        if let Some(row) = current_desired_row(connection, name)? {
            if row.kind == "agent" {
                return Ok(false);
            }
            deliberate_stop |=
                row.kind == "stop" && row.owner_run.is_none() && row.owner_generation.is_none();
        }
    }
    Ok(!deliberate_stop)
}

impl Store {
    pub fn missing_agent_fault_is_current(
        &self,
        run: &MissionRunView,
        step: &StepRunView,
    ) -> Result<bool> {
        if run.phase != "normal"
            || !matches!(run.status.as_str(), "running" | "standing" | "blocked")
            || step.generation != run.generation
            || !blocked(step)
        {
            return Ok(false);
        }
        missing_binding_tx(&self.readers.get(), step)
    }

}
