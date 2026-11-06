//! Sidebar rows for the runs st relates to an agent or person.
//!
//! st decides status, progress, ETA, who a run waits on, urgency, order and folding. This
//! module only turns each served row into display text; it never reorders, folds or ranks.

use fractal_client::{AgentRelatedRun, AgentRelatedRuns, RelatedRunWait};
use serde_json::Value;

/// What the row shows where the ETA goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Eta {
    /// The run waits on the reader; this replaces any ETA.
    WaitingOnYou,
    /// Remaining time, such as `12m` or `12m–1h`.
    Remaining(String),
    /// st served no estimate.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

impl Progress {
    /// `done/total`.
    pub fn text(self) -> String {
        format!("{}/{}", self.done, self.total)
    }

    /// A bar `width` cells wide; a run with no steps is empty.
    pub fn bar(self, width: usize) -> String {
        let filled = if self.total == 0 { 0 } else {
            (width as u128 * self.done.min(self.total) as u128 / self.total as u128) as usize
        };
        "█".repeat(filled) + &"░".repeat(width - filled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelatedRunRow {
    pub id: String,
    pub mission_id: String,
    /// st's status chip word (`waiting-on-you`, `running`, `waiting`, `stalled`, or the final
    /// status of a folded run).
    pub status: String,
    pub progress: Option<Progress>,
    pub eta: Eta,
    /// Past its deadline: renderers color the ETA as late.
    pub late: bool,
    /// Finished within the last day; renderers show it collapsed.
    pub folded: bool,
    /// The step the run is on, by title or path.
    pub step: Option<String>,
    /// Who or what the open steps wait on, in st's order without repeats.
    pub waits_on: Vec<String>,
}

impl RelatedRunRow {
    pub fn from_run(run: &AgentRelatedRun) -> Self {
        let mut waits_on = Vec::new();
        for wait in &run.waits_on {
            let label = wait.who.clone().unwrap_or_else(|| wait.kind.clone());
            if !waits_on.contains(&label) {
                waits_on.push(label);
            }
        }
        Self {
            id: run.id.clone(),
            mission_id: run.mission_id.clone(),
            status: run.status_chip.clone(),
            progress: progress(&run.progress),
            eta: if run.waiting_on_you { Eta::WaitingOnYou } else { run.eta.as_ref().and_then(eta).map_or(Eta::Unknown, Eta::Remaining) },
            late: run.late,
            folded: run.folded,
            step: run.current_step.as_ref().map(step_label),
            waits_on,
        }
    }

    /// The second line: `step · waits on a, b`, or whichever half is known.
    pub fn detail(&self) -> Option<String> {
        let waits = (!self.waits_on.is_empty()).then(|| format!("waits on {}", self.waits_on.join(", ")));
        match (&self.step, waits) {
            (Some(step), Some(waits)) => Some(format!("{step} · {waits}")),
            (step, waits) => step.clone().or(waits),
        }
    }
}

/// Every row, in the order st served them.
pub fn rows(runs: &AgentRelatedRuns) -> Vec<RelatedRunRow> {
    runs.rows.iter().map(RelatedRunRow::from_run).collect()
}

fn step_label(wait: &RelatedRunWait) -> String {
    wait.title.clone().unwrap_or_else(|| wait.path.clone())
}

fn progress(value: &Value) -> Option<Progress> {
    Some(Progress { done: value["done"].as_u64()?, total: value["total"].as_u64()? })
}

fn eta(value: &Value) -> Option<String> {
    let (low, high) = (span(value["remaining_low_ms"].as_u64()?), span(value["remaining_high_ms"].as_u64()?));
    Some(if low == high { low } else { format!("{low}–{high}") })
}

/// A span such as `45s`, `12m`, `3h` or `2d`.
fn span(ms: u64) -> String {
    match ms / 1_000 {
        seconds @ 0..60 => format!("{seconds}s"),
        seconds @ 60..3_600 => format!("{}m", seconds / 60),
        seconds @ 3_600..172_800 => format!("{}h", seconds / 3_600),
        seconds => format!("{}d", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(fields: Value) -> AgentRelatedRun {
        let mut value = json!({
            "id":"mission-run/a", "mission_id":"mission/a", "status":"running", "status_chip":"running",
            "progress":{"done":1,"total":4}, "eta":{"remaining_low_ms":720_000,"remaining_high_ms":3_600_000},
            "waiting_on_you":false, "late":false, "deadline_at":null, "current_step":null, "waits_on":[],
            "urgency_rank":3, "folded":false, "completed_at":null
        });
        value.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn wait(path: &str, title: Option<&str>, kind: &str, who: Option<&str>) -> Value {
        json!({"step":format!("step-run/a/{path}"), "path":path, "title":title, "state":"ready", "kind":kind, "who":who})
    }

    #[test]
    fn a_moving_run_shows_its_chip_progress_and_eta_range() {
        let row = RelatedRunRow::from_run(&run(json!({})));
        assert_eq!(row.status, "running");
        assert_eq!(row.progress.unwrap().text(), "1/4");
        assert_eq!(row.progress.unwrap().bar(8), "██░░░░░░");
        assert_eq!(row.eta, Eta::Remaining("12m–1h".into()));
        assert!(!row.late && !row.folded);
        assert_eq!(row.detail(), None);
    }

    #[test]
    fn waiting_on_you_replaces_the_eta_and_late_is_served_not_derived() {
        let row = RelatedRunRow::from_run(&run(json!({"status_chip":"waiting-on-you", "waiting_on_you":true, "late":true})));
        assert_eq!(row.status, "waiting-on-you");
        assert_eq!(row.eta, Eta::WaitingOnYou);
        assert!(row.late);
        let unknown = RelatedRunRow::from_run(&run(json!({"eta":null, "progress":{}})));
        assert_eq!((unknown.eta, unknown.progress), (Eta::Unknown, None));
        let exact = RelatedRunRow::from_run(&run(json!({"eta":{"remaining_low_ms":5_000,"remaining_high_ms":5_000}})));
        assert_eq!(exact.eta, Eta::Remaining("5s".into()));
    }

    #[test]
    fn the_second_line_names_the_step_and_who_it_waits_on_once_each() {
        let row = RelatedRunRow::from_run(&run(json!({
            "current_step":wait("check", Some("Check"), "ask", Some("person/reviewer")),
            "waits_on":[wait("check", Some("Check"), "ask", Some("person/reviewer")),
                wait("build", None, "agent", None), wait("lint", None, "ask", Some("person/reviewer"))],
        })));
        assert_eq!(row.step.as_deref(), Some("Check"));
        assert_eq!(row.detail().as_deref(), Some("Check · waits on person/reviewer, agent"));
        let untitled = RelatedRunRow::from_run(&run(json!({"current_step":wait("build", None, "agent", None)})));
        assert_eq!(untitled.detail().as_deref(), Some("build"));
    }

    #[test]
    fn rows_keep_st_order_and_folding() {
        let runs: AgentRelatedRuns = serde_json::from_value(json!({
            "kind":"agent-related-runs", "agent_id":"agent/a", "as_of":1, "work_cut":1, "missions_cut":1,
            "total_rows":2, "truncated":false,
            "rows":[serde_json::to_value(run(json!({"id":"mission-run/z"}))).unwrap(),
                serde_json::to_value(run(json!({"id":"mission-run/b", "status_chip":"completed", "folded":true,
                    "eta":null, "progress":{"done":4,"total":4}}))).unwrap()]
        })).unwrap();
        let rows = rows(&runs);
        assert_eq!(rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["mission-run/z", "mission-run/b"]);
        assert!(rows[1].folded);
        assert_eq!(rows[1].progress.unwrap().bar(4), "████");
        assert_eq!(Progress { done: 0, total: 0 }.bar(3), "░░░");
    }
}
