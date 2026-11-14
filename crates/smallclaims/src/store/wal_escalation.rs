//! The daemon's WAL recycle, escalated when the WAL keeps growing. The recycle truncates only a
//! WAL a passive checkpoint found fully copied, and while a writer commits nearly all the time
//! with readers on older snapshots (a daemon's startup maintenance does) it never is, so the WAL
//! grows for as long as the writer runs. Past `WalEscalation::frames` for
//! `WalEscalation::samples` checkpoints and larger than when it passed, the recycle restarts the
//! WAL as a trim does (`Store::restart_wal_report`).

use super::*;
use crate::sqlite::WalCheckpointReport;

/// When the daemon's WAL recycle escalates to a restart.
#[derive(Clone, Copy, Debug)]
pub struct WalEscalation {
    /// Frames past which a WAL that keeps growing is restarted.
    pub frames: i32,
    /// Checkpoints since it passed `frames` without a recycle, after which a WAL larger than at
    /// the crossing is restarted.
    pub samples: u32,
    /// How long a restart waits, under the writer, for readers on older snapshots.
    pub busy: std::time::Duration,
    /// Frames a restart may still have to copy when it takes the writer: other writes go on while
    /// it copies, so with a reader open the copy never catches up entirely.
    pub remainder: i32,
}

/// 262,144 frames is 1 GiB of 4 KiB pages, four times a trim's own bound, so a trim never
/// escalates; twelve of the recycle's 5 s checkpoints is a minute of growth. 1,024 frames is
/// 4 MiB to copy under the writer at most.
pub const WAL_ESCALATION: WalEscalation = WalEscalation {
    frames: 262_144,
    samples: 12,
    busy: std::time::Duration::from_millis(200),
    remainder: 1_024,
};

/// Checkpoints since the WAL passed the escalation's frames without being recycled since, and
/// its frames when it passed. A writer that commits in bursts leaves some checkpoints between its
/// commits, so growth is measured from the crossing, not checkpoint to checkpoint. An errored
/// checkpoint neither counts nor resets.
#[derive(Debug, Default)]
pub struct WalGrowth {
    passed: Option<i32>,
    samples: u32,
    frames: i32,
}

impl WalGrowth {
    /// Count one recycle checkpoint, and say whether the WAL is due a restart: past the bound for
    /// the samples, and larger than when it passed. A WAL a reader holds steady is not.
    pub fn observe(&mut self, report: Option<&WalCheckpointReport>, limits: &WalEscalation) -> bool {
        if let Some(report) = report {
            if report.frames > limits.frames && !report.recycled {
                self.passed.get_or_insert(report.frames);
                self.samples += 1;
                self.frames = report.frames;
            } else {
                *self = Self::default();
            }
        }
        self.passed.is_some_and(|passed| self.samples >= limits.samples && self.frames > passed)
    }

    /// A restart that recycled starts the count again; one that did not is tried again at the
    /// next checkpoint.
    pub fn escalated(&mut self, restarted: bool) {
        if restarted {
            *self = Self::default();
        }
    }
}

impl Store {
    pub fn set_wal_escalation(&self, limits: WalEscalation) {
        *self.wal_escalation.lock().unwrap_or_else(PoisonError::into_inner) = limits;
    }

    /// One step of the daemon's WAL recycle: the idle recycle, and when `growth` says the WAL has
    /// kept growing past the escalation's bound, a restart. The restart's report comes second.
    pub fn recycle_wal(
        &self,
        checkpoint: &Connection,
        growth: &mut WalGrowth,
    ) -> Result<(WalCheckpointReport, Option<WalCheckpointReport>)> {
        let report = self.checkpoint_idle_wal_report(checkpoint)?;
        let limits = *self.wal_escalation.lock().unwrap_or_else(PoisonError::into_inner);
        if self.shared_memory || !growth.observe(Some(&report), &limits) {
            return Ok((report, None));
        }
        let restart = self.restart_wal_report(checkpoint, limits.busy, limits.remainder)?;
        growth.escalated(restart.recycled);
        crate::windows::wal::escalated(&restart);
        Ok((report, Some(restart)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(frames: i32, recycled: bool) -> WalCheckpointReport {
        WalCheckpointReport {
            frames,
            backfilled: frames / 2,
            passive_ms: 0,
            writer_wait_ms: 0,
            truncate_ms: None,
            recycled,
            blocked_by: None,
        }
    }

    #[test]
    fn growth_past_the_bound_for_the_samples_is_due_a_restart() {
        let limits = WalEscalation { frames: 100, samples: 3, ..WAL_ESCALATION };
        let mut growth = WalGrowth::default();
        assert!(!growth.observe(Some(&sample(50, false)), &limits), "under the bound");
        assert!(!growth.observe(Some(&sample(150, false)), &limits));
        // A burst writer between commits: the same frames twice is still growth since the crossing.
        assert!(!growth.observe(Some(&sample(150, false)), &limits));
        assert!(!growth.observe(None, &limits), "an errored checkpoint neither counts nor resets");
        assert!(growth.observe(Some(&sample(250, false)), &limits), "three past the bound and larger than at the crossing");
        growth.escalated(false);
        assert!(growth.observe(Some(&sample(300, false)), &limits), "a restart that did not recycle is tried again");
        growth.escalated(true);
        assert!(!growth.observe(Some(&sample(10, false)), &limits));
        // Past the bound but steady (a WAL a slow reader holds) is not a restart.
        let mut steady = WalGrowth::default();
        for _ in 0..10 {
            assert!(!steady.observe(Some(&sample(500, false)), &limits));
        }
        // A recycled checkpoint starts over.
        let mut recycled = WalGrowth::default();
        assert!(!recycled.observe(Some(&sample(150, false)), &limits));
        assert!(!recycled.observe(Some(&sample(200, false)), &limits));
        assert!(!recycled.observe(Some(&sample(300, true)), &limits));
        assert!(!recycled.observe(Some(&sample(400, false)), &limits));
        assert!(!recycled.observe(Some(&sample(500, false)), &limits));
        assert!(recycled.observe(Some(&sample(600, false)), &limits));
    }
}
