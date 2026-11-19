//! Native program PR waits share the repository's standing observer and its noted facts.
use super::{ProgramEngine, Request};
use crate::github_watch::{self, ThreadRef};
use crate::store::Store;
use crate::watches::WatchesStore as _;
use crate::IdleHoldsStore as _;
use anyhow::{Context as _, Result};
use serde_json::{Value, json};

pub(crate) fn subject(request: &Request) -> String {
    format!("program-await-pr/{}", super::sha256(request.idempotency_key().as_bytes()))
}

pub(crate) fn settle(store: &Store, engine: &ProgramEngine, request: &Request, repo: &str, pr: u64, head: &str) -> Result<bool> {
    let thread = ThreadRef::parse(&format!("{repo}#{pr}"))?;
    let subject = subject(request);
    let observers = store.repository_observers()?;
    let observer_subject = thread.observer();
    let current = observers.iter().find(|(subject, _)| subject == &observer_subject).map(|(_, spec)| spec);
    let resource = store.standing_resource(&thread.locator(), &observer_subject, current, &observers)?;
    let registration = engine.register_wait(request, &subject, "await-pr", json!({
        "repo": repo, "pr": pr, "head": head,
    }))?;
    if registration["repo"] != repo || registration["pr"] != pr || registration["head"] != head {
        engine.fail(&request.step, request.attempt, "PR wait key reused with a different request")?;
        return Ok(true);
    }
    store.ensure_watch_observer(&thread)?;
    let observer = store.latest_actual_value(&observer_subject)?;
    if observer.as_ref().is_some_and(|actual| actual["state"] == "unreachable" && actual["error_code"] == "github-repository-not-found") {
        engine.resolve(request, &subject, failure("unknown-repo", repo, pr, "GitHub could not find the repository"))?;
        return Ok(true);
    }
    let item = format!("{resource}/pull-request/{pr}");
    let actual = store.latest_actual_value(&item)?;
    let Some(facts) = actual.as_ref().and_then(|actual| actual.get("facts")) else {
        // A repository listing is incremental, never evidence that an unseen PR is missing.
        let observed = store.latest_actual_value(&resource)?;
        if observed.as_ref().and_then(|value| value.pointer("/facts/await_prs_not_found"))
            .and_then(Value::as_array).is_some_and(|missing| missing.iter().any(|number| number.as_u64() == Some(pr)))
        {
            engine.resolve(request, &subject, failure("unknown-pr", repo, pr, "GitHub could not find the pull request"))?;
            return Ok(true);
        }
        return Ok(false);
    };
    let since = registration["since_unix_ms"].as_str().context("PR wait has no baseline")?.parse::<u128>()?;
    let left = if github_watch::open_outside_merge_queue(facts) && facts["head_sha"] == head {
        store.thread_left_merge_queue_on_head(&thread, since, head)?.is_some()
    } else { false };
    let Some(exit) = outcome(facts, head, left) else { return Ok(false) };
    engine.resolve(request, &subject, exit)?;
    Ok(true)
}

fn failure(code: &str, repo: &str, pr: u64, message: &str) -> Value {
    json!({"_tag": "Failure", "error": {"_tag": "AwaitPrFailed", "code": code, "repo": repo, "pr": pr, "message": message}})
}

fn outcome(facts: &Value, head: &str, left: bool) -> Option<Value> {
    let current = facts["head_sha"].as_str()?;
    let value = if current != head {
        json!({"outcome": "dequeued", "reason": "head-changed"})
    } else if facts["merged"].as_bool() == Some(true) {
        json!({"outcome": "merged", "mergeCommit": facts["merge_commit_sha"].as_str().filter(|sha| !sha.is_empty())?})
    } else if left {
        json!({"outcome": "dequeued", "reason": "merge-queue-exit"})
    } else { return None };
    Some(json!({"_tag": "Success", "value": value}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_requires_the_actual_merge_commit() {
        assert_eq!(outcome(&json!({"merged": true, "head_sha": "head", "merge_commit_sha": "merge"}), "head", false),
            Some(json!({"_tag": "Success", "value": {"outcome": "merged", "mergeCommit": "merge"}})));
        assert_eq!(outcome(&json!({"merged": true}), "head", false), None);
    }

    #[test]
    fn an_open_pr_that_never_queued_does_not_dequeue() {
        assert_eq!(outcome(&json!({"state": "open", "head_sha": "head"}), "head", false), None);
        assert_eq!(outcome(&json!({"state": "open", "head_sha": "head"}), "head", true),
            Some(json!({"_tag": "Success", "value": {"outcome": "dequeued", "reason": "merge-queue-exit"}})));
    }

    #[test]
    fn moved_head_is_a_successful_dequeue() {
        assert_eq!(outcome(&json!({"state": "open", "head_sha": "new"}), "old", false),
            Some(json!({"_tag": "Success", "value": {"outcome": "dequeued", "reason": "head-changed"}})));
    }
}
