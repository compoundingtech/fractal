//! A run that names a reporter tells that agent when it fails, is cancelled or stalls, and when
//! it completes if asked. The reconciler decides this while it evaluates the run, so a run that
//! does not change costs nothing, and a stall is a deadline on the run's item rather than a scan.
//!
//! One message per distinct event: its identity is the run, the event and, for a stall, the
//! last progress the run made, so an evaluation that repeats, a restart and a replayed pass all
//! find the message already sent. The message names the run and its steps and links the commands
//! that show them. It never copies fault or cancellation text, which can carry anything a step
//! printed. A reporter that is not a live agent gets a fault on the run instead, once per
//! reason, and the next evaluation looks again without sending anything twice.
//!
//! A run that never named a reporter still reports a stall by default: to its requester when that
//! is an agent, else to an agent that the stuck step's agent is `under`. Never to the stuck agent
//! itself, never to an agent that is not running, and never for a silence that was already past
//! its limit when the default was turned on, so an upgrade does not send a burst.
use crate::AttentionSnapshotStore;
use crate::MissionsStore;
use super::*;
use serde_json::json;

/// The scope of the fault a run records when its reporter cannot be told.
pub const REPORT_FAULT_SCOPE: &str = "report-to";

/// Who a run reports to and which events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReport {
    pub to: String,
    pub stalled_after_ms: u64,
    pub completed: bool,
    pub stalls: bool,
}

/// What a run says about its reporter.
pub(super) enum ReportChoice {
    /// The run, or its latest `report-to` change, names this report.
    Named(RunReport),
    /// `report-to --clear`: nobody, not even a default.
    Cleared,
    /// The run never named one, so its stalls go to a default reporter.
    Unnamed,
}

/// The agents that hold or are assigned the run's open work: the agents whose silence a stall is.
fn stuck_agents(run: &MissionRunView) -> Vec<String> {
    let mut agents = Vec::new();
    for step in &run.steps {
        if step.agentless || !matches!(step.status.as_str(), "ready" | "claimed" | "working") {
            continue;
        }
        let agent = step
            .claimant
            .as_ref()
            .or(step.assigned_to.as_ref())
            .or_else(|| (step.available_to.len() == 1).then(|| &step.available_to[0]));
        if let Some(agent) = agent.filter(|agent| agent.starts_with("agent/"))
            && !agents.contains(agent)
        {
            agents.push(agent.clone());
        }
    }
    agents
}

/// What happened to a run that its reporter is told.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunEvent {
    Failed,
    Cancelled,
    Completed,
    Stalled { since: u128 },
}

impl RunEvent {
    fn name(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Stalled { .. } => "stalled",
        }
    }
}

/// The last sign of life in a run: its own last change and each step's last sign of life.
pub fn run_activity(run: &MissionRunView) -> u128 {
    run.steps
        .iter()
        .flat_map(step_activity)
        .flatten()
        .chain([run.created_at_unix_ms, run.updated_at_unix_ms])
        .max()
        .unwrap_or_default()
}

/// A step's signs of life. A step nobody holds changes only when something happens to it, so
/// its last change counts. A held step is different: its holder's driver renews the lease every
/// minute, and each renewal moves both the lease end and the step's last change, so neither says
/// that work is being done. A held step shows life through the start of its current execution
/// interval (its claim, or the end of its last wait), its last progress record, and a declared
/// wait until that wait ends on its own.
fn step_activity(step: &StepRunView) -> [Option<u128>; 3] {
    let held = is_held(step);
    let declared = [
        step.execution_started_at_unix_ms,
        step.execution_paused_until_unix_ms,
    ];
    if held && declared.iter().any(Option::is_some) {
        [declared[0], step.progress_at_unix_ms, declared[1]]
    } else {
        [Some(step.updated_at_unix_ms), step.progress_at_unix_ms, None]
    }
}

/// Whether a worker holds the step, so that its driver renews the step's lease.
fn is_held(step: &StepRunView) -> bool {
    step.claimant.is_some() && matches!(step.status.as_str(), "claimed" | "working")
}

/// Whether a run is still working, so that silence means something. A run in cleanup is ending,
/// and a run that waits for the run before it has not started.
fn can_stall(run: &MissionRunView) -> bool {
    matches!(run.phase.as_str(), "normal" | "revision-draining" | "final")
        && !run.steps.iter().any(|step| {
            step.step == crate::mission::AFTER_RUN_STEP && step.status != "completed"
        })
}

impl<R: RuntimeControl> Reconciler<R> {
    /// Linked child runs do the parent's work. Follow their indexed parent-step links and
    /// register every run and step read, including nested work, so progress moves the stall
    /// deadline on the next incremental evaluation rather than waiting for a scan.
    fn run_report_activity(&self, run: &MissionRunView) -> Result<u128> {
        let mut activity = run_activity(run).max(self.held_progress(run)?);
        let mut pending = run
            .steps
            .iter()
            .map(|step| step.subject.clone())
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(step) = pending.pop() {
            for child in self.store.mission_run_children_for_step(&step)? {
                if !visited.insert(child.subject.clone()) {
                    continue;
                }
                activity = activity
                    .max(run_activity(&child))
                    .max(self.held_progress(&child)?);
                pending.extend(child.steps.into_iter().map(|step| step.subject));
            }
        }
        Ok(activity)
    }

    /// The last progress record of the steps a worker holds. A run read for evaluation does not
    /// carry step progress, and a held step's last change cannot stand in for it because lease
    /// renewals move it, so read each held step's own record.
    fn held_progress(&self, run: &MissionRunView) -> Result<u128> {
        let mut progress = 0;
        for step in &run.steps {
            if step.progress_at_unix_ms.is_some() || !is_held(step) {
                continue;
            }
            if let Some(at) = self
                .store
                .step_run(&step.subject)?
                .and_then(|step| step.progress_at_unix_ms)
            {
                progress = progress.max(at);
            }
        }
        Ok(progress)
    }

    /// Who a run reports to: its latest `st missions report-to`, else what it was created with.
    /// A default reporter is not named, so it is not returned here.
    pub fn run_report(&self, run: &str) -> Result<Option<RunReport>> {
        Ok(match self.report_choice(run)? {
            ReportChoice::Named(report) => Some(report),
            ReportChoice::Cleared | ReportChoice::Unnamed => None,
        })
    }

    pub(super) fn report_choice(&self, run: &str) -> Result<ReportChoice> {
        let fields = |claim: &crate::model::ClaimRecord| {
            crate::store::run_report_fields(&claim.body).map(
                |(to, stalled_after_ms, completed, stalls)| RunReport {
                    to,
                    stalled_after_ms,
                    completed,
                    stalls,
                },
            )
        };
        if let Some(changed) = self
            .store
            .latest_claim(run, Some(crate::store::RUN_REPORT_KIND))?
        {
            return Ok(fields(&changed).map_or(ReportChoice::Cleared, ReportChoice::Named));
        }
        Ok(self
            .store
            .latest_claim(run, Some("mission-run.created"))?
            .and_then(|created| fields(&created))
            .map_or(ReportChoice::Unnamed, ReportChoice::Named))
    }

    /// Turn the default stall report on or off. Turning it on starts the time before which a
    /// silence that was already past its limit is never reported by default.
    pub fn set_default_stall_reports(&self, on: bool) {
        let mut since = self
            .default_stall_reports_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *since = match (on, *since) {
            (false, _) => None,
            (true, None) => Some(now_ms()),
            (true, kept) => kept,
        };
    }

    /// The agents a run that names no reporter tells of its stall, in order, with why each is
    /// told: its requester when that is an agent, then each agent a stuck agent is `under`.
    /// A stuck agent itself is never one.
    fn default_reporters(&self, run: &MissionRunView, stuck: &[String]) -> Result<Vec<(String, String)>> {
        let mut reporters = Vec::<(String, String)>::new();
        if run.requester.starts_with("agent/") && !stuck.contains(&run.requester) {
            reporters.push((run.requester.clone(), "you requested the run".into()));
        }
        for agent in stuck {
            for declared in self.store.desired_subjects_named(std::slice::from_ref(agent))? {
                if declared.kind != "agent" {
                    continue;
                }
                for under in crate::graph::agent_under(&declared.desired) {
                    if !stuck.contains(&under.agent)
                        && !reporters.iter().any(|(to, _)| *to == under.agent)
                    {
                        reporters.push((under.agent, format!("{agent} is `under` you")));
                    }
                }
            }
        }
        Ok(reporters)
    }

    /// Tell a default reporter that a run that names none has stalled, and return when it will
    /// have if it has not yet. Only a stall is reported by default. A run linked from a parent
    /// step is left to its parent, which counts its activity and its stuck agents.
    pub(super) fn report_stall_by_default(&self, run: &MissionRunView, now: u128) -> Result<Option<u128>> {
        let Some(enabled_since) = *self
            .default_stall_reports_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        else {
            return Ok(None);
        };
        if run.parent_step_run.is_some() || !can_stall(run) {
            return Ok(None);
        }
        // Work in linked child runs, such as loop rounds and nested work, is the run's work too.
        let mut stuck = stuck_agents(run);
        let mut pending = run.steps.iter().map(|step| step.subject.clone()).collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(step) = pending.pop() {
            for child in self.store.mission_run_children_for_step(&step)? {
                if !visited.insert(child.subject.clone()) {
                    continue;
                }
                for agent in stuck_agents(&child) {
                    if !stuck.contains(&agent) {
                        stuck.push(agent);
                    }
                }
                pending.extend(child.steps.into_iter().map(|step| step.subject));
            }
        }
        if stuck.is_empty() {
            return Ok(None);
        }
        let reporters = self.default_reporters(run, &stuck)?;
        if reporters.is_empty() {
            return Ok(None);
        }
        let stalled_after_ms = crate::mission::DEFAULT_STALLED_AFTER_MS;
        let since = self.run_report_activity(run)?;
        let stalls_at = since.saturating_add(u128::from(stalled_after_ms));
        if now < stalls_at {
            return Ok(Some(stalls_at));
        }
        // A silence already past its limit when the default came on is old news, not a stall
        // this daemon saw happen.
        if stalls_at < enabled_since {
            return Ok(None);
        }
        for (to, why) in reporters {
            if self.store.agent_is_live(&to)? {
                let report = RunReport { to, stalled_after_ms, completed: false, stalls: true };
                self.tell_reporter(run, &report, RunEvent::Stalled { since }, Some(&why))?;
                break;
            }
        }
        Ok(None)
    }

    /// Tell the reporter what this run's phase calls for, and return when a run that has not
    /// stalled yet will have: the time to evaluate it again.
    pub fn report_run(
        &self,
        run: &MissionRunView,
        report: &RunReport,
        now: u128,
    ) -> Result<Option<u128>> {
        let event = match run.phase.as_str() {
            "cleanup-failed" => Some(RunEvent::Failed),
            "cleanup-cancelled" | "final-cancelled" => Some(RunEvent::Cancelled),
            "cleanup-completed" if report.completed => Some(RunEvent::Completed),
            _ => None,
        };
        if let Some(event) = event {
            self.tell_reporter(run, report, event, None)?;
            return Ok(None);
        }
        if !report.stalls || !can_stall(run) {
            return Ok(None);
        }
        let since = self.run_report_activity(run)?;
        let stalls_at = since.saturating_add(u128::from(report.stalled_after_ms));
        if now < stalls_at {
            return Ok(Some(stalls_at));
        }
        self.tell_reporter(run, report, RunEvent::Stalled { since }, None)?;
        Ok(None)
    }

    /// Tell the reporter of the run that owns `seat` that the seat stays down after its launch was retried
    /// as often as it is: one message for each seat and declaration, so a seat that stays stopped is
    /// noticed. A run that reports to nobody, or to an agent that is not running, is not told.
    pub(super) fn tell_seat_down(&self, seat: &str, run: &str, token: &str, reason: &str) -> Result<()> {
        let Some(report) = self.run_report(run)? else {
            return Ok(());
        };
        let key = format!("st3-run-report:{run}:seat-down:{seat}:{token}");
        let message_id = &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16];
        let subject = format!("message/{message_id}");
        if self.store.message_sent(&subject)?
            || !self.store.agent_is_live(&report.to)?
        {
            return Ok(());
        }
        crate::store::message_routing::send(&self.store, &ClaimInput {
            subject,
            kind: "message.sent".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("from".into(), Value::String("daemon/runtime".into())),
                ("to".into(), Value::String(report.to.clone())),
                (
                    "content".into(),
                    Value::String(format!(
                        "The seat {seat} of run {run} stays down: {reason}\n\nInspect: `st agents show {seat}`; restart it with `st agents restart {seat} --as YOU`.\n\nYou are told because the run names you in `report-to`. st sends one message for each seat and declaration."
                    )),
                ),
                ("status".into(), Value::String("sent".into())),
                ("title".into(), Value::String(format!("Seat stays down: {seat}"))),
                ("in_reply_to".into(), Value::Null),
                (
                    "tags".into(),
                    json!([format!("st3-run-report:seat-down"), format!("mission-run:{run}")]),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key),
        })?;
        self.signal_changed();
        Ok(())
    }

    fn tell_reporter(
        &self,
        run: &MissionRunView,
        report: &RunReport,
        event: RunEvent,
        default_because: Option<&str>,
    ) -> Result<()> {
        let key = match event {
            RunEvent::Stalled { since } => {
                format!("st3-run-report:{}:stalled:{since}", run.subject)
            }
            _ => format!("st3-run-report:{}:{}", run.subject, event.name()),
        };
        let message_id = &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..16];
        let subject = format!("message/{message_id}");
        if self.store.message_sent(&subject)? {
            return self.close_fault(
                &run.subject,
                REPORT_FAULT_SCOPE,
                "the reporter was told",
            );
        }
        if !self.store.agent_is_live(&report.to)? {
            return self.record_fault(
                &run.subject,
                REPORT_FAULT_SCOPE,
                Err(anyhow::anyhow!(
                    "{} is not a running agent, so it was not told that the run {}",
                    report.to,
                    event.name()
                )),
            );
        }
        let steps = |keep: &dyn Fn(&StepRunView) -> bool| {
            let listed = run
                .steps
                .iter()
                .filter(|step| keep(step))
                .map(|step| format!("- {} ({})", step.step, step.status))
                .collect::<Vec<_>>();
            if listed.is_empty() {
                String::new()
            } else {
                format!("\n\nSteps:\n{}", listed.join("\n"))
            }
        };
        let (title, detail) = match event {
            RunEvent::Failed => (
                "failed",
                format!(
                    "The run failed.{}",
                    steps(&|step| matches!(step.status.as_str(), "failed" | "orphaned"))
                ),
            ),
            RunEvent::Cancelled => ("was cancelled", "The run was cancelled.".to_owned()),
            RunEvent::Completed => ("completed", "The run completed.".to_owned()),
            RunEvent::Stalled { since } => (
                "has stalled",
                format!(
                    "The run has made no progress for {} (its limit is {}). It is not failed and may still go on.{}",
                    describe_duration(now_ms().saturating_sub(since)),
                    describe_duration(u128::from(report.stalled_after_ms)),
                    steps(&|step| {
                        !matches!(step.status.as_str(), "completed" | "cancelled")
                    })
                ),
            ),
        };
        let because = match default_because {
            None => "the run names you in `report-to`".to_owned(),
            Some(why) => format!(
                "{why} and the run names no `report-to`. A person or the requester can name a reporter, or stop these with `st missions report-to {} --clear`",
                run.subject
            ),
        };
        let content = format!(
            "{detail}\n\nRun: {run_subject}\nMission: {mission}\n\nInspect: `st missions show {run_subject}`, and `st work show STEP_SUBJECT` for a step. The reason is on the run, not in this message.\n\nYou are told because {because}. st sends one message for each event of a run.",
            run_subject = run.subject,
            mission = run.mission,
        );
        crate::store::message_routing::send(&self.store, &ClaimInput {
            subject,
            kind: "message.sent".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("from".into(), Value::String("daemon/runtime".into())),
                ("to".into(), Value::String(report.to.clone())),
                ("content".into(), Value::String(content)),
                ("status".into(), Value::String("sent".into())),
                (
                    "title".into(),
                    Value::String(format!("Run {title}: {}", run.subject)),
                ),
                ("in_reply_to".into(), Value::Null),
                (
                    "tags".into(),
                    json!([
                        format!("st3-run-report:{}", event.name()),
                        format!("mission-run:{}", run.subject),
                    ]),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(key),
        })?;
        self.close_fault(&run.subject, REPORT_FAULT_SCOPE, "the reporter was told")
    }
}

fn describe_duration(ms: u128) -> String {
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => format!("{}s", ms / 1_000),
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h{minutes}m"),
    }
}
