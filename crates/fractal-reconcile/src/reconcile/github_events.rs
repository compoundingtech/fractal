//! The keyed events one change of a GitHub item holds, derived by comparing the item's facts in
//! the plugin's mirror with the facts a new look recorded. The poller keeps them in its mirror,
//! and from step 3b of the plugin store also writes each as one `github.event` claim. An item the
//! mirror never held is a baseline and holds no event, so a member that starts polling, or whose
//! mirror is new, wakes nobody.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The operation names this observation, not the global event subject. Keep every payload
/// field in the fingerprint: even the same observer on two pollers can see different facts.
pub(super) fn observation_key(subject: &str, fields: &BTreeMap<String, Value>) -> anyhow::Result<String> {
    let digest = fingerprint("github-event-observation.v1", subject, fields)?;
    Ok(format!("github-event-observation:v1:{digest}"))
}

/// The operation names one poller's write of one completed CI run, like [`observation_key`].
pub(super) fn workflow_run_observation_key(subject: &str, fields: &BTreeMap<String, Value>) -> anyhow::Result<String> {
    let digest = fingerprint("github-workflow-run-observation.v1", subject, fields)?;
    Ok(format!("github-workflow-run-observation:v1:{digest}"))
}

fn fingerprint(domain: &str, subject: &str, fields: &BTreeMap<String, Value>) -> anyhow::Result<String> {
    let fields = fields.iter()
        .map(|(name, value)| (name, smallclaims::hash::canonical_json_value(value)))
        .collect::<BTreeMap<_, _>>();
    smallclaims::hash::canonical_hash(&(domain, subject, fields))
}

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

    fn event_claim(observer: &str, at: &str) -> crate::model::ClaimInput {
        let subject = "github-event/github/acme/garden/issue/7/closed";
        let fields = BTreeMap::from([
            ("version".into(), json!(1)),
            ("item".into(), json!("resource/github/acme/garden/issue/7")),
            ("event".into(), json!("closed")),
            ("observer".into(), json!(observer)),
            ("at".into(), json!(at)),
            ("detail".into(), json!({"state": "closed"})),
        ]);
        let key = observation_key(subject, &fields).unwrap();
        crate::model::ClaimInput {
            subject: subject.into(), kind: fractal_schema::github_plugin::EVENT.into(),
            actor: None, fields, evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(key),
        }
    }

    #[test]
    fn observation_keys_cover_every_field_and_canonicalize_nested_objects() {
        let input = event_claim("observer/a", "2026-11-11T10:00:00Z");
        let original = observation_key(&input.subject, &input.fields).unwrap();
        for (name, value) in [
            ("observer", json!("observer/b")),
            ("at", json!("2026-11-11T10:00:01Z")),
            ("head_sha", json!("abc")),
            ("detail", json!({"state": "open"})),
            ("event", json!("reopened")),
            ("version", json!(2)),
            ("item", json!("resource/github/acme/garden/issue/8")),
        ] {
            let mut fields = input.fields.clone();
            fields.insert(name.into(), value);
            assert_ne!(observation_key(&input.subject, &fields).unwrap(), original, "{name}");
        }
        assert_ne!(observation_key("github-event/github/acme/garden/issue/8/closed", &input.fields).unwrap(), original);
        let mut fields = input.fields;
        fields.insert("detail".into(), serde_json::from_str(r#"{"b":{"y":2,"x":1},"a":0}"#).unwrap());
        let first = observation_key(&input.subject, &fields).unwrap();
        fields.insert("detail".into(), serde_json::from_str(r#"{"a":0,"b":{"x":1,"y":2}}"#).unwrap());
        assert_eq!(observation_key(&input.subject, &fields).unwrap(), first);
    }

    /// Independent writers race before either sees the other's event. Old global keys and
    /// new observation keys must fold by subject, regardless of replication arrival order.
    #[test]
    fn concurrent_event_observations_and_legacy_claims_fold_once_without_conflicts() {
        use crate::store::Store;
        const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
        for (other_observer, legacy) in [("observer/b", false), ("observer/a", false), ("observer/b", true)] {
            let a = Store::open_memory("alder").unwrap();
            let b = Store::open_memory("birch").unwrap();
            let forward = Store::open_memory("cedar").unwrap();
            let reverse = Store::open_memory("dogwood").unwrap();
            for store in [&a, &b, &forward, &reverse] {
                store.bind_fleet(FLEET).unwrap();
            }
            let input = event_claim("observer/a", "2026-11-11T10:00:00Z");
            let first = a.append_claim(&input).unwrap();
            assert_eq!(a.append_claim(&input).unwrap().id, first.id, "exact retry");
            let mut other = event_claim(other_observer, "2026-11-11T10:00:01Z");
            if legacy {
                other.idempotency_key = Some(format!("github-event:{}", other.subject));
            }
            let second = b.append_claim(&other).unwrap();
            assert_ne!(first.body["_operation"]["id"], second.body["_operation"]["id"]);
            for (target, sources) in [(&forward, [&a, &b]), (&reverse, [&b, &a])] {
                for source in sources {
                    let exchange = source.export_replication_exchange_answering(
                        FLEET, &target.replication_inventory().unwrap(),
                        &target.replication_signature_requests().unwrap(),
                    ).unwrap();
                    target.receive_replication_exchange(&source.origin, FLEET, &exchange).unwrap();
                    target.validate_replication_backlog().unwrap();
                    target.project_replication_backlog().unwrap();
                }
                assert_eq!(target.idempotency_conflicts(10).unwrap().0, 0);
                assert_eq!(target.claims_for(&input.subject, None).unwrap().len(), 2, "provenance survives");
                let events = target.github_events("resource/github/acme/garden", Some(7), 10).unwrap();
                assert_eq!(events.len(), 1, "one event, not one row per observation");
                assert_eq!(events, target.github_events("resource/github/acme/garden", None, 1).unwrap());
                target.replay_replication_graph().unwrap();
                assert_eq!(events, target.github_events("resource/github/acme/garden", Some(7), 10).unwrap());
            }
            assert_eq!(
                forward.github_events("resource/github/acme/garden", None, 10).unwrap(),
                reverse.github_events("resource/github/acme/garden", None, 10).unwrap(),
                "canonical fold is independent of arrival",
            );
        }
    }

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
