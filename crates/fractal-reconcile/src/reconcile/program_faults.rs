//! A program step's faults (docs/st3/program-engine.md §8), decided while its run is evaluated.
//!
//! - `program/stalled`: a running program step whose journal shows no new entry within its
//!   `stalled-after`. The evaluation arms the deadline (`note_due`) instead of scanning; every
//!   journal entry is a claim on the step, which its run's evaluation read, so an entry moves the
//!   deadline and closes the fault.
//! - `program/host`: the engine reports that no host could run the step, or that its worker
//!   missed the liveness deadline (DQ8). A step whose host cannot start keeps its state and is
//!   dispatched again at a deadline; the engine wakes the step when it fails it or cannot resume
//!   it after a host exit.
//!
//! Both are faults on the step subject, closed when the cause ends or by the inactive sweep.
use super::*;
use crate::program_journal::{self, HOST_FAULT_SCOPE, STALLED_FAULT_SCOPE};
use anyhow::anyhow;

/// When a step whose host could not start is dispatched again.
pub const HOST_RETRY_MS: u128 = 30_000;

impl<R: RuntimeControl> Reconciler<R> {
    /// Record or close the program faults of one program step at `now`.
    pub(crate) fn reconcile_program_faults(
        &self,
        view: &crate::model::StepRunView,
        program: &crate::model::ProgramSpec,
        now: u128,
    ) -> Result<()> {
        let host = self.program_engine.get().and_then(|engine| engine.host_fault(&view.subject, view.attempt));
        let running = view.status == "working";
        if let Some(reason) = host {
            self.record_fault(&view.subject, HOST_FAULT_SCOPE, Err(anyhow!(reason)))?;
            // A step whose host could not start still waits to run: try it again then.
            if running {
                smallclaims::touched::note_due(now + HOST_RETRY_MS);
            }
            return self.close_fault(&view.subject, STALLED_FAULT_SCOPE, "the program host fault names why");
        }
        self.close_fault(&view.subject, HOST_FAULT_SCOPE, "the program host runs it")?;
        if !running {
            return self.close_fault(&view.subject, STALLED_FAULT_SCOPE, "the program step is not running");
        }
        let journal = self.store.program_journal(&view.subject, view.attempt, program_journal::SUMMARY_ENTRIES)?;
        let since = [journal.last_entry_at_unix_ms, view.execution_started_at_unix_ms]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(view.updated_at_unix_ms);
        let limit = program_journal::stalled_after_ms(program);
        let stalls_at = since + u128::from(limit);
        if now < stalls_at {
            smallclaims::touched::note_due(stalls_at);
            return self.close_fault(&view.subject, STALLED_FAULT_SCOPE, "the program's journal moved");
        }
        // The reason names no age, so a fault that lasts is recorded once.
        let open = match journal.innermost() {
            Some(entry) => match &entry.child {
                Some(child) => format!("open key `{}` waits on {child}", entry.key),
                None => format!("open key `{}`", entry.key),
            },
            None => "no open key".to_owned(),
        };
        self.record_fault(
            &view.subject,
            STALLED_FAULT_SCOPE,
            Err(anyhow!(
                "program step `{}` wrote no journal entry within its stalled-after of {}; {open}",
                view.step,
                crate::waiting_on::describe_age(u128::from(limit))
            )),
        )
    }
}
