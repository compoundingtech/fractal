//! The part of `store::work_list` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_delivery::store::work_list::*;
pub use fractal_delivery::store::work_list::{ACTOR_ORDERS, ActorEntry, ActorOrders, OrderCell, SeatQueue, WorkRow, WorkRows, run_key, work_state_rank};

/// Every leased step's lease end, by the lease index.
pub const WORK_LEASES: &str = "SELECT subject, CAST(lease_expires_at_unix_ms AS INTEGER) FROM step_runs
 INDEXED BY step_runs_lease_index
 WHERE lease_owner IS NOT NULL AND lease_expires_at_unix_ms IS NOT NULL";

/// The seat each of steps `?1`, a JSON array, is assigned to, if any, and its run, whatever its
/// state or generation, by the primary key: a seat's queue order follows the runs it holds
/// steps in.
pub const STEP_ASSIGNEES: &str = "SELECT assignee, run_id FROM step_runs
 WHERE subject IN (SELECT value FROM json_each(?1))";

/// Whether a replica record was repaired by a claim after `?1` through `?2`, by the kind index.
pub const REPAIRED_SINCE: &str = "SELECT 1 FROM claims
 WHERE kind='record.repaired' AND store_index>?1 AND store_index<=?2 LIMIT 1";

/// The earliest claim about step `?1` accepted after `?2`, by the subject index.
pub const NEXT_CLAIM_AFTER: &str = "SELECT MIN(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
 WHERE subject=?1 AND CAST(accepted_at_unix_ms AS INTEGER)>?2";

/// What claims between two cuts changed in the work list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkChanges {
    /// Steps whose row any of those claims can change.
    pub steps: BTreeSet<String>,
    /// Whether a seat's queue moved, which reorders that seat's windows with no row changing.
    pub reorder: bool,
    /// The seats whose queue moved.
    pub moved_seats: BTreeSet<String>,
    /// The runs those claims name or reach, as run keys: a seat that read one rereads it.
    pub runs: BTreeSet<String>,
}

pub trait WorkListStore {
    /// Which steps' work rows the claims after `after` through `through` can change, read in
    /// the caller's snapshot. A row reads its step, the run, generation and root run that own
    /// it, revision proposals for that run, the person asks that block it, which their
    /// requesters' declarations and runtime decide, and the seats it owns with their usage. A
    /// claim about any other subject changes no row. `seats` names the step each seat shown on
    /// a row belonged to when that row was folded.
    fn work_list_changes(
            &self,
            after: u64,
            through: u64,
            seats: &HashMap<String, String>,
        ) -> Result<WorkChanges>;
    /// The steps whose worker lease ended after `after` and by `through`: their rows show them
    /// ready again with no new claim.
    fn steps_with_leases_ended(&self, after: u128, through: u128) -> Result<BTreeSet<String>>;
    /// The steps the current work list can show: those whose state is not waiting or ended.
    /// Their effective state can hide more of them, never show others.
    fn work_list_candidates(&self) -> Result<BTreeSet<String>>;
    /// Every step that holds a worker lease, with the lease's end: few, by the lease index.
    fn work_leases(&self) -> Result<HashMap<String, u128>>;
    /// When the earliest claim about `subject` accepted after `time` was accepted. Replicated
    /// claims keep their own times, so one can arrive before the list's time reaches it.
    fn next_claim_after(&self, subject: &str, time: u128) -> Result<Option<u128>>;
    /// The lease `subject`'s claims last named for the open execution interval of `attempt` at
    /// `at`, when one is open.
    fn step_timing_lease(&self, subject: &str, attempt: u32, at: u128) -> Result<Option<u128>>;
    /// The seats `steps` are assigned to and the runs they are in, whatever their state or
    /// generation, read in the caller's snapshot. Every step's run, with or without a seat.
    fn step_assignees(&self, steps: &BTreeSet<String>) -> Result<(BTreeSet<String>, BTreeSet<String>)>;
    /// Each of `seats`' queue order and the runs it read, in the caller's snapshot, from the same
    /// inputs [`Store::seat_run_order`] reads. A seat with no queued run and no move is left
    /// out. One read of every seat's inputs serves them all: per seat, the moves query scans
    /// every move claim by kind and the joins every open run anyway.
    fn seat_queues_of(&self, seats: &BTreeSet<String>) -> Result<HashMap<String, SeatQueue>>;
    /// Whether a replica record was repaired by a claim after `after` through `through`, read in
    /// the caller's snapshot. A repair applied with its claim, as `apply` does, drops a seat's
    /// move with no claim about the seat and no forget; the other routes forget when the record
    /// flips. Only claims newer than `after` count, so one repair folds the list from nothing
    /// once, however far a lagging frontier makes a fold read back.
    fn repaired_since(&self, after: u64, through: u64) -> Result<bool>;
    /// Count `count` runs a warm fold looked up in its reverse index, for tests.
    fn count_seat_lookups(&self, count: usize);
    /// Count one direct current-work read, which an enabled work list never makes. Counted in
    /// tests only.
    fn count_direct_work_read(&self);
    fn published_work_list(&self) -> &published_list::PublishedList<WorkRows>;
}

impl WorkListStore for Store {

    fn work_list_changes(
            &self,
            after: u64,
            through: u64,
            seats: &HashMap<String, String>,
        ) -> Result<WorkChanges> {
        let connection = self.readers.get();
        let mut steps = BTreeSet::new();
        let mut reorder = false;
        let mut moved_seats = BTreeSet::new();
        let mut runs = BTreeSet::new();
        let mut requesters = BTreeSet::new();
        let mut every_ask = false;
        let mut tree_roots = BTreeSet::new();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hd9f7e7a219ce owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ published_list::CLAIMS_SINCE)?;
        let claims = statement
            .query_map(params![after, through], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (subject, kind, origin) in claims {
            if let Some(run) = subject.strip_prefix("mission-run/") {
                runs.insert(run.to_owned());
                // A run's state ends the steps of the runs under it, and decides whether asks
                // in them are current: refold the runs under it too.
                if matches!(kind.as_str(), "mission-run.state" | "mission-run.created") {
                    tree_roots.insert(run.to_owned());
                }
            } else if let Some(generation) = subject.strip_prefix("run-generation/") {
                let mut owner = connection
                    .prepare_cached("SELECT run_id FROM run_generations WHERE id=?1")?;
                if let Some(run) = owner.query_row(/* sqlite-large-value: allow-for-now lv-h198436c4efad owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ [generation], |row| row.get::<_, String>(0)).optional()? {
                    runs.insert(run);
                }
            } else if let Some(proposal) = subject.strip_prefix("revision-proposal/") {
                // A draining run shows only its held steps, with no claim on the run.
                runs.extend(self.revision_proposal_run(proposal)?);
            } else if subject.starts_with("step-run/") {
                // A person's answer or a cancelled ask unblocks the step that asked, and ends
                // a run the ask started; answers and cancellations name neither, the ask does.
                if kind.starts_with("work.person-") {
                    let (asked_from, started) = self.ask_origin_and_run(&subject)?;
                    steps.extend(origin.or(asked_from));
                    runs.extend(started.map(|run| run.trim_start_matches("mission-run/").to_owned()));
                }
                steps.insert(subject);
            } else if subject.starts_with("subscription/") || subject.starts_with("schedule/") {
                // An ask's currency walks the subscription or schedule that delivered its run.
                every_ask = true;
            } else if subject.starts_with("agent/") {
                // A seat a step owns shows on that step's row with its usage.
                let mut owned = connection.prepare_cached(
                    "SELECT owner_step FROM desired WHERE subject=?1 AND owner_step IS NOT NULL",
                )?;
                for step in owned.query_map([&subject], |row| row.get::<_, String>(0))? {
                    steps.insert(step?);
                }
                // A seat that left its step: that step's row no longer shows it.
                steps.extend(seats.get(&subject).cloned());
                // A seat's queue order moves its ready work in its own windows.
                if kind == crate::seat_queue::MOVED_CLAIM {
                    reorder = true;
                    moved_seats.insert(viewer_seat(&subject));
                }
                if published_list::decides_asks(&kind) {
                    requesters.insert(subject);
                }
            }
        }
        runs.extend(self.runs_under(&tree_roots)?);
        // A step that asked decides whether its asks are current, as when it leaves waiting.
        let asked = self.asks_from_steps(&steps)?;
        steps.extend(asked);
        // A requester's declaration and runtime decide whether its asks still block steps.
        let asks = if every_ask { self.every_ask()? } else { self.asks_by_requesters(&requesters)? };
        for (ask, origin) in asks {
            steps.insert(ask);
            steps.extend(origin);
        }
        if !runs.is_empty() {
            let mut owned = connection.prepare_cached(
                "SELECT subject FROM step_runs INDEXED BY step_runs_run_index WHERE run_id=?1",
            )?;
            for run in &runs {
                for step in owned.query_map([run], |row| row.get::<_, String>(0))? {
                    steps.insert(step?);
                }
            }
        }
        let runs = runs.iter().map(|run| run_key(run)).collect();
        Ok(WorkChanges { steps, reorder, moved_seats, runs })
    }

    fn steps_with_leases_ended(&self, after: u128, through: u128) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject FROM step_runs INDEXED BY step_runs_lease_index
             WHERE lease_owner IS NOT NULL
               AND CAST(lease_expires_at_unix_ms AS INTEGER)>?1
               AND CAST(lease_expires_at_unix_ms AS INTEGER)<=?2",
        )?;
        let steps = statement
            .query_map(
                params![after.min(i64::MAX as u128) as i64, through.min(i64::MAX as u128) as i64],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(steps)
    }

    fn work_list_candidates(&self) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject FROM step_runs INDEXED BY step_runs_open_index
             WHERE status NOT IN ('completed','failed','cancelled') AND status<>'pending'",
        )?;
        let steps = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(steps)
    }

    fn work_leases(&self) -> Result<HashMap<String, u128>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h13082a1df187 owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ WORK_LEASES)?;
        let leases = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u128)))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(leases)
    }

    fn next_claim_after(&self, subject: &str, time: u128) -> Result<Option<u128>> {
        let connection = self.readers.get();
        let next = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h15f9715ba1db owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ NEXT_CLAIM_AFTER)?
            .query_row(/* sqlite-large-value: allow-for-now lv-hb16ae5c869ea owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ params![subject, time.min(i64::MAX as u128) as i64], |row| row.get::<_, Option<i64>>(0))?;
        Ok(next.map(|next| next.max(0) as u128))
    }

    fn step_timing_lease(&self, subject: &str, attempt: u32, at: u128) -> Result<Option<u128>> {
        Ok(step_timing_lease_at(&self.readers.get(), subject, attempt, at)?)
    }

    fn step_assignees(&self, steps: &BTreeSet<String>) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
        if steps.is_empty() {
            return Ok(Default::default());
        }
        let connection = self.readers.get();
        let rows = connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h1ddde8762a89 owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ STEP_ASSIGNEES)?
            .query_map([serde_json::to_string(steps)?], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let seats = rows
            .iter()
            .filter_map(|(seat, _)| seat.as_deref())
            .filter(|seat| seat.starts_with("agent/"))
            .map(viewer_seat)
            .collect();
        let runs = rows.iter().map(|(_, run)| run_key(run)).collect();
        Ok((seats, runs))
    }

    fn seat_queues_of(&self, seats: &BTreeSet<String>) -> Result<HashMap<String, SeatQueue>> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.fractal.seat_order_reads.fetch_add(seats.len(), std::sync::atomic::Ordering::Relaxed);
            *self.fractal.last_seats_read.lock().unwrap() = seats.clone();
        }
        if seats.is_empty() {
            return Ok(HashMap::new());
        }
        smallclaims::touched::note_read(|| format!("kind:{}", seat_queue::MOVED_CLAIM));
        let connection = self.readers.get();
        // Every move and join in one pass, but only these seats' named runs probed.
        let mut every = seat_queue_inputs_of_tx(&connection, None, Some(seats))?;
        let mut queues = HashMap::new();
        for seat in seats {
            let Some(inputs) = every.remove(seat) else {
                continue;
            };
            let order = inputs.live_order();
            let mut runs = inputs.joins.iter().map(|join| run_key(&join.run)).collect::<BTreeSet<_>>();
            for recorded in &inputs.moves {
                runs.insert(run_key(&recorded.movement.run));
                runs.extend(recorded.movement.anchor.as_deref().map(run_key));
            }
            queues.insert(seat.clone(), SeatQueue { order, runs });
        }
        Ok(queues)
    }

    fn repaired_since(&self, after: u64, through: u64) -> Result<bool> {
        let connection = self.readers.get();
        Ok(connection
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-h8fe8d15d1c36 owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ REPAIRED_SINCE)?
            .query_row(/* sqlite-large-value: allow-for-now lv-h8dac11e4ef43 owner=fractal_missions::store::work_list TODO=trace-statement-and-value-representation */ params![after, through], |_| Ok(()))
            .optional()?
            .is_some())
    }

    fn count_seat_lookups(&self, count: usize) {
        #[cfg(any(test, feature = "test-support"))]
        self.fractal.seat_lookups.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let _ = count;
    }

    fn count_direct_work_read(&self) {
        #[cfg(any(test, feature = "test-support"))]
        self.fractal.direct_work_reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn published_work_list(&self) -> &published_list::PublishedList<WorkRows> {
        &self.fractal.published_work
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_lists_reads_search_their_indexes_and_scan_no_table() {
        let store = Store::open_memory("node").unwrap();
        let connection = store.readers.get();
        for (name, sql) in [
            ("claims since", published_list::CLAIMS_SINCE.to_owned()),
            ("asks by requesters", published_list::ASKS_BY_REQUESTERS.to_owned()),
            ("ask origin", published_list::ASK_ORIGIN_AND_RUN.to_owned()),
            ("asks from steps", published_list::ASKS_FROM_STEPS.to_owned()),
            ("run steps", published_list::RUN_STEPS.to_owned()),
            ("child runs", published_list::CHILD_RUNS.to_owned()),
            ("work leases", WORK_LEASES.to_owned()),
            ("next claim", NEXT_CLAIM_AFTER.to_owned()),
            ("step assignees", STEP_ASSIGNEES.to_owned()),
            ("repaired since", REPAIRED_SINCE.to_owned()),
            ("seat queue moves", seat_queue_moves_query()),
            ("selected work", selected_work_at_snapshot_query(false)),
            ("selected mission keys", super::super::mission_list::mission_keys_sql(true)),
        ] {
            let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let parameters = statement.parameter_count();
            let plan = statement
                .query_map(rusqlite::params_from_iter(std::iter::repeat_n("[]", parameters)), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for line in &plan {
                for table in ["claims", "step_runs", "mission_runs", "mission_definitions"] {
                    assert!(
                        !line.starts_with(&format!("SCAN {table}")),
                        "{name} scans {table}:\n{}",
                        plan.join("\n")
                    );
                }
            }
        }
    }
}
