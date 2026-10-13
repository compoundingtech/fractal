//! Gate seats are declared with a round, but readiness alone authorizes their start.
use super::*;

impl<R: RuntimeControl> Reconciler<R> {
    pub(super) fn is_loop_gate(&self, subject: &DesiredSubject) -> Result<bool> {
        self.store.is_loop_gate(subject)
    }

    pub(super) fn idle_loop_gate_count(&self, desired: &[DesiredSubject]) -> Result<u64> {
        let mut idle = 0;
        for subject in desired {
            if subject.member.as_ref().is_none_or(|member| member.host != self.host)
                || !self.is_loop_gate(subject)?
            {
                continue;
            }
            let running = self.store.latest_actual_value(&subject.subject)?.is_some_and(|actual| {
                actual_field(&actual, "status").and_then(Value::as_str) == Some("running")
            });
            if running && self.store.current_harness(&subject.subject)?.is_some_and(|harness| {
                harness.state == "idle"
            }) {
                idle += 1;
            }
        }
        Ok(idle)
    }

    pub(super) fn record_loop_gate_metrics(&self, desired: &[DesiredSubject]) -> Result<()> {
        crate::reconcile_telemetry::record_idle_loop_gates(self.idle_loop_gate_count(desired)?);
        Ok(())
    }
}
