//! Which items a reconcile pass must evaluate.
//!
//! Each item (a mission run, a stop, later live members and intake declarations) remembers what
//! its last evaluation read, and when its result could next change with time alone. A pass reads
//! what changed since the last one (claims, local observations, the PTY snapshot, watched files,
//! exec runners), turns each change into the keys a read would have noted, and marks the items that
//! read any of them. An item needs evaluating when it is new, marked, or due; the others are
//! skipped. Every [`FULL_PASS_INTERVAL_MS`] a section evaluates every item anyway, and an
//! evaluation that writes although its item was not marked is counted as a correction: a write
//! the skipping passes missed. See `doc/fleet/smalltalk/idle-cpu-incremental-design`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use anyhow::Result;
use serde_json::Value;

/// One new claim or local observation.
#[derive(Clone, Debug)]
pub struct Change {
    pub subject: String,
    pub kind: String,
    pub actor: Option<String>,
    pub body: String,
}

/// What a source of changes answers: the changes after an index and a local observation, and the
/// indexes the answer reaches.
#[derive(Clone, Debug, Default)]
pub struct ChangeFeed {
    pub index: u64,
    pub local: i64,
    pub changes: Vec<Change>,
}

/// The change feed a pass reads. The store implements it, so this module names no store code.
pub trait ChangeSource {
    /// What changed after store index `index` and local observation `local`: each new claim and
    /// local observation as subject, kind, actor and body.
    fn changes_since(&self, index: u64, local: i64) -> Result<ChangeFeed>;
}

impl<T: ChangeSource + ?Sized> ChangeSource for std::sync::Arc<T> {
    fn changes_since(&self, index: u64, local: i64) -> Result<ChangeFeed> {
        (**self).changes_since(index, local)
    }
}

/// The owner, repository, thread number and agent name of a `subscription/watch/…` subject.
pub fn split_watch_subject(subject: &str) -> Option<(&str, &str, u64, &str)> {
    let rest = subject.strip_prefix("subscription/watch/")?;
    let mut parts = rest.splitn(4, '/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    let number = parts.next()?.parse().ok()?;
    let agent = parts.next()?;
    Some((owner, repository, number, agent))
}

/// Shared, so a file watcher or timer can mark what it changed.
#[derive(Clone, Default)]
pub struct Incremental {
    state: std::sync::Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    /// The change feed's watermark; `None` until the first pass, which evaluates every item.
    watermark: Option<(u64, i64)>,
    items: HashMap<String, Item>,
    /// Which items read each key.
    readers: HashMap<String, BTreeSet<String>>,
    /// The last status seen for each exec runtime an item waits on (`exec:` keys).
    execs: HashMap<String, Option<String>>,
    /// When each section's last full pass ran.
    last_full: HashMap<&'static str, u128>,
    /// The last PTY snapshot, by runtime: `None` before the first, or while it was unavailable.
    ptys: Option<HashMap<String, String>>,
    /// The last value seen for each key observed by value, such as `live-workspaces`.
    values: HashMap<String, String>,
    /// When each polled key (such as a terminal's screen) was last looked at.
    polled_at: HashMap<String, u128>,
}

/// How often a pass evaluates every item even when nothing marked them, counting what it corrects.
/// Every five minutes. Every minute, a large node's full passes reached a third of the pass's
/// disk reads. A half hour was too long: some work is still reached only by a full pass (a loop
/// timeout, an LLM gate's time limit, trust-prompt recovery, rendered files, a gate after a
/// pending one), and the correction counter cannot see most of it. Lengthen this only after a
/// day in which the counter shows no correction of any kind. Each full pass is noted in the
/// profile with any correction it makes.
pub const FULL_PASS_INTERVAL_MS: u128 = 5 * 60_000;
/// Periodic work that is slow waits at least this many times its own cost; see
/// [`cost_scaled_interval`].
pub const FULL_PASS_COST_MULTIPLE: u128 = 10;

/// The interval for periodic work that costs `cost_ms` each time: at least `base_ms`, at least
/// [`FULL_PASS_COST_MULTIPLE`] times its cost, at most `max_ms`.
pub fn cost_scaled_interval(base_ms: u128, cost_ms: u128, max_ms: u128) -> u128 {
    cost_ms
        .saturating_mul(FULL_PASS_COST_MULTIPLE)
        .min(max_ms)
        .max(base_ms)
}

struct Item {
    reads: BTreeSet<String>,
    due: Option<u128>,
    dirty: bool,
}

/// The keys a change can affect: its subject, its actor's work, its kind, and for some kinds a
/// key a read notes for subjects it cannot name in advance.
pub fn change_keys(change: &Change) -> Vec<String> {
    let mut keys = vec![change.subject.clone(), format!("kind:{}", change.kind)];
    if let Some(actor) = &change.actor {
        keys.push(format!("actor:{actor}"));
    }
    let field = |name: &str| -> Option<String> {
        let body: Value = serde_json::from_str(&change.body).ok()?;
        body.get("fields")
            .unwrap_or(&body)
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match change.kind.as_str() {
        "message.sent" => keys.extend(field("to").map(|to| format!("mailbox:{to}"))),
        "mission-run.report-to" => {
            keys.extend(field("report_to").map(|agent| format!("reports-to:{agent}")));
        }
        "mission-run.created" => {
            keys.extend(field("report_to").map(|agent| format!("reports-to:{agent}")));
            keys.extend(field("root_mission_run").map(|root| format!("children:{root}")));
            keys.extend(field("parent_step_run").map(|step| format!("children-of-step:{step}")));
        }
        "agent.retired" => keys.push("kind:intent.desired".into()),
        "owned-set.revised" => {
            keys.push("kind:intent.desired".into());
            if let Ok(body) = serde_json::from_str::<Value>(&change.body) {
                for map in ["members", "retired"] {
                    if let Some(members) = body["fields"]["body"][map].as_object() {
                        keys.extend(members.keys().cloned());
                    }
                }
            }
        }
        "intent.desired" => {
            if let Some((_, _, _, agent)) = split_watch_subject(&change.subject) {
                keys.push(format!("watches:agent/{agent}"));
            }
            keys.extend(field("owner_run").map(|run| format!("owned:{run}")));
            keys.extend(field("owner_step").map(|step| format!("owned-step:{step}")));
            keys.extend(field("previous_owner_step").map(|step| format!("owned-step:{step}")));
            keys.extend(field("previous_owner_generation"));
            keys.extend(field("owner_generation"));
        }
        "run-generation.created" => {
            keys.extend(field("run").map(|run| format!("generations:{run}")));
        }
        "work.released" => {
            // The recipient may not have read this step before. The releasing actor
            // and old step subject already invalidate the previous holder's work.
            keys.extend(field("handoff_to").map(|actor| format!("actor:{actor}")));
        }
        "subscription.mission-started" => {
            keys.extend(field("mission_run").map(|run| format!("subscription-run:{run}")));
        }
        _ => {}
    }
    if matches!(change.kind.as_str(), "intent.desired" | "owned-set.revised" | "agent.retired")
        && keys.iter().any(|key| key.starts_with("agent/"))
    {
        keys.push("desired-kind:agent".into());
    }
    keys
}

impl Incremental {
    /// Read what changed since the last pass and mark the items that read it.
    pub fn observe<S: ChangeSource + ?Sized>(&self, store: &S) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((index, local)) = state.watermark else {
            let feed = store.changes_since(i64::MAX as u64, i64::MAX)?;
            state.watermark = Some((feed.index, feed.local));
            return Ok(());
        };
        let feed = store.changes_since(index, local)?;
        state.watermark = Some((feed.index, feed.local));
        let State { items, readers, .. } = &mut *state;
        for change in &feed.changes {
            for key in change_keys(change) {
                for item in readers.get(&key).into_iter().flatten() {
                    if let Some(item) = items.get_mut(item) {
                        item.dirty = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// Mark the items that read `key`, for a change that is not a claim, such as a watched file.
    pub fn touch(&self, key: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::mark_locked(&mut state, key);
    }

    /// Whether this pass must evaluate every item of `section`: the first since a start, or the
    /// first [`FULL_PASS_INTERVAL_MS`] after the section's last one. Records it as the section's
    /// last full pass, and notes it in the profile.
    pub fn take_full_pass(&self, section: &'static str, now: u128) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = state
            .last_full
            .get(section)
            .is_none_or(|last| now.saturating_sub(*last) >= FULL_PASS_INTERVAL_MS);
        if due {
            state.last_full.insert(section, now);
            smallclaims::profile::note("reconcile full pass");
        }
        due
    }

    /// Compare this pass's PTY snapshot (each runtime's state, as text) with the last one, and mark
    /// the items that read a runtime whose state changed (`pty:{runtime}`). When the snapshot comes
    /// back after it was unavailable, every item that read any runtime is marked.
    pub fn observe_ptys(&self, snapshot: Option<HashMap<String, String>>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(snapshot) = snapshot else {
            state.ptys = None;
            return;
        };
        let changed = match &state.ptys {
            Some(previous) => previous
                .iter()
                .filter(|(id, value)| snapshot.get(*id) != Some(value))
                .map(|(id, _)| format!("pty:{id}"))
                .chain(
                    snapshot
                        .keys()
                        .filter(|id| !previous.contains_key(*id))
                        .map(|id| format!("pty:{id}")),
                )
                .collect::<Vec<_>>(),
            None => state
                .readers
                .keys()
                .filter(|key| key.starts_with("pty:"))
                .cloned()
                .collect(),
        };
        state.ptys = Some(snapshot);
        for key in changed {
            Self::mark_locked(&mut state, &key);
        }
    }

    /// Mark the items that read `key` when `value` differs from the value last observed for it.
    pub fn observe_value(&self, key: &str, value: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.values.get(key) != Some(&value) {
            state.values.insert(key.to_owned(), value);
            Self::mark_locked(&mut state, key);
        }
    }

    /// Remember what an evaluation saw for a polled key, so a later poll marks its readers only
    /// when the value changes from that.
    pub fn saw_value(&self, key: &str, value: String, now: u128) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.values.insert(key.to_owned(), value);
        state.polled_at.insert(key.to_owned(), now);
    }

    /// Look again, at most every `every_ms`, at each key with `prefix` that an item read, and
    /// mark its readers when the value changed. `poll` gets the key without the prefix.
    pub fn poll_values(
        &self,
        prefix: &str,
        every_ms: u128,
        now: u128,
        poll: impl Fn(&str) -> Option<String>,
    ) {
        let keys = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .readers
                .keys()
                .filter(|key| key.starts_with(prefix))
                .filter(|key| {
                    state
                        .polled_at
                        .get(*key)
                        .is_none_or(|at| now.saturating_sub(*at) >= every_ms)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        for key in keys {
            let value = poll(&key[prefix.len()..]).unwrap_or_default();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.polled_at.insert(key.clone(), now);
            if state.values.get(&key) != Some(&value) {
                state.values.insert(key.clone(), value);
                Self::mark_locked(&mut state, &key);
            }
        }
    }

    fn mark_locked(state: &mut State, key: &str) {
        let State { items, readers, .. } = state;
        for item in readers.get(key).into_iter().flatten() {
            if let Some(item) = items.get_mut(item) {
                item.dirty = true;
            }
        }
    }

    /// The earliest time an item whose name starts with `prefix` comes due.
    pub fn next_due(&self, prefix: &str) -> Option<u128> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .items
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .filter_map(|(_, item)| item.due)
            .min()
    }

    /// When the next section's full pass is due: the earliest last full pass plus
    /// [`FULL_PASS_INTERVAL_MS`]. A due time an item failed to record still comes round then.
    pub fn next_full_pass(&self) -> Option<u128> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .last_full
            .values()
            .min()
            .map(|last| last.saturating_add(FULL_PASS_INTERVAL_MS))
    }

    /// When `item`'s last evaluation said it is next due, if it said.
    pub fn due_of(&self, item: &str) -> Option<u128> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .items
            .get(item)
            .and_then(|item| item.due)
    }

    /// What `item`'s last evaluation read.
    pub fn reads_of(&self, item: &str) -> BTreeSet<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .items
            .get(item)
            .map(|item| item.reads.clone())
            .unwrap_or_default()
    }

    /// Look again at every exec runtime an item read (`exec:{runtime}`), and mark the items that
    /// read one whose status changed since the last look.
    pub fn observe_execs(&self, observe: impl Fn(&str) -> Option<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys = state
            .readers
            .keys()
            .filter(|key| key.starts_with("exec:"))
            .cloned()
            .collect::<Vec<_>>();
        state.execs.retain(|key, _| keys.contains(key));
        for key in keys {
            let status = observe(&key["exec:".len()..]);
            let previous = state.execs.insert(key.clone(), status.clone());
            // Evaluations wait only on running runners, so the first look counts any other status
            // as a change: the runner may have stopped after the evaluation saw it run.
            let changed = match previous {
                Some(previous) => previous != status,
                None => status.as_deref() != Some("running"),
            };
            if changed {
                let readers = state.readers.get(&key).cloned().unwrap_or_default();
                for item in readers {
                    if let Some(item) = state.items.get_mut(&item) {
                        item.dirty = true;
                    }
                }
            }
        }
    }

    /// Whether `item` needs evaluating at `now`: it is new, something it read changed, or its time
    /// has come.
    pub fn needs(&self, item: &str, now: u128) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .items
            .get(item)
            .is_none_or(|item| item.dirty || item.due.is_some_and(|due| due <= now))
    }

    /// The items named with `prefix` that read something the feed has after `cursor`, in name
    /// order, and the cursor to pass next time. With no cursor it starts where the passes' own
    /// reading of the feed is. Unlike [`Self::observe`], it marks nothing and leaves the passes'
    /// place in the feed alone: an evaluation clears its item's mark, so a mark set from another
    /// thread while a pass evaluates that item would be lost until the next full pass.
    pub fn readers_since<S: ChangeSource + ?Sized>(
        &self,
        store: &S,
        cursor: Option<(u64, i64)>,
        prefix: &str,
    ) -> Result<((u64, i64), Vec<String>)> {
        let watermark = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .watermark;
        let Some((index, local)) = cursor.or(watermark) else {
            let feed = store.changes_since(i64::MAX as u64, i64::MAX)?;
            return Ok(((feed.index, feed.local), Vec::new()));
        };
        let feed = store.changes_since(index, local)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let readers = feed
            .changes
            .iter()
            .flat_map(change_keys)
            .flat_map(|key| state.readers.get(&key).into_iter().flatten())
            .filter(|item| item.starts_with(prefix))
            .cloned()
            .collect::<BTreeSet<_>>();
        Ok(((feed.index, feed.local), readers.into_iter().collect()))
    }

    pub fn evaluated(&self, item: &str, reads: BTreeSet<String>, due: Option<u128>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let State { items, readers, .. } = &mut *state;
        if let Some(previous) = items.get(item) {
            for key in &previous.reads {
                if let Some(set) = readers.get_mut(key) {
                    set.remove(item);
                    if set.is_empty() {
                        readers.remove(key);
                    }
                }
            }
        }
        for key in &reads {
            readers
                .entry(key.clone())
                .or_default()
                .insert(item.to_owned());
        }
        items.insert(
            item.to_owned(),
            Item {
                reads,
                due,
                dirty: false,
            },
        );
    }

    /// Forget items of `kind` (a subject prefix such as `mission-run/`) that are no longer active.
    pub fn retain(&self, prefix: &str, active: &BTreeSet<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let gone = state
            .items
            .keys()
            .filter(|item| item.starts_with(prefix) && !active.contains(*item))
            .cloned()
            .collect::<Vec<_>>();
        for item in gone {
            self.forget_locked(&mut state, &item);
        }
    }

    fn forget_locked(&self, state: &mut State, item: &str) {
        if let Some(previous) = state.items.remove(item) {
            for key in previous.reads {
                if let Some(set) = state.readers.get_mut(&key) {
                    set.remove(item);
                    if set.is_empty() {
                        state.readers.remove(&key);
                    }
                }
            }
        }
    }
}

pub use crate::otel::thread_cpu;

/// When a run's result could next change with time alone: the earliest lease expiry, retry
/// backoff, step timeout or run deadline still ahead of `now`.
pub fn run_due(run: &fractal_kernel::model::MissionRunView, now: u128) -> Option<u128> {
    run.deadline_at_unix_ms
        .filter(|time| *time > now)
        .into_iter()
        .chain(run.steps.iter().filter_map(|step| step_due(step, now)))
        .min()
}

/// When a step's view could next change with time alone: its lease expiry, retry backoff or
/// timeout, whichever comes first after `now`.
pub fn step_due(step: &fractal_kernel::model::StepRunView, now: u128) -> Option<u128> {
    let timeout =
        step.execution_started_at_unix_ms
            .zip(step.timeout_ms)
            .map(|(started, timeout)| {
                started
                    .saturating_add(u128::from(timeout))
                    .saturating_add(u128::from(step.timeout_extension_ms))
            });
    [
        step.claim_expires_at_unix_ms,
        step.not_before_unix_ms,
        step.execution_paused_until_unix_ms,
        timeout,
    ]
    .into_iter()
    .flatten()
    .filter(|time| *time > now)
    .min()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_section_is_evaluated_in_full_once_every_five_minutes() {
        use super::*;
        let incremental = Incremental::default();
        // The first pass after a start is full, and the reconciler wakes for the next one.
        assert!(incremental.take_full_pass("member", 0));
        assert_eq!(incremental.next_full_pass(), Some(5 * 60_000));
        // Every pass in between skips what nothing marked, however long the full pass took.
        for now in [30_000, 2 * 60_000, 5 * 60_000 - 1] {
            assert!(
                !incremental.take_full_pass("member", now),
                "full again at {now}"
            );
        }
        assert!(incremental.take_full_pass("member", 5 * 60_000));
        assert_eq!(incremental.next_full_pass(), Some(10 * 60_000));
        // Each section keeps its own five minutes.
        assert!(incremental.take_full_pass("stop", 6 * 60_000));
        assert!(!incremental.take_full_pass("stop", 10 * 60_000));
        assert!(incremental.take_full_pass("member", 10 * 60_000));
        assert_eq!(cost_scaled_interval(30_000, 0, 600_000), 30_000);
        assert_eq!(cost_scaled_interval(30_000, 25_000, 600_000), 250_000);
        assert_eq!(cost_scaled_interval(30_000, u128::MAX, 600_000), 600_000);
    }

    #[test]
    fn agent_collection_keys_ignore_message_declarations_and_cover_owned_set_members() {
        assert!(
            !super::change_keys(&change("message/m", "intent.desired", None, "{}"))
                .contains(&"desired-kind:agent".to_owned())
        );
        assert!(
            !super::change_keys(&change("agent/a", "harness.observed", None, "{}"))
                .contains(&"desired-kind:agent".to_owned())
        );
        assert!(
            super::change_keys(&change("agent/a", "intent.desired", None, "{}"))
                .contains(&"desired-kind:agent".to_owned())
        );
        assert!(
            super::change_keys(&change(
                "owned-set/x",
                "owned-set.revised",
                None,
                r#"{"fields":{"body":{"members":{"agent/a":{}}}}}"#
            ))
            .contains(&"desired-kind:agent".to_owned())
        );
    }

    use super::*;

    #[test]
    fn a_watch_subject_names_its_thread_and_seat() {
        assert_eq!(
            split_watch_subject("subscription/watch/acme/garden/12/example/planner"),
            Some(("acme", "garden", 12, "example/planner"))
        );
        assert_eq!(split_watch_subject("subscription/watch/acme/garden/x/planner"), None);
        assert_eq!(split_watch_subject("intent/acme/garden/12/planner"), None);
    }

    /// Every change so far; an index is how many of them a reader has seen.
    struct Feed(Mutex<Vec<Change>>);

    impl ChangeSource for Feed {
        fn changes_since(&self, index: u64, _local: i64) -> Result<ChangeFeed> {
            let changes = self.0.lock().unwrap();
            let seen = usize::try_from(index).unwrap_or(usize::MAX).min(changes.len());
            Ok(ChangeFeed {
                index: changes.len() as u64,
                local: 0,
                changes: changes[seen..].to_vec(),
            })
        }
    }

    #[test]
    fn readers_since_names_the_readers_of_a_change_and_leaves_the_passes_marks_alone() {
        let feed = Feed(Mutex::new(Vec::new()));
        let incremental = Incremental::default();
        incremental.observe(&feed).unwrap();
        let reads = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect();
        incremental.evaluated("mission-run/a", reads(&["resource/x"]), None);
        incremental.evaluated("mission-run/b", reads(&["resource/y"]), None);
        incremental.evaluated("stop/c", reads(&["resource/x"]), None);
        feed.0.lock().unwrap().push(change("resource/x", "resource.observed", None, "{}"));

        let (cursor, readers) = incremental.readers_since(&feed, None, "mission-run/").unwrap();
        assert_eq!(readers, ["mission-run/a"]);
        let (_, again) = incremental.readers_since(&feed, Some(cursor), "mission-run/").unwrap();
        assert!(again.is_empty());

        // It marked nothing, and the passes still read the change from where they were.
        assert!(!incremental.needs("stop/c", 0));
        incremental.observe(&feed).unwrap();
        assert!(incremental.needs("stop/c", 0));
        assert!(incremental.needs("mission-run/a", 0));
        assert!(!incremental.needs("mission-run/b", 0));
    }

    fn change(subject: &str, kind: &str, actor: Option<&str>, body: &str) -> Change {
        Change {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(Into::into),
            body: body.into(),
        }
    }
}
