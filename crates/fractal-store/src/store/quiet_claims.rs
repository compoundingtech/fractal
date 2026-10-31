//! Quiet claims. A held step is alive while its holder makes progress: `work progress`, a wait,
//! a resume, an extension, an ask, or a new claim. The lease is not progress: a seat's driver
//! renews it whether or not the seat works. A held step with an open declared wait is not quiet.
//!
//! A held step with no open declared wait, quiet for its run's stalled-after (3 hours unless the
//! run sets one), raises a `step-quiet` fault to its holder. Thirty minutes later, still quiet, it
//! is stalled: it raises a `step-stalled` fault to the run's report-to, else its requester, and
//! shows as stalled. Both are operational failures on the step whose episode names the attempt
//! and the quiet period, so any progress ends them: the next quiet period is a new episode.

use super::*;

/// The fault a quiet held step raises to its holder.
pub const STEP_QUIET_CONDITION: &str = "step-quiet";
/// The fault a step still quiet after the grace raises to its run's report-to.
pub const STEP_STALLED_CONDITION: &str = "step-stalled";
/// How long a quiet step's holder has to answer its fault before the step is stalled.
pub const STALLED_GRACE_MS: u128 = 30 * 60_000;

/// When the holder of `subject`'s attempt last made progress, as of `snapshot_unix_ms`: its
/// newest claim, progress (waits and resumes included), extension or ended wait, or the step's
/// carry into this generation. Renewals are not progress.
pub fn quiet_since_at(
    connection: &Connection,
    subject: &str,
    attempt: u32,
    snapshot_unix_ms: u128,
) -> rusqlite::Result<Option<u128>> {
    let snapshot = snapshot_unix_ms.to_string();
    Ok(connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h880386559c79 owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT accepted_at_unix_ms FROM claims
             WHERE subject=?1
               AND kind IN ('work.claimed','work.progress','work.extended','work.wait-ended','step-run.carried')
               AND (kind='step-run.carried' OR json_extract(body, '$.fields.attempt')=?2)
               AND (length(accepted_at_unix_ms), accepted_at_unix_ms) <= (length(?3), ?3)
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        ))?
        .query_row(/* sqlite-large-value: allow-for-now lv-h0843c6ae4c5c owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ params![subject, attempt, snapshot], |row| row.get::<_, String>(0))
        .optional()?
        .and_then(|at| at.parse().ok()))
}

/// The episode of a quiet or stalled fault for one quiet period of one attempt.
pub fn quiet_episode(condition: &str, subject: &str, attempt: u32, since: u128) -> String {
    let key = format!("{condition}:{subject}:{attempt}:{since}");
    let digest = hex::encode(Sha256::digest(key.as_bytes()));
    format!("attention/{}", &digest[..32])
}

/// Whether a quiet or stalled fault with `episode` still holds for `step` as of `as_of`: the step
/// is held, has no open declared wait, and has made no progress since the fault's quiet period
/// began.
pub fn quiet_fault_current(
    connection: &Connection,
    step: &StepRunView,
    condition: &str,
    episode: &str,
    as_of: u128,
) -> rusqlite::Result<bool> {
    if !matches!(step.status.as_str(), "claimed" | "working")
        || step.claim_expires_at_unix_ms.is_none_or(|expiry| expiry <= as_of)
        || declared_waits::declared_wait_at(connection, &step.subject, step.attempt, as_of)?
            .is_some()
    {
        return Ok(false);
    }
    Ok(
        quiet_since_at(connection, &step.subject, step.attempt, as_of)?.is_some_and(|since| {
            quiet_episode(condition, &step.subject, step.attempt, since) == episode
        }),
    )
}

/// When a held step became stalled, if it is stalled now: its current quiet period raised a
/// `step-stalled` fault.
pub(super) fn enrich_stalled_at(
    connection: &Connection,
    view: &mut StepRunView,
    snapshot_unix_ms: u128,
) -> rusqlite::Result<()> {
    if !matches!(view.status.as_str(), "claimed" | "working") || view.waiting_on.is_some() {
        return Ok(());
    }
    view.stalled_at_unix_ms = stalled_at(connection, &view.subject, view.attempt, snapshot_unix_ms)?;
    Ok(())
}

/// When a held step with no open declared wait became stalled, if its current quiet period
/// raised a `step-stalled` fault.
pub(super) fn stalled_at(
    connection: &Connection,
    subject: &str,
    attempt: u32,
    snapshot_unix_ms: u128,
) -> rusqlite::Result<Option<u128>> {
    let snapshot = snapshot_unix_ms.to_string();
    let Some((episode, at)) = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-h3277c3220609 owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT json_extract(body, '$.fields.episode'), accepted_at_unix_ms FROM claims
             WHERE subject=?1 AND kind='operational.failure'
               AND json_extract(body, '$.fields.condition')=?2
               AND (length(accepted_at_unix_ms), accepted_at_unix_ms) <= (length(?3), ?3)
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        ))?
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hbb0950c5c3f6 owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ params![subject, STEP_STALLED_CONDITION, snapshot],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(since) = quiet_since_at(connection, subject, attempt, snapshot_unix_ms)? else {
        return Ok(None);
    };
    Ok((episode.as_deref()
        == Some(quiet_episode(STEP_STALLED_CONDITION, subject, attempt, since).as_str()))
    .then(|| at.parse().ok())
    .flatten())
}

impl Store {
    /// When the holder of a held step's attempt last made progress.
    pub fn quiet_since(&self, subject: &str, attempt: u32) -> Result<Option<u128>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        Ok(quiet_since_at(&connection, subject, attempt, now_ms())?)
    }

    /// When this daemon or a peer first raised the fault `episode` on `subject`, if it has: the
    /// stalled grace runs from the holder's fault, not from the deadline it was due at.
    pub fn quiet_fault_at(&self, subject: &str, episode: &str) -> Result<Option<u128>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        Ok(connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h17e9cc15e678 owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT accepted_at_unix_ms FROM claims
                 WHERE subject=?1 AND kind='operational.failure'
                   AND json_extract(body, '$.fields.episode')=?2
                 ORDER BY CANONICAL_ASC(claims) LIMIT 1",
            ))?
            .query_row(/* sqlite-large-value: allow-for-now lv-h2ba4633a62ab owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ params![subject, episode], |row| row.get::<_, String>(0))
            .optional()?
            .and_then(|at| at.parse().ok()))
    }

    /// Raise a quiet or stalled fault on a step. Nothing more is written while the same
    /// episode is pending.
    pub fn record_quiet_fault(
        &self,
        condition: &str,
        episode: &str,
        input: &AttentionRequest,
    ) -> Result<AttentionRequestView, St3Error> {
        self.record_failure(episode, input, condition)
    }

    /// The runs with a stalled step now, for the stalled-runs target: each pending stalled fault
    /// whose step is still stalled. It reads only stalled faults, by their condition.
    pub fn stalled_runs(&self, as_of: u128) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let faults = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-hae9b17afdb8c owner=fractal_store::store::quiet_claims TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT subject, json_extract(body, '$.fields.episode') FROM claims
                 WHERE kind='operational.failure'
                   AND json_extract(body, '$.fields.condition')=?1
                 ORDER BY CANONICAL_DESC(claims)",
            ))?
            .query_map([STEP_STALLED_CONDITION], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut runs = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for (subject, episode) in faults {
            if !seen.insert(subject.clone()) {
                continue;
            }
            let (Some(episode), Some(step)) = (episode, person_work::step(&connection, &subject)?)
            else {
                continue;
            };
            if quiet_fault_current(&connection, &step, STEP_STALLED_CONDITION, &episode, as_of)? {
                runs.insert(step.run.clone());
            }
        }
        Ok(runs)
    }
}
