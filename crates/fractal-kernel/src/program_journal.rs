//! What a program step's journal shows (docs/st3/program-engine.md §8): each inner step's latest
//! status, timing, failure and linked child, folded from the step's newest `program.step` claims.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The claim kind of a program journal entry, on the program's `step-run` subject.
pub const JOURNAL_KIND: &str = "program.step";
/// The reserved journal key the daemon writes on `done` with the program's encoded success value.
/// It is the program's result, not an inner step: replay and views skip it.
pub const RESULT_KEY: &str = "$result";
/// The fault on a program step whose journal shows no new entry within its `stalled-after`.
pub const STALLED_FAULT_SCOPE: &str = "program/stalled";
/// The fault on a program step whose host cannot run it, or whose worker stopped answering.
pub const HOST_FAULT_SCOPE: &str = "program/host";
/// How long a program step may go without a journal entry when it declares no `stalled-after`:
/// the mission-level default.
pub const DEFAULT_STALLED_AFTER_MS: u64 = 30 * 60 * 1000;
/// How many of a step's newest journal entries a view folds. A long loop's older keys are
/// counted out as `truncated`, never replayed for presentation.
pub const VIEW_ENTRIES: usize = 200;
/// How many of a step's newest journal entries the work list and the stall check read to name
/// the innermost open key.
pub const SUMMARY_ENTRIES: usize = 64;
/// How much of an entry's typed error a view keeps.
const ERROR_CHARS: usize = 300;

/// How long `program` may go without a new journal entry before it counts as stalled.
pub fn stalled_after_ms(program: &crate::model::ProgramSpec) -> u64 {
    program.stalled_after_ms.unwrap_or(DEFAULT_STALLED_AFTER_MS)
}

/// One inner step: the latest entry of its key in the current attempt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProgramEntry {
    pub key: String,
    /// `started`, `completed` or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_unix_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_ms: Option<u128>,
    /// The effect's duration, as the host measured it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// The typed failure, with its `_tag` and fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The child `mission-run/…` or ask subject the key waits on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
}

/// One attempt's journal, as far as a view reads it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProgramJournal {
    /// Each key's latest entry, in the order the keys first appear.
    #[serde(default)]
    pub entries: Vec<ProgramEntry>,
    /// The keys whose latest entry is `started`, most recently started first: the first is the
    /// innermost open key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_entry_at_unix_ms: Option<u128>,
    /// Older entries exist beyond the ones read.
    #[serde(default)]
    pub truncated: bool,
}

impl ProgramJournal {
    /// Fold one attempt's entries, newest first: each claim's `fields` and acceptance time.
    pub fn fold<'a>(newest_first: impl IntoIterator<Item = (&'a Value, u128)>, truncated: bool) -> Self {
        let mut journal = Self { truncated, ..Self::default() };
        // Newest first, so a key's first entry seen is its latest, and the last one seen orders it.
        let mut index = HashMap::<&str, usize>::new();
        let mut keys = Vec::<(usize, ProgramEntry)>::new();
        for (seen, (fields, at)) in newest_first.into_iter().enumerate() {
            journal.last_entry_at_unix_ms.get_or_insert(at);
            let Some(key) = fields.get("key").and_then(Value::as_str).filter(|key| *key != RESULT_KEY) else { continue };
            let status = fields.get("status").and_then(Value::as_str).unwrap_or_default();
            let slot = *index.entry(key).or_insert_with(|| {
                keys.push((
                    seen,
                    ProgramEntry {
                        key: key.to_owned(),
                        status: status.to_owned(),
                        started_at_unix_ms: None,
                        ended_at_unix_ms: (status != "started").then_some(at),
                        ms: fields.get("ms").and_then(Value::as_u64),
                        error: fields.get("error").filter(|error| !error.is_null()).map(error_text),
                        child: None,
                    },
                ));
                keys.len() - 1
            });
            let (oldest, entry) = &mut keys[slot];
            *oldest = seen;
            // A resumed effect starts again: its latest start is the one that runs.
            if status == "started" && entry.started_at_unix_ms.is_none() {
                entry.started_at_unix_ms = Some(at);
            }
            if entry.child.is_none() {
                entry.child = fields.get("child").and_then(Value::as_str).map(str::to_owned);
            }
        }
        keys.sort_by_key(|(oldest, _)| std::cmp::Reverse(*oldest));
        let mut open = keys
            .iter()
            .filter(|(_, entry)| entry.status == "started")
            .map(|(_, entry)| (entry.started_at_unix_ms, entry.key.clone()))
            .collect::<Vec<_>>();
        open.sort_by(|left, right| right.cmp(left));
        journal.open = open.into_iter().map(|(_, key)| key).collect();
        journal.entries = keys.into_iter().map(|(_, entry)| entry).collect();
        journal
    }

    /// The innermost open key's entry.
    pub fn innermost(&self) -> Option<&ProgramEntry> {
        let key = self.open.first()?;
        self.entries.iter().find(|entry| &entry.key == key)
    }
}

/// A program step of a run, with its current attempt's journal, as `missions show` and the run
/// view present it. The journal's fields sit at the top level.
///
/// Deserializing goes through `ProgramRunViewFields`: serde's `flatten` buffers the journal
/// through a value type that cannot hold its `u128` timestamps.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(from = "ProgramRunViewFields")]
pub struct ProgramRunView {
    pub step_run: String,
    pub path: String,
    pub attempt: u32,
    pub bundle_hash: String,
    /// The declared `stalled-after`, else [`DEFAULT_STALLED_AFTER_MS`].
    pub stalled_after_ms: u64,
    #[serde(flatten)]
    pub journal: ProgramJournal,
    /// The open `program/stalled` or `program/host` fault on the step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<ProgramFault>,
}

/// [`ProgramRunView`]'s wire fields, with the journal's spelled out.
#[derive(Deserialize)]
struct ProgramRunViewFields {
    step_run: String,
    path: String,
    attempt: u32,
    bundle_hash: String,
    stalled_after_ms: u64,
    #[serde(default)]
    entries: Vec<ProgramEntry>,
    #[serde(default)]
    open: Vec<String>,
    #[serde(default)]
    last_entry_at_unix_ms: Option<u128>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    fault: Option<ProgramFault>,
}

impl From<ProgramRunViewFields> for ProgramRunView {
    fn from(fields: ProgramRunViewFields) -> Self {
        let ProgramRunViewFields { step_run, path, attempt, bundle_hash, stalled_after_ms, entries, open, last_entry_at_unix_ms, truncated, fault } = fields;
        Self {
            step_run,
            path,
            attempt,
            bundle_hash,
            stalled_after_ms,
            journal: ProgramJournal { entries, open, last_entry_at_unix_ms, truncated },
            fault,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ProgramFault {
    pub scope: String,
    pub reason: String,
}

/// A typed failure keeps its tag and fields; a long one is cut.
fn error_text(error: &Value) -> String {
    let text = match error {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() <= ERROR_CHARS {
        return text;
    }
    let mut cut = text.chars().take(ERROR_CHARS).collect::<String>();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_journal_shows_each_keys_latest_entry_and_its_innermost_open_key() {
        // Newest first, as the store reads them.
        let rows = [
            (json!({"key": "review", "status": "started", "child": "mission-run/child"}), 60),
            (json!({"key": "check", "status": "started"}), 50),
            (json!({"key": "merge", "status": "failed", "ms": 900, "error": {"_tag": "Conflict", "path": "a.rs"}}), 40),
            (json!({"key": "merge", "status": "started"}), 30),
            (json!({"key": "render", "status": "completed", "ms": 120, "value": "ok"}), 20),
            (json!({"key": "render", "status": "started"}), 10),
        ];
        let journal = ProgramJournal::fold(rows.iter().map(|(fields, at)| (fields, *at)), false);
        let keys = journal.entries.iter().map(|entry| (entry.key.as_str(), entry.status.as_str())).collect::<Vec<_>>();
        assert_eq!(keys, [("render", "completed"), ("merge", "failed"), ("check", "started"), ("review", "started")]);
        assert_eq!(journal.open, ["review", "check"]);
        assert_eq!(journal.last_entry_at_unix_ms, Some(60));
        let render = &journal.entries[0];
        assert_eq!((render.started_at_unix_ms, render.ended_at_unix_ms, render.ms), (Some(10), Some(20), Some(120)));
        assert_eq!(journal.entries[1].error.as_deref(), Some(r#"{"_tag":"Conflict","path":"a.rs"}"#));
        let innermost = journal.innermost().unwrap();
        assert_eq!((innermost.key.as_str(), innermost.child.as_deref()), ("review", Some("mission-run/child")));
        assert_eq!(innermost.ended_at_unix_ms, None);
    }

    #[test]
    fn the_program_result_is_not_an_inner_step() {
        let rows = [
            (json!({"key": RESULT_KEY, "status": "completed", "value": {"merged": true}}), 30),
            (json!({"key": "merge", "status": "completed", "ms": 40}), 20),
            (json!({"key": "merge", "status": "started"}), 10),
        ];
        let journal = ProgramJournal::fold(rows.iter().map(|(fields, at)| (fields, *at)), false);
        let keys = journal.entries.iter().map(|entry| entry.key.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["merge"]);
        assert_eq!(journal.last_entry_at_unix_ms, Some(30));
    }

    #[test]
    fn a_long_error_is_cut() {
        let rows = [(json!({"key": "k", "status": "failed", "error": "x".repeat(400)}), 1)];
        let journal = ProgramJournal::fold(rows.iter().map(|(fields, at)| (fields, *at)), true);
        let error = journal.entries[0].error.as_deref().unwrap();
        assert_eq!(error.chars().count(), ERROR_CHARS + 1);
        assert!(journal.truncated && journal.open.is_empty());
    }

    fn timed_view() -> ProgramRunView {
        let rows = [
            (json!({"key": "merge", "status": "started", "child": "mission-run/child"}), 30),
            (json!({"key": "render", "status": "completed", "ms": 120}), 20),
            (json!({"key": "render", "status": "started"}), 10),
        ];
        ProgramRunView {
            step_run: "step-run/g/run".into(),
            path: "run".into(),
            attempt: 1,
            bundle_hash: "0".repeat(64),
            stalled_after_ms: DEFAULT_STALLED_AFTER_MS,
            journal: ProgramJournal::fold(rows.iter().map(|(fields, at)| (fields, *at)), false),
            fault: Some(ProgramFault { scope: STALLED_FAULT_SCOPE.into(), reason: "no entry".into() }),
        }
    }

    #[test]
    fn a_program_run_view_keeps_its_journal_fields_at_the_top_level_and_round_trips() {
        let view = timed_view();
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["last_entry_at_unix_ms"], 30);
        assert_eq!(value["open"], json!(["merge"]));
        assert_eq!(value["entries"][0]["started_at_unix_ms"], 10);
        assert!(value.get("journal").is_none());
        assert_eq!(serde_json::from_value::<ProgramRunView>(value).unwrap(), view);
    }

    #[test]
    fn a_mission_run_page_with_a_timed_program_step_round_trips() {
        let run = json!({
            "subject": "mission-run/g", "id": "g", "mission": "m", "generation": "g",
            "initial_revision": "1", "revision": "2", "root_revision": "1", "root_mission_run": "mission-run/g",
            "workspace": "w", "requester": "person/owner", "mode": "run", "status": "running", "phase": "work",
            "created_at_unix_ms": 1, "updated_at_unix_ms": 2, "programs": [timed_view()],
        });
        let page = json!({"runs": [run], "has_more": false, "frontier": 1});
        let decoded = serde_json::from_value::<crate::model::MissionRunTreePage>(page).unwrap();
        assert_eq!(decoded.runs[0].programs, [timed_view()]);
    }
}
