//! What a mission run waits on, and the fleet's blocked runs grouped by cause, read from state st
//! already keeps current: the published work list (every open step with its holder, lease, last
//! progress and wake), the published missions list's open human gates and agents roster, and,
//! for one run, its own steps' rows. Neither read folds claims or replays history, so each costs
//! what the open work and the run's steps are, never the store's size (docs/st3/waiting-on.md).

use crate::MissionsStore;
use crate::PublishedViewsStore;
use crate::WorkListStore;
use super::*;
use smallclaims::store::now_ms;
use std::collections::HashMap;
use crate::store::mission_list::MissionRows;
use crate::store::published_list::{ListRead, Publication};
use crate::store::work_list::{WorkRow, WorkRows};
use crate::waiting_on::{
    BlockedGroup, BlockedRun, BlockedRuns, RunSteps, RunVerdict, RunWaitingOn, StepSettled, StepWait,
    WaitKind, describe_age,
};
use fractal_ui_model::missions::{Fleet, Gate, Seat};

/// How long a held step may go without progress, and a ready step without a claim, before it
/// counts as stuck: the time st waits before it nudges an idle holder.
pub const DEFAULT_QUIET_AFTER_MS: u64 = crate::config::IDLE_NUDGE_AFTER_MS;

pub const WAITING_ON_UNAVAILABLE: &str = "waiting-on-unavailable";

#[derive(Debug, Default, Deserialize)]
pub struct WaitingOnQuery {
    /// Overrides [`DEFAULT_QUIET_AFTER_MS`].
    pub quiet_after_ms: Option<u64>,
}

/// The published state both reads use, or why it cannot be served now.
struct Sources {
    work: Arc<Publication<WorkRows>>,
    missions: Arc<Publication<MissionRows>>,
}

fn unavailable() -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: WAITING_ON_UNAVAILABLE.into(),
        message: "this daemon publishes no work and missions lists (ST3_PUBLISHED_LISTS=off), which this view reads"
            .into(),
        details: Box::default(),
    }
}

fn sources(store: &Store) -> Result<Sources, ApiError> {
    let list = store.published_work_list();
    let work = match list.current() {
        ListRead::Served(publication, _) => publication,
        ListRead::NeverStarted if !store.collection_view_published("work") => return Err(unavailable()),
        ListRead::NeverStarted | ListRead::NotReady => {
            list.request_refresh();
            return Err(published_lists::work_list_not_ready());
        }
        ListRead::Ended => return Err(published_lists::work_list_ended()),
    };
    let Some((missions, _)) = client_v0::published_missions::current(store)? else {
        return Err(unavailable());
    };
    Ok(Sources { work, missions })
}

/// What a seat's row says about whether it can work: stopped, failed, suspended, down, retired
/// or faulted cannot.
fn seat_down(seat: &Seat) -> bool {
    matches!(seat.state.as_str(), "failed" | "stopped" | "suspended" | "down" | "retired") || seat.fault
}

/// The published facts one classification reads beside a step's own row.
struct Context<'a> {
    rows: &'a WorkRows,
    gates: &'a [Gate],
    fleet: Option<&'a dyn Fleet>,
    now: u128,
    quiet_after_ms: u64,
}

impl Context<'_> {
    fn seat(&self, agent: &str) -> Option<Seat> {
        self.fleet.and_then(|fleet| fleet.seat(agent))
    }

    fn quiet(&self, since: u128) -> bool {
        self.now.saturating_sub(since) > u128::from(self.quiet_after_ms)
    }
}

/// What one open step waits on.
fn classify(row: &WorkRow, context: &Context<'_>) -> StepWait {
    let view = &row.view;
    let mut wait = StepWait {
        step: view.subject.clone(),
        path: view.step.clone(),
        title: view.title.clone(),
        state: view.status.clone(),
        kind: WaitKind::Ready,
        who: None,
        seat_state: None,
        since_unix_ms: Some(view.updated_at_unix_ms),
        lease_expires_at_unix_ms: None,
        until_unix_ms: None,
        detail: None,
        wake_attempts: None,
    };
    let held = matches!(view.status.as_str(), "claimed" | "working" | "verifying");
    if held {
        wait.lease_expires_at_unix_ms = view.claim_expires_at_unix_ms;
    }
    // A person must answer: a human gate on the step, the step is an ask, or it waits on one.
    if let Some(gate) = context.gates.iter().find(|gate| gate.step.as_deref() == Some(view.subject.as_str())) {
        wait.kind = WaitKind::Ask;
        wait.who = Some(gate.person.clone());
        wait.detail = Some("a human gate waits for an answer".into());
        return wait;
    }
    if let Some(person) = view.assigned_to.as_deref().filter(|actor| actor.starts_with("person/")) {
        wait.kind = WaitKind::Ask;
        wait.who = Some(person.to_owned());
        wait.since_unix_ms = Some(view.created_at_unix_ms);
        wait.detail = view.title.clone().or_else(|| view.goals.first().cloned());
        return wait;
    }
    if view.status == "waiting-person" {
        let ask = view.blocked_reason.as_deref().and_then(|ask| context.rows.rows.get(ask));
        wait.kind = WaitKind::Ask;
        wait.who = ask.and_then(|ask| ask.view.assigned_to.clone());
        wait.since_unix_ms = ask.map_or(Some(view.updated_at_unix_ms), |ask| Some(ask.view.created_at_unix_ms));
        wait.detail = ask
            .and_then(|ask| ask.view.title.clone().or_else(|| ask.view.goals.first().cloned()))
            .or_else(|| view.blocked_reason.clone());
        return wait;
    }
    if view.status == "blocked" {
        wait.kind = WaitKind::Hold;
        wait.until_unix_ms = view.not_before_unix_ms;
        wait.detail = view.blocked_reason.clone().or_else(|| {
            (!view.blockers.is_empty()).then(|| format!("blocked by {}", view.blockers.join(", ")))
        });
        return wait;
    }
    if view.agentless {
        wait.who = Some("st".into());
        match row.value["gate_kind"].as_str() {
            Some("watch") => wait.kind = WaitKind::Watching,
            Some("run") => {
                wait.kind = WaitKind::Hold;
                wait.detail = Some(
                    view.blocked_reason.clone().unwrap_or_else(|| "waits for another run to finish".into()),
                );
            }
            kind => {
                wait.kind = WaitKind::Verifying;
                wait.detail = kind.map(|kind| format!("st checks its {kind} gates"));
            }
        }
        return wait;
    }
    if let Some(until) = view.not_before_unix_ms.filter(|until| *until > context.now) {
        wait.kind = WaitKind::Hold;
        wait.until_unix_ms = Some(until);
        wait.detail = Some(format!("retry backoff, attempt {}", view.attempt));
        return wait;
    }
    // The seat that holds the step, or that would take it: a ready step waits in whichever queue
    // holds it, whoever it was assigned to.
    let queued = (view.status == "ready" && view.claimant.is_none())
        .then(|| context.fleet.and_then(|fleet| fleet.queued_for(&view.subject)))
        .flatten();
    let seat_id = if held { view.claimant.clone() } else { None }
        .or_else(|| queued.as_ref().map(|seat| seat.id.clone()))
        .or_else(|| view.claimant.clone())
        .or_else(|| view.assigned_to.clone());
    let seat = queued.or_else(|| seat_id.as_deref().and_then(|id| context.seat(id)));
    wait.who = seat_id.clone().or_else(|| (!view.available_to.is_empty()).then(|| view.available_to.join(", ")));
    wait.seat_state = seat.as_ref().map(|seat| seat.state.clone());
    if let Some(seat) = seat.as_ref().filter(|seat| seat_down(seat)) {
        wait.kind = WaitKind::SeatDown;
        wait.detail = Some(if held {
            format!("{} holds it and is {}", seat.id, seat.state)
        } else {
            format!("{} would take it and is {}", seat.id, seat.state)
        });
        if held {
            wait.since_unix_ms = view.progress_at_unix_ms.or(view.execution_started_at_unix_ms).or(wait.since_unix_ms);
        }
        return wait;
    }
    match view.status.as_str() {
        "claimed" | "working" => {
            if let Some(until) = view.execution_paused_until_unix_ms.filter(|until| *until > context.now) {
                wait.kind = WaitKind::Hold;
                wait.until_unix_ms = Some(until);
                wait.since_unix_ms = view.progress_at_unix_ms.or(wait.since_unix_ms);
                wait.detail = Some(view.progress_summary.clone().unwrap_or_else(|| "a declared wait".into()));
                return wait;
            }
            // The last sign the holder is moving: its progress, else the start of this attempt.
            let since = [view.progress_at_unix_ms, view.execution_started_at_unix_ms]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(view.updated_at_unix_ms);
            wait.since_unix_ms = Some(since);
            wait.kind = if context.quiet(since) { WaitKind::NoProgress } else { WaitKind::Working };
            wait.detail = view.progress_summary.clone();
        }
        "verifying" => {
            wait.kind = WaitKind::Verifying;
            wait.detail = view.completion_summary.clone();
        }
        "ready" => {
            wait.wake_attempts = view.wake.as_ref().map(|wake| wake.attempts).filter(|attempts| *attempts > 0);
            match seat.as_ref().and_then(|seat| seat.busy_with.clone()) {
                // Its seat holds other work: it waits its turn, and nothing is wrong.
                Some((mission, path)) => {
                    wait.kind = WaitKind::Queued;
                    wait.detail = Some(format!(
                        "behind {} › {path}",
                        mission.trim_start_matches("mission/")
                    ));
                }
                _ => {
                    wait.kind = if context.quiet(view.updated_at_unix_ms) {
                        WaitKind::Unclaimed
                    } else {
                        WaitKind::Ready
                    };
                }
            }
        }
        _ => {
            wait.kind = WaitKind::Hold;
            wait.detail = view.blocked_reason.clone();
        }
    }
    wait
}

/// Most blocking first: the causes in their order, a step that asked before the ask it made,
/// then whatever has waited longest.
fn sort_waits(waits: &mut [StepWait]) {
    let ask_itself = |wait: &StepWait| wait.state != "waiting-person";
    waits.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(ask_itself(left).cmp(&ask_itself(right)))
            .then(left.since_unix_ms.cmp(&right.since_unix_ms))
            .then(left.path.cmp(&right.path))
    });
}

/// The run's verdict and cause from its open steps, sorted: stalled when any step is stuck,
/// whatever else moves; else moving when any step moves; else waiting on its first cause.
fn verdict(open: &[StepWait]) -> (RunVerdict, Option<WaitKind>) {
    let cause = open.iter().map(|wait| wait.kind).find(|kind| kind.is_cause());
    if open.iter().any(|wait| wait.kind.stalls()) {
        return (RunVerdict::Stalled, cause);
    }
    if open.iter().any(|wait| !wait.kind.is_cause()) {
        return (RunVerdict::Moving, None);
    }
    (if cause.is_some() { RunVerdict::Waiting } else { RunVerdict::Moving }, cause)
}

/// One step's wait in words, as a run's reason and a fleet line say it.
pub fn describe(wait: &StepWait, now: u128) -> String {
    let age = wait.since_unix_ms.map(|since| describe_age(now.saturating_sub(since))).unwrap_or_default();
    let who = wait.who.as_deref().unwrap_or("nobody");
    let detail = wait.detail.as_deref().map(|detail| format!(": {detail}")).unwrap_or_default();
    match wait.kind {
        WaitKind::Ask => format!("{} waits on {who} for {age}{detail}", wait.path),
        WaitKind::SeatDown => format!(
            "{}: {who} is {}{}",
            wait.path,
            wait.seat_state.as_deref().unwrap_or("down"),
            if wait.state == "ready" { ", and would take it" } else { " and holds it" }
        ),
        WaitKind::NoProgress => format!("{}: {who} holds it, no progress for {age}{detail}", wait.path),
        WaitKind::Unclaimed => format!("{}: ready for {age}, nobody claims it", wait.path),
        WaitKind::Hold => match wait.until_unix_ms {
            Some(until) if until > now => {
                format!("{}: held for {} more{detail}", wait.path, describe_age(until - now))
            }
            _ => format!("{}: held{detail}", wait.path),
        },
        WaitKind::Working => format!("{}: {who} working, last progress {age} ago{detail}", wait.path),
        WaitKind::Verifying => format!("{}: verifying{detail}", wait.path),
        WaitKind::Ready => format!("{}: ready for {age}", wait.path),
        WaitKind::Queued => format!("{}: queued for {who}{detail}", wait.path),
        WaitKind::Watching => format!("{}: watching", wait.path),
    }
}

fn reason(verdict: RunVerdict, open: &[StepWait], pending: usize, status: &str, now: u128) -> String {
    match verdict {
        RunVerdict::Finished => format!("finished: {status}"),
        RunVerdict::Stalled => {
            let stuck = open.iter().find(|wait| wait.kind.stalls()).expect("a stalled run has a stuck step");
            describe(stuck, now)
        }
        RunVerdict::Waiting => describe(&open[0], now),
        RunVerdict::Moving => match open.iter().find(|wait| !wait.kind.is_cause()) {
            Some(moving) => describe(moving, now),
            None if pending > 0 => format!("no open step; {pending} pending for st to make ready"),
            None => "no open step".into(),
        },
    }
}

fn context<'a>(work: &'a WorkRows, missions: &'a MissionRows, now: u128, quiet_after_ms: u64) -> Context<'a> {
    Context {
        rows: work,
        gates: &missions.gates,
        fleet: missions.roster.as_deref().map(|roster| roster as &dyn Fleet),
        now,
        quiet_after_ms,
    }
}

/// What one run waits on: its open steps from the work list, the rest from its own rows.
pub fn run_waiting_on(
    steps: &RunSteps,
    work: &Publication<WorkRows>,
    missions: &MissionRows,
    now: u128,
    quiet_after_ms: u64,
) -> RunWaitingOn {
    let context = context(&work.rows, missions, now, quiet_after_ms);
    let mut open = Vec::new();
    let mut pending = Vec::new();
    let mut finished = Vec::new();
    for step in &steps.steps {
        if let Some(row) = work.rows.rows.get(&step.subject) {
            open.push(classify(row, &context));
            continue;
        }
        let settled = StepSettled {
            step: step.subject.clone(),
            path: step.path.clone(),
            title: step.title.clone(),
            state: step.status.clone(),
            at_unix_ms: step.updated_at_unix_ms,
        };
        if matches!(step.status.as_str(), "completed" | "failed" | "cancelled") {
            finished.push(settled);
        } else {
            pending.push(settled);
        }
    }
    sort_waits(&mut open);
    finished.sort_by(|left, right| right.at_unix_ms.cmp(&left.at_unix_ms).then(left.path.cmp(&right.path)));
    let (verdict, cause) = if matches!(steps.status.as_str(), "completed" | "failed" | "cancelled") {
        (RunVerdict::Finished, None)
    } else {
        verdict(&open)
    };
    RunWaitingOn {
        run: steps.run.clone(),
        mission: steps.mission.clone(),
        status: steps.status.clone(),
        generation: steps.generation.clone(),
        reason: reason(verdict, &open, pending.len(), &steps.status, now),
        verdict,
        cause,
        open,
        pending,
        finished,
        quiet_after_ms,
        work_cut: work.cut,
        as_of_unix_ms: now,
    }
}

/// Every run with open work that is stalled or waiting, one line each, grouped by cause.
pub fn blocked_runs(
    work: &Publication<WorkRows>,
    missions: &MissionRows,
    now: u128,
    quiet_after_ms: u64,
) -> BlockedRuns {
    let context = context(&work.rows, missions, now, quiet_after_ms);
    let mut runs = HashMap::<&str, (Option<&str>, Vec<StepWait>)>::new();
    for row in work.rows.rows.values() {
        let entry = runs.entry(row.view.run.as_str()).or_insert_with(|| (row.value["mission_id"].as_str(), Vec::new()));
        entry.1.push(classify(row, &context));
    }
    let runs_read = runs.len();
    let mut groups = WaitKind::CAUSES
        .iter()
        .map(|cause| BlockedGroup { cause: *cause, runs: Vec::new() })
        .collect::<Vec<_>>();
    for (run, (mission, mut open)) in runs {
        sort_waits(&mut open);
        let (run_verdict, Some(cause)) = verdict(&open) else { continue };
        if run_verdict == RunVerdict::Moving {
            continue;
        }
        let blocked_steps = open.iter().filter(|wait| wait.kind.is_cause()).count();
        // A stalled run is listed under its stuck step's cause; a waiting one under its first.
        let step = match run_verdict {
            RunVerdict::Stalled => open.iter().find(|wait| wait.kind.stalls()),
            _ => open.iter().find(|wait| wait.kind == cause),
        }
        .expect("a blocked run has a step of its cause")
        .clone();
        let cause = step.kind;
        let group = groups.iter_mut().find(|group| group.cause == cause).expect("every cause has a group");
        group.runs.push(BlockedRun {
            run: run.to_owned(),
            mission: mission.map(str::to_owned),
            cause,
            verdict: run_verdict,
            step,
            blocked_steps,
        });
    }
    for group in &mut groups {
        group.runs.sort_by(|left, right| {
            left.step.since_unix_ms.cmp(&right.step.since_unix_ms).then(left.run.cmp(&right.run))
        });
    }
    groups.retain(|group| !group.runs.is_empty());
    BlockedRuns { groups, runs_read, quiet_after_ms, work_cut: work.cut, as_of_unix_ms: now }
}

pub async fn get_run_waiting_on(
    State(state): State<AppState>,
    AxumPath(run): AxumPath<String>,
    Query(query): Query<WaitingOnQuery>,
) -> Result<Json<RunWaitingOn>, ApiError> {
    let Sources { work, missions } = sources(&state.store)?;
    let quiet_after_ms = query.quiet_after_ms.unwrap_or(DEFAULT_QUIET_AFTER_MS);
    let store = state.store.clone();
    let wanted = run.clone();
    blocking_store(move || {
        let Some(steps) = store.run_step_states(&wanted)? else {
            return Ok(None);
        };
        Ok(Some(run_waiting_on(&steps, &work, &missions.rows, now_ms(), quiet_after_ms)))
    })
    .await?
    .map(Json)
    .ok_or_else(|| ApiError::not_found(format!("mission run `{run}` does not exist")))
}

pub async fn get_blocked_runs(
    State(state): State<AppState>,
    Query(query): Query<WaitingOnQuery>,
) -> Result<Json<BlockedRuns>, ApiError> {
    let Sources { work, missions } = sources(&state.store)?;
    let quiet_after_ms = query.quiet_after_ms.unwrap_or(DEFAULT_QUIET_AFTER_MS);
    Ok(Json(blocked_runs(&work, &missions.rows, now_ms(), quiet_after_ms)))
}

#[cfg(test)]
#[path = "waiting_on_tests.rs"]
mod tests;
