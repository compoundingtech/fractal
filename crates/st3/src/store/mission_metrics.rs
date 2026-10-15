//! Bounded read-only catch-up over newly committed local mission outcomes.
use super::*;

#[cfg(test)]
#[path = "mission_metrics_tests.rs"]
mod tests;

pub(crate) const COMPLETION_BATCH: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MissionOutcome { Succeeded, Failed, Cancelled }

impl MissionOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self { Self::Succeeded => "succeeded", Self::Failed => "failed", Self::Cancelled => "cancelled" }
    }
}

#[derive(Debug)]
pub(crate) struct MissionCompletion {
    pub seconds: f64,
    pub outcome: MissionOutcome,
}

pub(crate) const WAIT_REASONS: [&str; 3] = ["human", "dependency", "infrastructure"];

#[derive(Clone, Debug, Default)]
pub(crate) struct MissionWaitMetrics {
    pub counts: [u64; 3],
    pub oldest_seconds: [f64; 3],
    pub unknown: u64,
}

pub(crate) struct MissionWaitEvidence<'a> {
    pub state: &'a str,
    pub person: bool,
    pub reason: i64,
    pub activated: Option<i64>,
    pub state_since: i64,
    pub pause_started: Option<i64>,
    pub paused_until: Option<i64>,
    pub clock_known: bool,
    pub agentless: bool,
}

impl MissionWaitEvidence<'_> {
    pub(crate) fn observe(self, metrics: &mut MissionWaitMetrics, now: i64) {
        let waiting = match self.state {
            "waiting-person" => Some((0, self.state_since)),
            "ready" | "pending" if self.person => Some((0, self.activated.unwrap_or(self.state_since))),
            "pending" => Some((1, self.state_since)),
            "blocked" if self.reason == 1 =>
                Some((1, self.state_since)),
            "blocked" | "orphaned" => Some((2, self.state_since)),
            "working" if self.agentless && self.reason == 2 =>
                Some((1, self.state_since)),
            "claimed" | "working" if !self.clock_known => {
                metrics.unknown += 1;
                None
            }
            "claimed" | "working" if self.paused_until.is_some_and(|until| until >= now) => {
                if let Some(since) = self.pause_started { Some((1, since)) }
                else { metrics.unknown += 1; None }
            }
            _ => None,
        };
        if let Some((bucket, since)) = waiting {
            metrics.counts[bucket] += 1;
            metrics.oldest_seconds[bucket] = metrics.oldest_seconds[bucket]
                .max(now.saturating_sub(since).max(0) as f64 / 1000.0);
        }
    }
}

impl Store {
    /// The first tick establishes a frontier: process metrics do not backfill history.
    /// Subsequent ticks consume at most one batch and never skip its unconsumed tail.
    pub(crate) fn mission_completion_batch(&self, after: Option<u64>) -> Result<(u64, Vec<MissionCompletion>)> {
        let connection = self.readers.get();
        let Some(mut cursor) = after else { return Ok((current_index(&connection)?, Vec::new())); };
        let mut statement = connection.prepare_cached(
            "SELECT c.store_index, c.origin, json_extract(c.body,'$.fields.status'),
                    json_extract(c.body,'$.fields.phase'), c.accepted_at_unix_ms,
                    r.created_at_unix_ms
             FROM claims c INDEXED BY claims_kind_index
             LEFT JOIN mission_runs r ON r.id=substr(c.subject,13)
             WHERE c.kind='mission-run.state' AND c.store_index>?1
             ORDER BY c.store_index LIMIT ?2"
        )?;
        let mut rows = statement.query(params![cursor, COMPLETION_BATCH])?;
        let mut samples = Vec::new();
        while let Some(row) = rows.next()? {
            cursor = row.get(0)?;
            let origin = row.get_ref(1)?.as_str()?;
            let state = row.get_ref(2)?.as_str_or_null()?;
            let phase = row.get_ref(3)?.as_str_or_null()?;
            if origin != self.origin || phase != Some("terminal") { continue; }
            let outcome = match state {
                Some("completed") => MissionOutcome::Succeeded,
                Some("failed") => MissionOutcome::Failed,
                Some("cancelled") => MissionOutcome::Cancelled,
                _ => continue,
            };
            let accepted = row.get_ref(4)?.as_str()?;
            let created = row.get_ref(5)?.as_str_or_null()?;
            // Do not advance the caller's cursor if the projection has not caught up.
            let created = created.context("terminal mission metric is missing its run projection")?;
            samples.push(MissionCompletion {
                seconds: accepted.parse::<u128>()?.saturating_sub(created.parse::<u128>()?) as f64 / 1000.0,
                outcome,
            });
        }
        Ok((cursor, samples))
    }
}
