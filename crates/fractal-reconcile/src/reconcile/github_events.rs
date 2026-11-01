//! The keyed events one change of a GitHub item holds, derived by comparing the item's facts in
//! the plugin's mirror with the facts a new look recorded. The poller keeps them in its mirror,
//! and from step 3b of the plugin store also writes each as one `github.event` claim. An item the
//! mirror never held is a baseline and holds no event, so a member that starts polling, or whose
//! mirror is new, wakes nobody.

use serde_json::{Map, Value, json};

/// One thing that happened to one item.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// What names this event among the item's events, such as `checks/SHA/fail` or `comment/42`.
    /// An event that can happen again on the same item takes the next free `KEY/N` (see
    /// [`occurrence_key`]).
    pub key: String,
    /// One of `fractal_schema::github_plugin::EVENTS`.
    pub event: &'static str,
    pub head_sha: Option<String>,
    /// A few small facts about it, such as a comment's author or the failed check names.
    pub detail: Map<String, Value>,
}

fn text<'a>(facts: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    facts.get(name).and_then(Value::as_str)
}

fn open(facts: &Map<String, Value>) -> bool {
    text(facts, "state") == Some("open")
}

fn merged(facts: &Map<String, Value>) -> bool {
    facts.get("merged").and_then(Value::as_bool) == Some(true)
}

fn in_queue(facts: &Map<String, Value>) -> bool {
    facts.get("merge_queue").is_some_and(|queue| !queue.is_null())
}

fn required_state(facts: &Map<String, Value>) -> Option<&str> {
    facts.get("required_checks").and_then(|checks| checks.get("state")).and_then(Value::as_str)
}

/// The events between an item's mirrored facts and its new ones, oldest first.
pub fn derive(old: Option<&Map<String, Value>>, new: &Map<String, Value>) -> Vec<Event> {
    let Some(old) = old else {
        return Vec::new();
    };
    let head = text(new, "head_sha").map(str::to_owned);
    let event = |key: String, event: &'static str, detail: Map<String, Value>| Event {
        key,
        event,
        head_sha: head.clone(),
        detail,
    };
    let mut events = Vec::new();
    if let (Some(before), Some(after)) = (text(old, "head_sha"), text(new, "head_sha"))
        && before != after
    {
        let mut detail = Map::new();
        detail.insert("from".into(), before.into());
        events.push(event(format!("head/{after}"), "head-changed", detail));
    }
    if let (Some(before), Some(after)) = (
        old.get("draft").and_then(Value::as_bool),
        new.get("draft").and_then(Value::as_bool),
    ) && before != after
    {
        let (key, name) = if after { ("became-draft", "became-draft") } else { ("became-ready", "became-ready") };
        events.push(event(key.into(), name, Map::new()));
    }
    if in_queue(old) != in_queue(new) && (open(new) || !in_queue(new)) {
        let (key, name, queue) = if in_queue(new) {
            ("merge-queue-entered", "merge-queue-entered", new.get("merge_queue"))
        } else {
            ("merge-queue-left", "merge-queue-left", old.get("merge_queue"))
        };
        let mut detail = Map::new();
        if let Some(position) = queue.and_then(|queue| queue.get("position")).filter(|value| !value.is_null()) {
            detail.insert("position".into(), position.clone());
        }
        events.push(event(key.into(), name, detail));
    }
    if let Some(now) = required_state(new).filter(|state| *state == "pass" || *state == "fail")
        && (text(old, "head_sha") != text(new, "head_sha") || required_state(old) != Some(now))
    {
        let mut detail = Map::new();
        if now == "fail"
            && let Some(failed) = new.get("required_checks").and_then(|checks| checks.get("failed"))
                .and_then(Value::as_array)
        {
            detail.insert(
                "failed".into(),
                Value::Array(failed.iter().take(fractal_schema::github_plugin::MAX_FAILED_CHECKS).cloned().collect()),
            );
        }
        let sha = head.as_deref().unwrap_or("unknown");
        let (key, name) = if now == "pass" {
            (format!("checks/{sha}/pass"), "checks-passed")
        } else {
            (format!("checks/{sha}/fail"), "checks-failed")
        };
        events.push(event(key, name, detail));
    }
    let known: std::collections::BTreeSet<(String, u64)> = old
        .get("recent_comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(crate::resource::recent_comment_key)
        .collect();
    for entry in new.get("recent_comments").and_then(Value::as_array).into_iter().flatten() {
        let (kind, id) = crate::resource::recent_comment_key(entry);
        if known.contains(&(kind.clone(), id)) {
            continue;
        }
        let mut detail = Map::new();
        for name in ["author", "at", "state"] {
            if let Some(value) = entry.get(name).filter(|value| !value.is_null()) {
                detail.insert(name.into(), value.clone());
            }
        }
        let name = if kind == "review" { "review" } else { "comment" };
        events.push(event(format!("{name}/{id}"), name, detail));
    }
    if open(old) && new.get("state").is_some() && !open(new) {
        if merged(new) {
            let mut detail = Map::new();
            if let Some(sha) = new.get("merge_commit_sha").filter(|value| !value.is_null()) {
                detail.insert("merge_commit_sha".into(), sha.clone());
            }
            events.push(event("merged".into(), "merged", detail));
        } else {
            events.push(event("closed".into(), "closed", Map::new()));
        }
    } else if !open(old) && open(new) {
        events.push(event("reopened".into(), "reopened", Map::new()));
    }
    events
}

/// The key an event takes when `taken` says which of its item's keys are already used: the
/// event's own key the first time, then `KEY/2`, `KEY/3`, and so on. A comment, a head or a
/// check result on one head happens once, so only a repeated draft, queue, close or reopen moves.
pub fn occurrence_key(key: &str, mut taken: impl FnMut(&str) -> anyhow::Result<bool>) -> anyhow::Result<String> {
    if !taken(key)? {
        return Ok(key.to_owned());
    }
    for n in 2.. {
        let candidate = format!("{key}/{n}");
        if !taken(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!("an unbounded search returns")
}

/// One event as the mirror and the read API show it.
pub fn view(event: &Event, item: &str, subject: &str, at: &str) -> Value {
    json!({
        "subject": subject,
        "item": item,
        "key": event.key,
        "event": event.event,
        "head_sha": event.head_sha,
        "at": at,
        "detail": event.detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn names(events: &[Event]) -> Vec<(&str, &str)> {
        events.iter().map(|event| (event.key.as_str(), event.event)).collect()
    }

    #[test]
    fn a_new_item_is_a_baseline_and_holds_no_event() {
        assert!(derive(None, &facts(json!({"state": "open", "head_sha": "a"}))).is_empty());
    }

    #[test]
    fn each_change_of_a_pull_request_is_one_keyed_event() {
        let before = facts(json!({"state": "open", "draft": true, "head_sha": "a",
            "required_checks": {"state": "pending"},
            "recent_comments": [{"kind": "comment", "id": 1, "author": "ada", "at": "2026-10-30T10:00:00Z"}]}));
        let after = facts(json!({"state": "open", "draft": false, "head_sha": "b",
            "merge_queue": {"state": "QUEUED", "position": 3},
            "required_checks": {"state": "fail", "failed": ["build"]},
            "recent_comments": [
                {"kind": "comment", "id": 1, "author": "ada", "at": "2026-10-30T10:00:00Z"},
                {"kind": "review", "id": 9, "author": "lin", "at": "2026-10-30T11:00:00Z", "state": "APPROVED"}]}));
        let events = derive(Some(&before), &after);
        assert_eq!(names(&events), [
            ("head/b", "head-changed"),
            ("became-ready", "became-ready"),
            ("merge-queue-entered", "merge-queue-entered"),
            ("checks/b/fail", "checks-failed"),
            ("review/9", "review"),
        ]);
        assert_eq!(events[0].detail["from"], "a");
        assert_eq!(events[2].detail["position"], 3);
        assert_eq!(events[3].detail["failed"], json!(["build"]));
        assert_eq!(events[4].detail["state"], "APPROVED");
        assert!(events.iter().all(|event| event.head_sha.as_deref() == Some("b")));

        let merged = facts(json!({"state": "closed", "merged": true, "head_sha": "b",
            "merge_commit_sha": "m", "required_checks": {"state": "fail"}}));
        assert_eq!(names(&derive(Some(&after), &merged)), [
            ("merge-queue-left", "merge-queue-left"), ("merged", "merged"),
        ]);
        let reopened = facts(json!({"state": "open", "head_sha": "b", "required_checks": {"state": "fail"}}));
        assert_eq!(names(&derive(Some(&merged), &reopened)), [("reopened", "reopened")]);
        assert!(derive(Some(&reopened), &reopened).is_empty(), "the same facts hold nothing");
    }

    #[test]
    fn a_repeated_event_takes_the_next_free_key() {
        let taken = ["closed", "closed/2"];
        assert_eq!(occurrence_key("closed", |key| Ok(taken.contains(&key))).unwrap(), "closed/3");
        assert_eq!(occurrence_key("reopened", |key| Ok(taken.contains(&key))).unwrap(), "reopened");
        assert!(occurrence_key("closed", |_| anyhow::bail!("unreadable")).is_err(), "an error is not a free key");
    }
}
