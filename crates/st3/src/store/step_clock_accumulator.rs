//! Raw event state: never settle a wall-clock snapshot into the persisted accumulator.
use super::*;

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Accumulator {
    started: Option<u128>,
    elapsed: u128,
    lease_expires: Option<u128>,
    paused_until: Option<u128>,
    pause_started: Option<u128>,
    waited: u128,
}

impl Accumulator {
    pub(super) fn apply(&mut self, kind: &str, body: &Value, accepted: u128, attempt: u32) {
        let fields = body.get("fields").unwrap_or(body);
        if kind.starts_with("work.")
            && fields.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        {
            return;
        }

        // A wait ends at its deadline whether or not anyone resumed.
        if let Some(deadline) = self.paused_until
            && accepted > deadline
        {
            self.started = Some(deadline);
            end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, deadline);
        }
        if let (Some(interval_start), Some(expiry)) = (self.started, self.lease_expires)
            && accepted > expiry
        {
            self.elapsed = self.elapsed.saturating_add(expiry.saturating_sub(interval_start));
            self.started = None;
            self.lease_expires = None;
        }

        match kind {
            "step-run.state"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "claimed" | "working")) =>
            {
                // Agentless execution has no `work.claimed` event. Its reconciler-owned
                // `working` transition is therefore the authoritative opening edge. Worker
                // claims may also carry this projection; keeping the earliest edge is
                // idempotent and their following work event supplies the lease. A declared
                // wait keeps the clock stopped.
                if self.started.is_none() && self.paused_until.is_none() {
                    self.started = Some(accepted);
                }
            }
            "work.claimed" => {
                // A new claim ends any wait the previous holder declared.
                end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
                if self.started.is_none() {
                    self.started = Some(accepted);
                }
                self.lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            // A claim carried into a successor generation opens its interval at the cutover.
            "step-run.carried"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "claimed" | "working"))
                    && fields
                        .get("claimant")
                        .is_some_and(|claimant| !claimant.is_null()) =>
            {
                end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
                if self.started.is_none() {
                    self.started = Some(accepted);
                }
                self.lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            "work.renewed" | "work.progress" | "work.extended"
                if self.started.is_some() || self.paused_until.is_some() =>
            {
                let clock = if kind == "work.progress" {
                    fields.get("clock").and_then(Value::as_str)
                } else {
                    None
                };
                let wait_until = fields
                    .get("wait_until_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from)
                    .filter(|deadline| *deadline > accepted);
                match (clock, wait_until) {
                    (Some("paused"), Some(deadline)) => {
                        // Declaring again while a wait runs spends the time so far and starts
                        // the next span, so chained declarations draw on one budget.
                        end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
                        self.pause_started = Some(accepted);
                        if let Some(interval_start) = self.started {
                            let interval_end =
                                self.lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                            self.elapsed = self.elapsed
                                .saturating_add(interval_end.saturating_sub(interval_start));
                        }
                        self.started = None;
                        self.paused_until = Some(deadline);
                    }
                    (Some("running"), _) if self.paused_until.is_some() => {
                        end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
                        self.started = Some(accepted);
                    }
                    _ => {}
                }
                self.lease_expires = fields
                    .get("claim_expires_at_unix_ms")
                    .and_then(Value::as_u64)
                    .map(u128::from);
            }
            "work.submitted" | "work.failed" | "work.released" => {
                if let Some(interval_start) = self.started {
                    let interval_end =
                        self.lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                    self.elapsed = self.elapsed.saturating_add(interval_end.saturating_sub(interval_start));
                }
                self.started = None;
                self.lease_expires = None;
                end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
            }
            "step-run.state"
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| !matches!(status, "claimed" | "working")) =>
            {
                if let Some(interval_start) = self.started {
                    let interval_end =
                        self.lease_expires.map_or(accepted, |expiry| accepted.min(expiry));
                    self.elapsed = self.elapsed.saturating_add(interval_end.saturating_sub(interval_start));
                }
                self.started = None;
                self.lease_expires = None;
                end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, accepted);
            }
            _ => {}
        }
    }

    pub(super) fn snapshot(mut self, snapshot_unix_ms: u128) -> ClockFold {
        if let Some(deadline) = self.paused_until
            && snapshot_unix_ms > deadline
        {
            self.started = Some(deadline);
            end_wait(&mut self.waited, &mut self.pause_started, &mut self.paused_until, deadline);
        }
        if let Some(began) = self.pause_started {
            self.waited = self.waited.saturating_add(snapshot_unix_ms.saturating_sub(began));
        }
        ClockFold {
            started: self.started,
            elapsed: self.elapsed,
            lease_expires: self.lease_expires,
            paused_until: self.paused_until,
            waited: self.waited,
        }
    }
}
