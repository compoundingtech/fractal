//! A step's attempts, waits and transitions, folded at read time from the durable lifecycle
//! claims of its `step-run/...` subject for the mission detail read. Nothing is stored.
//!
//! The store bounds each lifecycle kind's input before reading bodies, retaining sparse boundary
//! context independently of frequent progress notes. Lease renewals are excluded. Clockless
//! progress notes do not move the fold. Every read kind is durable; the store marks incomplete
//! input windows, and the fold also marks histories shortened by the output caps below.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

/// The newest attempts a step's history keeps.
pub const ATTEMPTS_LIMIT: usize = 20;
/// The newest waits a step's history keeps.
pub const WAITS_LIMIT: usize = 40;
/// The newest transitions a step's history keeps.
pub const TRANSITIONS_LIMIT: usize = 60;
/// Newest claims retained per lifecycle kind, plus one older boundary event. Keeping each kind's
/// suffix independently retains sparse state/claim/ask context when progress notes are frequent.
pub const CLAIMS_PER_KIND_LIMIT: usize = 128;

/// The claim kinds the history fold reads. `work.progress` rows are read only with a `clock`.
pub const STEP_HISTORY_KINDS: &str = "'step-run.state','step-run.carried','step-run.retried',\
    'work.claimed','work.progress','work.submitted','work.failed','work.released',\
    'work.person-asked','work.person-done','work.person-cancelled','work.nudged'";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StepHistory {
    pub attempts: Vec<StepAttempt>,
    pub waits: Vec<StepWait>,
    pub transitions: Vec<StepTransition>,
    /// False when a cap dropped older attempts, waits or transitions.
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StepAttempt {
    pub n: u64,
    pub claimant: Option<String>,
    pub started_at: Option<u128>,
    pub ended_at: Option<u128>,
    /// `done`, `failed`, `cancelled` or `running`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StepWait {
    /// `you`, `paused`, `retry`, `dependency` or `queue`.
    pub kind: &'static str,
    pub attempt: u64,
    pub start: u128,
    pub end: Option<u128>,
    /// The deadline the wait declared: a pause's `wait_until`, a retry's `not_before`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StepTransition {
    pub at: u128,
    /// The step's state after this event, in the `MissionStep.state` vocabulary.
    pub status: String,
    pub attempt: u64,
    /// The event that moved it: `state` (including a terminal person response), `carried`,
    /// `retried`, `claimed`, `submitted`, `failed`, `released`, or `nudged` (a stall marker
    /// that leaves the state as it was).
    pub event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// For `nudged`: since when the holder had been idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_since: Option<u128>,
}

/// A person ask made on another step (a runtime person step) that names this step as its origin:
/// its open and close times and the attempt of this step it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignHold {
    pub origin_attempt: u64,
    pub start: u128,
    pub end: Option<u128>,
    pub reason: Option<String>,
}

fn text(fields: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn millis(value: &Value) -> Option<u128> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .map(u128::from)
}

fn client_state(status: &str) -> String {
    fractal_store::client_values::client_work_state(status).to_owned()
}

fn outcome_of(status: &str) -> Option<&'static str> {
    match status {
        "completed" => Some("done"),
        "failed" => Some("failed"),
        "cancelled" | "skipped" => Some("cancelled"),
        _ => None,
    }
}

/// Open waits by kind, at most one of each, keyed so that `you` waits can stand per ask key.
#[derive(Default)]
struct OpenWaits(BTreeMap<(&'static str, String), StepWait>);

impl OpenWaits {
    fn open(&mut self, key: String, wait: StepWait, closed: &mut Vec<StepWait>) {
        if let Some(previous) = self.0.remove(&(wait.kind, key.clone())) {
            closed.push(StepWait {
                end: Some(wait.start),
                ..previous
            });
        }
        self.0.insert((wait.kind, key), wait);
    }
    fn close(&mut self, kind: &'static str, at: u128, closed: &mut Vec<StepWait>) {
        let keys = self
            .0
            .keys()
            .filter(|(open, _)| *open == kind)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            let wait = self.0.remove(&key).expect("an open wait");
            closed.push(StepWait {
                end: Some(at.max(wait.start)),
                ..wait
            });
        }
    }
    fn close_key(
        &mut self,
        kind: &'static str,
        key: &str,
        at: u128,
        closed: &mut Vec<StepWait>,
    ) -> bool {
        match self.0.remove(&(kind, key.to_owned())) {
            Some(wait) => {
                closed.push(StepWait {
                    end: Some(at.max(wait.start)),
                    ..wait
                });
                true
            }
            None => false,
        }
    }
}

/// Fold one step's lifecycle claims (canonical order) into its history as of `snapshot_unix_ms`.
/// `holds` are person asks other steps made on this step's behalf.
pub fn fold_step_history(
    events: &[(String, Value, u128)],
    holds: &[ForeignHold],
    snapshot_unix_ms: u128,
) -> StepHistory {
    let mut attempts = BTreeMap::<u64, StepAttempt>::new();
    let mut waits = Vec::<StepWait>::new();
    let mut open = OpenWaits::default();
    let mut transitions = Vec::<StepTransition>::new();
    let mut current_attempt = 1_u64;
    let mut status = String::new();
    // A retry's backoff ends at `not_before`; a ready step queues only after it.
    let mut retry_until: Option<u128> = None;
    let mut paused_until: Option<u128> = None;

    fn end_attempt(
        attempt: &mut StepAttempt,
        at: u128,
        outcome: &'static str,
        reason: Option<String>,
    ) {
        if attempt.ended_at.is_none() {
            attempt.ended_at = Some(at);
            attempt.outcome = outcome;
        }
        if attempt.reason.is_none() {
            attempt.reason = reason;
        }
    }

    for (kind, body, at) in events {
        let at = *at;
        let fields = body.get("fields").unwrap_or(body);
        let field_attempt = fields.get("attempt").and_then(Value::as_u64);
        if matches!(
            kind.as_str(),
            "step-run.state" | "step-run.carried" | "step-run.retried"
        ) && let Some(value) = field_attempt
        {
            current_attempt = value;
        }
        let attempt = field_attempt.unwrap_or(current_attempt);
        let reason = text(fields, &["reason", "summary"]);

        // A late reconciliation or claim must not extend the declared retry backoff.
        if let Some(deadline) = retry_until
            && at >= deadline
        {
            open.close("retry", deadline, &mut waits);
            retry_until = None;
        }

        // A declared pause ends on its own at its deadline.
        if let Some(deadline) = paused_until
            && at > deadline
        {
            open.close("paused", deadline, &mut waits);
            paused_until = None;
        }

        let mut moved: Option<(String, &'static str)> = None;
        match kind.as_str() {
            "step-run.state" | "step-run.carried" | "step-run.retried" => {
                let Some(next) = fields.get("status").and_then(Value::as_str) else {
                    continue;
                };
                let event = match kind.as_str() {
                    "step-run.state" => "state",
                    "step-run.carried" => "carried",
                    _ => "retried",
                };
                if kind == "step-run.retried" {
                    // The retried claim opens the next attempt; the one before it ended here.
                    if let Some(previous) = attempt.checked_sub(1).filter(|n| *n > 0)
                        && let Some(entry) = attempts.get_mut(&previous)
                    {
                        end_attempt(entry, at, "failed", reason.clone());
                    }
                    open.close("paused", at, &mut waits);
                    paused_until = None;
                    retry_until = fields
                        .get("not_before_unix_ms")
                        .and_then(millis)
                        .filter(|until| *until > at);
                    if let Some(until) = retry_until {
                        open.open(
                            String::new(),
                            StepWait {
                                kind: "retry",
                                attempt,
                                start: at,
                                end: None,
                                until: Some(until),
                                reason: reason.clone(),
                            },
                            &mut waits,
                        );
                    }
                }
                if matches!(next, "claimed" | "working") {
                    let entry = attempt_mut(&mut attempts, attempt);
                    entry.started_at.get_or_insert(at);
                    if let Some(claimant) = fields.get("claimant").and_then(Value::as_str) {
                        entry.claimant = Some(claimant.to_owned());
                    }
                }
                if let Some(outcome) = outcome_of(next) {
                    let entry = attempt_mut(&mut attempts, attempt);
                    end_attempt(entry, at, outcome, reason.clone());
                }
                moved = Some((next.to_owned(), event));
            }
            "work.claimed" => {
                let entry = attempt_mut(&mut attempts, attempt);
                entry.started_at.get_or_insert(at);
                if let Some(claimant) = fields.get("claimant").and_then(Value::as_str) {
                    entry.claimant = Some(claimant.to_owned());
                }
                // A new claim ends any wait the previous holder declared.
                open.close("paused", at, &mut waits);
                paused_until = None;
                moved = Some(("claimed".to_owned(), "claimed"));
            }
            "work.submitted" => moved = Some(("verifying".to_owned(), "submitted")),
            "work.released" => moved = Some(("ready".to_owned(), "released")),
            "work.failed" => {
                let entry = attempt_mut(&mut attempts, attempt);
                end_attempt(entry, at, "failed", reason.clone());
                moved = Some(("failed".to_owned(), "failed"));
            }
            "work.progress" => match fields.get("clock").and_then(Value::as_str) {
                Some("paused") => {
                    if let Some(deadline) = fields
                        .get("wait_until_unix_ms")
                        .and_then(millis)
                        .filter(|deadline| *deadline > at)
                    {
                        // Declaring again while a wait runs ends it and starts the next span.
                        open.open(
                            String::new(),
                            StepWait {
                                kind: "paused",
                                attempt,
                                start: at,
                                end: None,
                                until: Some(deadline),
                                reason: reason.clone(),
                            },
                            &mut waits,
                        );
                        paused_until = Some(deadline);
                    }
                }
                Some("running") => {
                    open.close("paused", at, &mut waits);
                    paused_until = None;
                }
                _ => {}
            },
            "work.person-asked" => {
                if fields.pointer("/request/type").and_then(Value::as_str) != Some("update") {
                    let key = text(fields, &["key"]).unwrap_or_else(|| at.to_string());
                    open.open(
                        key,
                        StepWait {
                            kind: "you",
                            attempt,
                            start: at,
                            end: None,
                            until: None,
                            reason: text(fields, &["title", "reason"]),
                        },
                        &mut waits,
                    );
                }
            }
            "work.person-done" | "work.person-cancelled" => {
                let closed = text(fields, &["key"])
                    .is_some_and(|key| open.close_key("you", &key, at, &mut waits));
                if !closed {
                    open.close("you", at, &mut waits);
                }
                // A person step completes through its own answer.
                if let Some(next) = fields.get("status").and_then(Value::as_str)
                    && let Some(outcome) = outcome_of(next)
                {
                    let entry = attempt_mut(&mut attempts, attempt);
                    end_attempt(entry, at, outcome, reason.clone());
                    moved = Some((next.to_owned(), "state"));
                }
            }
            "work.nudged" => {
                let nudge = StepTransition {
                    at,
                    status: if status.is_empty() {
                        client_state("claimed")
                    } else {
                        client_state(&status)
                    },
                    attempt,
                    event: "nudged",
                    reason: reason.clone(),
                    idle_since: fields.get("idle_since_unix_ms").and_then(millis),
                };
                // Nudges of one idle stretch stand as one marker, so they never crowd the
                // state edges out of the cap: the latest nudge, since the earliest idle time.
                match transitions.last_mut() {
                    Some(last)
                        if last.event == "nudged"
                            && last.status == nudge.status
                            && last.attempt == nudge.attempt =>
                    {
                        last.at = nudge.at;
                        last.reason = nudge.reason.or(last.reason.take());
                        last.idle_since = last.idle_since.or(nudge.idle_since);
                    }
                    _ => transitions.push(nudge),
                }
            }
            _ => {}
        }

        let Some((next, event)) = moved else { continue };
        // Waits that the new state ends, and the ones it starts.
        match next.as_str() {
            "pending" => {
                open.close("queue", at, &mut waits);
                if retry_until.is_none_or(|until| until <= at) {
                    open.open(
                        String::new(),
                        StepWait {
                            kind: "dependency",
                            attempt,
                            start: at,
                            end: None,
                            until: None,
                            reason: reason.clone(),
                        },
                        &mut waits,
                    );
                }
            }
            "ready" => {
                open.close("dependency", at, &mut waits);
                let start = retry_until.map_or(at, |until| until.max(at));
                if retry_until.is_some_and(|until| until > at) {
                    // The backoff still runs; the queue starts when it ends.
                } else {
                    open.close("retry", at, &mut waits);
                }
                if start <= snapshot_unix_ms {
                    open.open(
                        String::new(),
                        StepWait {
                            kind: "queue",
                            attempt,
                            start,
                            end: None,
                            until: None,
                            reason: None,
                        },
                        &mut waits,
                    );
                }
            }
            "claimed" | "working" | "verifying" | "blocked" => {
                for kind in ["dependency", "queue", "retry"] {
                    open.close(kind, at, &mut waits);
                }
                retry_until = None;
                if next != "claimed" && next != "working" {
                    open.close("paused", at, &mut waits);
                    paused_until = None;
                }
            }
            terminal if outcome_of(terminal).is_some() => {
                // Terminal: everything still open ends here.
                for kind in ["dependency", "queue", "retry", "paused", "you"] {
                    open.close(kind, at, &mut waits);
                }
                retry_until = None;
                paused_until = None;
            }
            // `waiting-person` and any later word: the ask that caused it is its own wait.
            _ => {}
        }
        let status_word = client_state(&next);
        let repeated = transitions
            .iter()
            .rev()
            .find(|transition| transition.event != "nudged")
            .is_some_and(|last| last.status == status_word && last.attempt == attempt);
        status = next;
        if !repeated {
            transitions.push(StepTransition {
                at,
                status: status_word,
                attempt,
                event,
                reason,
                idle_since: None,
            });
        }
    }

    // A pause past its deadline, and a retry past its backoff, ended on their own.
    if let Some(deadline) = paused_until
        && snapshot_unix_ms > deadline
    {
        open.close("paused", deadline, &mut waits);
    }
    let still_open =
        std::mem::take(&mut open.0)
            .into_values()
            .map(|wait| match (wait.kind, wait.until) {
                ("retry", Some(until)) if until <= snapshot_unix_ms => StepWait {
                    end: Some(until),
                    ..wait
                },
                _ => wait,
            });
    waits.extend(still_open);
    for hold in holds {
        waits.push(StepWait {
            kind: "you",
            attempt: hold.origin_attempt,
            start: hold.start,
            end: hold.end,
            until: None,
            reason: hold.reason.clone(),
        });
    }
    waits.sort_by_key(|wait| (wait.start, wait.end.unwrap_or(u128::MAX), wait.kind));

    let mut attempts = attempts.into_values().collect::<Vec<_>>();
    let complete = attempts.len() <= ATTEMPTS_LIMIT
        && waits.len() <= WAITS_LIMIT
        && transitions.len() <= TRANSITIONS_LIMIT;
    keep_newest(&mut attempts, ATTEMPTS_LIMIT);
    keep_newest(&mut waits, WAITS_LIMIT);
    keep_newest(&mut transitions, TRANSITIONS_LIMIT);
    StepHistory {
        attempts,
        waits,
        transitions,
        complete,
    }
}

fn attempt_mut(attempts: &mut BTreeMap<u64, StepAttempt>, n: u64) -> &mut StepAttempt {
    attempts.entry(n).or_insert_with(|| StepAttempt {
        n,
        claimant: None,
        started_at: None,
        ended_at: None,
        outcome: "running",
        reason: None,
    })
}

fn keep_newest<T>(items: &mut Vec<T>, limit: usize) {
    if items.len() > limit {
        items.drain(..items.len() - limit);
    }
}

/// Person asks in `events_by_subject` made on other steps that name one of these steps as their
/// origin, keyed by the origin step.
pub fn foreign_holds(
    events_by_subject: &BTreeMap<String, Vec<(String, Value, u128)>>,
) -> BTreeMap<String, Vec<ForeignHold>> {
    let subjects = events_by_subject.keys().cloned().collect::<BTreeSet<_>>();
    let mut holds = BTreeMap::<String, Vec<ForeignHold>>::new();
    for (subject, events) in events_by_subject {
        for (index, (kind, body, at)) in events.iter().enumerate() {
            let fields = body.get("fields").unwrap_or(body);
            let Some(origin) = fields.get("origin_step").and_then(Value::as_str) else {
                continue;
            };
            if kind != "work.person-asked"
                || origin == subject
                || !subjects.contains(origin)
                || fields.pointer("/request/type").and_then(Value::as_str) == Some("update")
            {
                continue;
            }
            let end = events[index + 1..]
                .iter()
                .find(|(kind, body, _)| {
                    matches!(kind.as_str(), "work.person-done" | "work.person-cancelled")
                        || (kind == "step-run.state"
                            && body.pointer("/fields/status").and_then(Value::as_str)
                                .and_then(outcome_of).is_some())
                })
                .map(|(_, _, at)| *at);
            holds
                .entry(origin.to_owned())
                .or_default()
                .push(ForeignHold {
                    origin_attempt: fields
                        .get("origin_attempt")
                        .and_then(Value::as_u64)
                        .unwrap_or(1),
                    start: *at,
                    end,
                    reason: text(fields, &["title", "reason"]),
                });
        }
    }
    holds
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, fields: Value, at: u128) -> (String, Value, u128) {
        (kind.to_owned(), json!({ "fields": fields }), at)
    }

    #[test]
    fn a_failed_attempt_keeps_its_reason_and_the_retry_backoff_is_a_wait() {
        let events = vec![
            event(
                "step-run.state",
                json!({"status": "pending", "attempt": 1}),
                5,
            ),
            event(
                "step-run.state",
                json!({"status": "ready", "attempt": 1}),
                10,
            ),
            event(
                "work.claimed",
                json!({"attempt": 1, "claimant": "agent/one"}),
                20,
            ),
            event(
                "step-run.state",
                json!({"status": "working", "attempt": 1}),
                20,
            ),
            event(
                "work.failed",
                json!({"attempt": 1, "reason": "tests failed"}),
                50,
            ),
            event(
                "step-run.state",
                json!({"status": "failed", "attempt": 1}),
                50,
            ),
            event(
                "step-run.retried",
                json!({"status": "pending", "attempt": 2, "reason": "retry after failure", "not_before_unix_ms": 80}),
                60,
            ),
            event(
                "step-run.state",
                json!({"status": "ready", "attempt": 2}),
                80,
            ),
            event(
                "work.claimed",
                json!({"attempt": 2, "claimant": "agent/two"}),
                90,
            ),
            event("work.submitted", json!({"attempt": 2}), 120),
            event(
                "step-run.state",
                json!({"status": "completed", "attempt": 2}),
                130,
            ),
        ];
        let history = fold_step_history(&events, &[], 200);
        assert!(history.complete);
        assert_eq!(
            history.attempts,
            vec![
                StepAttempt {
                    n: 1,
                    claimant: Some("agent/one".into()),
                    started_at: Some(20),
                    ended_at: Some(50),
                    outcome: "failed",
                    reason: Some("tests failed".into())
                },
                StepAttempt {
                    n: 2,
                    claimant: Some("agent/two".into()),
                    started_at: Some(90),
                    ended_at: Some(130),
                    outcome: "done",
                    reason: None
                },
            ]
        );
        let waits = history
            .waits
            .iter()
            .map(|wait| (wait.kind, wait.attempt, wait.start, wait.end))
            .collect::<Vec<_>>();
        assert_eq!(
            waits,
            vec![
                ("dependency", 1, 5, Some(10)),
                ("queue", 1, 10, Some(20)),
                ("retry", 2, 60, Some(80)),
                ("queue", 2, 80, Some(90)),
            ]
        );
        let retry = history
            .waits
            .iter()
            .find(|wait| wait.kind == "retry")
            .unwrap();
        assert_eq!(
            (retry.until, retry.reason.as_deref()),
            (Some(80), Some("retry after failure"))
        );
        let states = history
            .transitions
            .iter()
            .map(|t| (t.status.as_str(), t.attempt, t.event))
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                ("waiting", 1, "state"),
                ("ready", 1, "state"),
                ("claimed", 1, "claimed"),
                ("failed", 1, "failed"),
                ("waiting", 2, "retried"),
                ("ready", 2, "state"),
                ("claimed", 2, "claimed"),
                ("verifying", 2, "submitted"),
                ("completed", 2, "state"),
            ]
        );
    }

    #[test]
    fn a_retry_backoff_still_running_stays_open_and_ends_at_not_before_once_passed() {
        let events = vec![
            event(
                "work.claimed",
                json!({"attempt": 1, "claimant": "agent/one"}),
                10,
            ),
            event("work.failed", json!({"attempt": 1, "reason": "boom"}), 20),
            event(
                "step-run.retried",
                json!({"status": "pending", "attempt": 2, "reason": "boom", "not_before_unix_ms": 100}),
                21,
            ),
        ];
        let open = fold_step_history(&events, &[], 50);
        let retry = open.waits.iter().find(|wait| wait.kind == "retry").unwrap();
        assert_eq!((retry.start, retry.end, retry.until), (21, None, Some(100)));
        let passed = fold_step_history(&events, &[], 150);
        assert_eq!(
            passed
                .waits
                .iter()
                .find(|wait| wait.kind == "retry")
                .unwrap()
                .end,
            Some(100)
        );
        for (kind, fields, at, expected_end) in [
            ("step-run.state", json!({"status": "ready", "attempt": 2}), 110, 100),
            ("work.claimed", json!({"attempt": 2, "claimant": "agent/two"}), 120, 100),
            ("work.claimed", json!({"attempt": 2, "claimant": "agent/two"}), 90, 90),
        ] {
            let mut events = events.clone();
            events.push(event(kind, fields, at));
            let history = fold_step_history(&events, &[], 150);
            let retry = history.waits.iter().find(|wait| wait.kind == "retry").unwrap();
            assert_eq!(retry.end, Some(expected_end), "{kind} at {at}");
        }
    }

    #[test]
    fn person_asks_and_declared_pauses_are_waits() {
        let events = vec![
            event(
                "work.claimed",
                json!({"attempt": 1, "claimant": "agent/one"}),
                10,
            ),
            event(
                "work.person-asked",
                json!({"attempt": 1, "key": "k1", "title": "Pick a name"}),
                20,
            ),
            event(
                "work.person-asked",
                json!({"attempt": 1, "key": "u1", "request": {"type": "update"}}),
                21,
            ),
            event(
                "work.person-done",
                json!({"attempt": 1, "key": "k1", "status": "answered"}),
                40,
            ),
            event(
                "work.progress",
                json!({"attempt": 1, "clock": "paused", "wait_until_unix_ms": 100, "summary": "waiting for CI"}),
                50,
            ),
            event(
                "work.progress",
                json!({"attempt": 1, "clock": "running"}),
                70,
            ),
            event(
                "work.progress",
                json!({"attempt": 1, "clock": "paused", "wait_until_unix_ms": 90}),
                80,
            ),
            event(
                "work.nudged",
                json!({"attempt": 1, "idle_since_unix_ms": "75", "reason": "idle"}),
                95,
            ),
        ];
        let history = fold_step_history(
            &events,
            &[ForeignHold {
                origin_attempt: 1,
                start: 30,
                end: None,
                reason: Some("Approve".into()),
            }],
            120,
        );
        let waits = history
            .waits
            .iter()
            .map(|wait| (wait.kind, wait.start, wait.end, wait.reason.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(
            waits,
            vec![
                ("you", 20, Some(40), Some("Pick a name")),
                ("you", 30, None, Some("Approve")),
                ("paused", 50, Some(70), Some("waiting for CI")),
                ("paused", 80, Some(90), None),
            ]
        );
        assert_eq!(history.attempts[0].outcome, "running");
        let nudge = history.transitions.last().unwrap();
        assert_eq!(
            (nudge.event, nudge.status.as_str(), nudge.idle_since),
            ("nudged", "claimed", Some(75))
        );
    }

    #[test]
    fn caps_keep_the_newest_attempts_and_say_the_history_is_incomplete() {
        let mut events = Vec::new();
        for n in 1..=25_u64 {
            let at = u128::from(n) * 100;
            events.push(event(
                "work.claimed",
                json!({"attempt": n, "claimant": "agent/one"}),
                at,
            ));
            events.push(event(
                "work.failed",
                json!({"attempt": n, "reason": format!("failure {n}")}),
                at + 10,
            ));
            events.push(event(
                "step-run.retried",
                json!({"status": "ready", "attempt": n + 1, "reason": "again"}),
                at + 20,
            ));
        }
        let history = fold_step_history(&events, &[], 10_000);
        assert!(!history.complete);
        assert_eq!(history.attempts.len(), ATTEMPTS_LIMIT);
        assert_eq!(history.attempts.first().unwrap().n, 6);
        assert_eq!(
            history.attempts.last().unwrap().reason.as_deref(),
            Some("failure 25")
        );
    }

    #[test]
    fn repeated_nudges_coalesce_and_keep_the_state_edges_within_the_cap() {
        let mut events = vec![event(
            "work.claimed",
            json!({"attempt": 1, "claimant": "agent/one"}),
            10,
        )];
        for n in 0..80_u128 {
            events.push(event(
                "work.nudged",
                json!({"attempt": 1, "idle_since_unix_ms": (15 + n).to_string(), "reason": "idle"}),
                20 + n,
            ));
        }
        events.push(event("work.submitted", json!({"attempt": 1}), 200));
        let history = fold_step_history(&events, &[], 300);
        assert!(history.complete);
        let shown = history
            .transitions
            .iter()
            .map(|transition| (transition.event, transition.at, transition.idle_since))
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            vec![
                ("claimed", 10, None),
                ("nudged", 99, Some(15)),
                ("submitted", 200, None)
            ]
        );
    }

    #[test]
    fn foreign_person_steps_hold_their_origin_step() {
        let mut events = BTreeMap::new();
        events.insert(
            "step-run/r/g/build".to_owned(),
            vec![event("work.claimed", json!({"attempt": 2}), 10)],
        );
        events.insert("step-run/r/g/ask".to_owned(), vec![
            event("work.person-asked", json!({"origin_step": "step-run/r/g/build", "origin_attempt": 2, "title": "Approve?"}), 20),
            event("work.person-done", json!({"status": "completed"}), 35),
        ]);
        let holds = foreign_holds(&events);
        assert_eq!(
            holds["step-run/r/g/build"],
            vec![ForeignHold {
                origin_attempt: 2,
                start: 20,
                end: Some(35),
                reason: Some("Approve?".into())
            }]
        );
        assert!(!holds.contains_key("step-run/r/g/ask"));
    }
}
