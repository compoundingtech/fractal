//! Native program PR waits share the repository's standing observer and its noted facts.
use super::{ProgramEngine, Request};
use crate::github_watch::{self, ThreadRef};
use crate::store::Store;
use crate::watches::WatchesStore as _;
use crate::IdleHoldsStore as _;
use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Group shared repository interests without losing a new wait on an already watched PR.
pub(crate) fn targets(registrations: Vec<Value>, repo: &str) -> Result<Vec<crate::resource::AwaitPrTarget>> {
    let mut targets = BTreeMap::<u64, BTreeSet<String>>::new();
    for registration in registrations {
        if !registration["repo"].as_str().context("PR registration has no repository")?.eq_ignore_ascii_case(repo) {
            continue;
        }
        let pr = registration["pr"].as_u64().context("PR registration has no PR number")?;
        let token = registration["registration"].as_str().context("PR registration has no acquisition token")?;
        targets.entry(pr).or_default().insert(token.to_owned());
    }
    Ok(targets.into_iter().map(|(pr, registrations)| crate::resource::AwaitPrTarget {
        pr, registrations: registrations.into_iter().collect(),
    }).collect())
}

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
    let registration = engine.register_pr(request, &subject, json!({
        "repo": repo, "pr": pr, "head": head,
    }))?;
    if registration["repo"] != repo || registration["pr"] != pr || registration["head"] != head {
        engine.fail(&request.step, request.attempt, "PR wait key reused with a different request")?;
        return Ok(true);
    }
    store.ensure_watch_observer(&thread)?;
    let token = registration["registration"].as_str().context("PR registration has no acquisition token")?;
    let observer = store.latest_observation(&observer_subject, "observer.state")?;
    if observer.as_ref().is_some_and(|claim| {
        claim.body["fields"]["state"] == "unreachable"
            && claim.body["fields"]["error_code"] == "github-repository-not-found"
            && includes_token(&claim.body["evidence"], token)
    }) {
        engine.resolve(request, &subject, failure("unknown-repo", repo, pr, "GitHub could not find the repository"))?;
        return Ok(true);
    }
    // Listings are incremental. Only an explicit target 404 acquired for this wait proves absence.
    let observed = store.latest_actual_value(&resource)?;
    if observed.as_ref().and_then(|value| value.pointer("/facts/await_prs_not_found"))
        .and_then(Value::as_array).is_some_and(|missing| missing.iter().any(|target| {
            target["pr"].as_u64() == Some(pr) && includes_token(&target["registrations"], token)
        }))
    {
        engine.resolve(request, &subject, failure("unknown-pr", repo, pr, "GitHub could not find the pull request"))?;
        return Ok(true);
    }
    let item = format!("{resource}/pull-request/{pr}");
    let actual = store.latest_actual_value(&item)?;
    let Some(facts) = actual.as_ref().and_then(|actual| actual.get("facts")) else { return Ok(false) };
    // The paired response prevents an old observer from mixing a cached head with a later merge.
    let Some(acquired) = acquired(facts, token) else { return Ok(false) };
    let since = registration["since_unix_ms"].as_str().context("PR wait has no baseline")?.parse::<u128>()?;
    let left = if github_watch::open_outside_merge_queue(facts) && acquired["head"] == head {
        store.thread_left_merge_queue_on_head(&thread, since, head)?.is_some()
    } else { false };
    let Some(exit) = outcome(acquired, head, left) else { return Ok(false) };
    engine.resolve(request, &subject, exit)?;
    Ok(true)
}

fn failure(code: &str, repo: &str, pr: u64, message: &str) -> Value {
    json!({"_tag": "Failure", "error": {"_tag": "AwaitPrFailed", "code": code, "repo": repo, "pr": pr, "message": message}})
}

fn includes_token(tokens: &Value, token: &str) -> bool {
    tokens.as_array().is_some_and(|tokens| tokens.iter().any(|value| value.as_str() == Some(token)))
}

fn acquired<'a>(facts: &'a Value, token: &str) -> Option<&'a Value> {
    let acquired = facts.get("await_pr_observation")?;
    includes_token(&acquired["registrations"], token).then_some(acquired)
}

fn outcome(facts: &Value, head: &str, left: bool) -> Option<Value> {
    let current = facts["head"].as_str()?;
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
        assert_eq!(outcome(&json!({"merged": true, "head": "head", "merge_commit_sha": "merge"}), "head", false),
            Some(json!({"_tag": "Success", "value": {"outcome": "merged", "mergeCommit": "merge"}})));
        assert_eq!(outcome(&json!({"merged": true}), "head", false), None);
    }

    #[test]
    fn an_open_pr_that_never_queued_does_not_dequeue() {
        assert_eq!(outcome(&json!({"state": "open", "head": "head"}), "head", false), None);
        assert_eq!(outcome(&json!({"state": "open", "head": "head"}), "head", true),
            Some(json!({"_tag": "Success", "value": {"outcome": "dequeued", "reason": "merge-queue-exit"}})));
    }

    #[test]
    fn moved_head_is_a_successful_dequeue() {
        assert_eq!(outcome(&json!({"state": "open", "head": "new"}), "old", false),
            Some(json!({"_tag": "Success", "value": {"outcome": "dequeued", "reason": "head-changed"}})));
    }

    #[test]
    fn targets_group_all_registration_tokens_for_a_shared_pr() {
        let grouped = targets(vec![
            json!({"repo":"acme/garden","pr":12,"registration":"claim-b"}),
            json!({"repo":"acme/garden","pr":12,"registration":"claim-a"}),
            json!({"repo":"elsewhere/garden","pr":12,"registration":"unrelated"}),
        ], "ACME/GARDEN").unwrap();
        assert_eq!(grouped, vec![crate::resource::AwaitPrTarget {
            pr: 12, registrations: vec!["claim-a".into(), "claim-b".into()],
        }]);
    }

    #[test]
    fn paired_acquisition_cannot_mix_a_retained_token_with_an_unrelated_head() {
        let facts = json!({
            "head_sha":"cached-other-head","merged":true,"merge_commit_sha":"unrelated",
            "await_pr_observation":{
                "registrations":["claim-a"],"head":"requested","merged":false,"merge_commit_sha":null,
            },
        });
        assert!(acquired(&facts, "claim-b").is_none(), "another wait's proof is not ours");
        assert_eq!(outcome(acquired(&facts, "claim-a").unwrap(), "requested", false), None);
    }
}
