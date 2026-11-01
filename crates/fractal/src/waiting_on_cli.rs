//! `st missions waiting-on RUN` and `st missions blocked`: what a run waits on, and every blocked
//! run in the fleet by cause, rendered from the daemon's published answer.

use fractal_daemon::waiting_on::{BlockedRuns, RunWaitingOn, StepSettled, StepWait, WaitKind, describe_age};
use std::fmt::Write as _;

/// How many finished steps a run's page names before it counts the rest.
const FINISHED_SHOWN: usize = 10;

fn short(id: &str) -> &str {
    id.trim_start_matches("mission-run/").trim_start_matches("mission/")
}

fn age(now: u128, at: Option<u128>) -> Option<String> {
    at.map(|at| describe_age(now.saturating_sub(at)))
}

/// One open step: its kind, path, whom it waits on, for how long, and what st knows of it.
fn step_line(wait: &StepWait, now: u128) -> String {
    let mut line = format!("{:<12} {}", wait.kind.name(), wait.path);
    if let Some(who) = &wait.who {
        let _ = write!(line, " · {who}");
        if let Some(state) = wait.seat_state.as_deref().filter(|state| *state != "running") {
            let _ = write!(line, " ({state})");
        }
    }
    let since = age(now, wait.since_unix_ms);
    match (wait.kind, since) {
        (WaitKind::NoProgress | WaitKind::Working, Some(since)) => {
            let _ = write!(line, " · last progress {since} ago");
        }
        (WaitKind::Unclaimed | WaitKind::Ready, Some(since)) => {
            let _ = write!(line, " · ready {since}");
        }
        (_, Some(since)) => {
            let _ = write!(line, " · {since}");
        }
        (_, None) => {}
    }
    if let Some(until) = wait.until_unix_ms.filter(|until| *until > now) {
        let _ = write!(line, " · until in {}", describe_age(until - now));
    }
    if let Some(lease) = wait.lease_expires_at_unix_ms {
        if lease > now {
            let _ = write!(line, " · lease {}", describe_age(lease - now));
        } else {
            let _ = write!(line, " · lease ended {} ago", describe_age(now - lease));
        }
    }
    if let Some(attempts) = wait.wake_attempts {
        let _ = write!(line, " · woken {attempts}×");
    }
    if let Some(detail) = &wait.detail {
        let _ = write!(line, " · {detail}");
    }
    line
}

fn settled_line(step: &StepSettled, now: u128) -> String {
    format!("{:<12} {} · {} ago", step.state, step.path, describe_age(now.saturating_sub(step.at_unix_ms)))
}

pub fn render_run_waiting_on(view: &RunWaitingOn) -> String {
    let now = view.as_of_unix_ms;
    let verdict = serde_json::to_value(view.verdict).ok();
    let verdict = verdict.as_ref().and_then(|value| value.as_str()).unwrap_or("unknown");
    let mut out = String::new();
    let _ = writeln!(out, "RUN      {}", view.run);
    let _ = writeln!(out, "MISSION  {}", view.mission);
    let _ = writeln!(out, "STATE    {} · {verdict}", view.status);
    let _ = writeln!(out, "WHY      {}", view.reason);
    let _ = writeln!(
        out,
        "READ     published work at index {} · stuck after {} quiet",
        view.work_cut,
        describe_age(u128::from(view.quiet_after_ms))
    );
    if !view.open.is_empty() {
        let _ = writeln!(out, "\nWAITING ON");
        for wait in &view.open {
            let _ = writeln!(out, "  {}", step_line(wait, now));
        }
    }
    if !view.pending.is_empty() {
        let _ = writeln!(out, "\nNOT READY");
        for step in &view.pending {
            let _ = writeln!(out, "  {}", settled_line(step, now));
        }
    }
    if !view.finished.is_empty() {
        let _ = writeln!(out, "\nFINISHED");
        for step in view.finished.iter().take(FINISHED_SHOWN) {
            let _ = writeln!(out, "  {}", settled_line(step, now));
        }
        if view.finished.len() > FINISHED_SHOWN {
            let _ = writeln!(out, "  … {} more", view.finished.len() - FINISHED_SHOWN);
        }
    }
    out
}

pub fn render_blocked_runs(view: &BlockedRuns) -> String {
    let now = view.as_of_unix_ms;
    let blocked = view.groups.iter().map(|group| group.runs.len()).sum::<usize>();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "BLOCKED RUNS  {blocked} of {} runs with open work · stuck after {} quiet · published work at index {}",
        view.runs_read,
        describe_age(u128::from(view.quiet_after_ms)),
        view.work_cut
    );
    for group in &view.groups {
        let _ = writeln!(out, "\n{} ({})", group.cause.heading(), group.runs.len());
        for run in &group.runs {
            let more = match run.blocked_steps {
                0 | 1 => String::new(),
                steps => format!(" · {steps} steps blocked"),
            };
            let _ = writeln!(out, "  {}  {}{more}", short(&run.run), step_line(&run.step, now));
        }
    }
    if blocked == 0 {
        let _ = writeln!(out, "\nNothing is blocked.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fractal_daemon::waiting_on::{BlockedGroup, BlockedRun, RunVerdict};

    const NOW: u128 = 10_000_000_000;
    const MINUTE: u128 = 60_000;

    fn wait(kind: WaitKind, path: &str, who: &str, ago: u128) -> StepWait {
        StepWait {
            step: format!("step-run/g1/{path}"),
            path: path.into(),
            title: None,
            state: "working".into(),
            kind,
            who: Some(who.into()),
            seat_state: Some("running".into()),
            since_unix_ms: Some(NOW - ago),
            lease_expires_at_unix_ms: None,
            until_unix_ms: None,
            detail: None,
            wake_attempts: None,
        }
    }

    #[test]
    fn a_run_page_says_why_then_each_open_step_then_what_finished() {
        let mut quiet = wait(WaitKind::NoProgress, "plant", "agent/garden/ash", 95 * MINUTE);
        quiet.lease_expires_at_unix_ms = Some(NOW + 8 * MINUTE);
        quiet.detail = Some("watering the north bed".into());
        let mut stopped = wait(WaitKind::SeatDown, "water", "agent/garden/birch", 40 * MINUTE);
        stopped.seat_state = Some("stopped".into());
        let view = RunWaitingOn {
            run: "mission-run/garden/alpha/1".into(),
            mission: "mission/garden/alpha".into(),
            status: "running".into(),
            generation: "run-generation/g1".into(),
            verdict: RunVerdict::Stalled,
            cause: Some(WaitKind::SeatDown),
            reason: "water: agent/garden/birch is stopped, and would take it".into(),
            open: vec![stopped, quiet],
            pending: vec![],
            finished: (0..12)
                .map(|index| StepSettled {
                    step: format!("step-run/g1/seed-{index}"),
                    path: format!("seed-{index}"),
                    title: None,
                    state: "completed".into(),
                    at_unix_ms: NOW - 3 * 60 * MINUTE,
                })
                .collect(),
            quiet_after_ms: 30 * 60_000,
            work_cut: 42,
            as_of_unix_ms: NOW,
        };
        let page = render_run_waiting_on(&view);
        assert!(page.contains("STATE    running · stalled\n"), "{page}");
        assert!(page.contains("WHY      water: agent/garden/birch is stopped"), "{page}");
        assert!(page.contains("  seat-down    water · agent/garden/birch (stopped) · 40m\n"), "{page}");
        assert!(
            page.contains(
                "  no-progress  plant · agent/garden/ash · last progress 1h 35m ago · lease 8m · watering the north bed\n"
            ),
            "{page}"
        );
        assert!(page.contains("  completed    seed-0 · 3h ago\n"), "{page}");
        assert!(page.contains("  … 2 more\n"), "{page}");
        assert!(page.find("WAITING ON") < page.find("FINISHED"));
    }

    #[test]
    fn the_fleet_list_groups_one_line_per_run_under_its_cause() {
        let mut ask = wait(WaitKind::Ask, "review", "person/ada", 2 * 60 * MINUTE);
        ask.seat_state = None;
        ask.detail = Some("Which beds stay?".into());
        let view = BlockedRuns {
            groups: vec![BlockedGroup {
                cause: WaitKind::Ask,
                runs: vec![BlockedRun {
                    run: "mission-run/garden/review/7".into(),
                    mission: Some("mission/garden/review".into()),
                    cause: WaitKind::Ask,
                    verdict: RunVerdict::Waiting,
                    step: ask,
                    blocked_steps: 2,
                }],
            }],
            runs_read: 9,
            quiet_after_ms: 30 * 60_000,
            work_cut: 42,
            as_of_unix_ms: NOW,
        };
        let page = render_blocked_runs(&view);
        assert!(page.starts_with("BLOCKED RUNS  1 of 9 runs with open work · stuck after 30m quiet"), "{page}");
        assert!(page.contains("\nwaiting on a person (1)\n"), "{page}");
        assert!(
            page.contains(
                "  garden/review/7  ask          review · person/ada · 2h · Which beds stay? · 2 steps blocked\n"
            ),
            "{page}"
        );
        let empty = BlockedRuns { groups: vec![], ..view };
        assert!(render_blocked_runs(&empty).ends_with("\nNothing is blocked.\n"));
    }
}
