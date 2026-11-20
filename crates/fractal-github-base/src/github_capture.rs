//! Restartable acquisition only: callers publish facts, checkpoints and receipts together.
//!
//! A restored acquisition starts the mutable UPDATED_AT listing at its head. Only a
//! claim-backed receipt may skip a PR; neither an offset nor a local cache proves capture.
//! Full enumeration runs at least daily; incremental passes overlap the previous start
//! by two hours. Every selected PR's ready/draft connection is read from creation, with
//! no date filter or event cap. Each call sends at most one admitted GraphQL request.
use std::collections::VecDeque;
use std::pin::Pin;

use anyhow::{Context as _, Result, bail, ensure};
use fractal_schema::github_capture::{
    Coverage, FactFamily, FactImage, LifecycleFact, LifecycleKind, MergeFact,
    PullRequestFact, PullRequestState, RepositoryFact,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::github_http::GithubAuth;
use crate::resource::{LocalObserverBudget, ProviderForbidden, ProviderRateLimit, ProviderUnauthenticated};

/// Store-owned publication boundary, driven by the existing repository observer.
/// Implementations select authored sources and commit bounded acquisition pages;
/// historical images never enter operational observation facts or wake watches.
pub trait GitHubCaptureLedger: std::fmt::Debug + Send + Sync {
    fn step<'a>(
        &'a self,
        api_base: &'a str,
        repository: &'a str,
        auth: &'a GithubAuth,
    ) -> Pin<Box<dyn Future<Output = Result<CaptureProgress>> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureProgress {
    /// Whether this locator is selected by an authored capture source.
    pub active: bool,
    /// Continue on the same observer deadline; no separate acquisition timer.
    pub continue_after_ms: Option<u64>,
}

const DAY_MS: u64 = 86_400_000;
const OVERLAP_MS: u64 = 7_200_000;
const REPOSITORY_FIELDS: &str = "id nameWithOwner isPrivate";
const PULL_FIELDS: &str = "id number author {login} state isDraft createdAt updatedAt closedAt mergedAt repository {id}";
const TIMELINE: &str = r#"timelineItems(first:100,after:$after,itemTypes:[READY_FOR_REVIEW_EVENT,CONVERT_TO_DRAFT_EVENT]) {
 nodes {__typename ... on ReadyForReviewEvent {id createdAt} ... on ConvertToDraftEvent {id createdAt}}
 pageInfo {hasNextPage endCursor}}
"#;

/// A locator bootstraps identity; once resolved, node ID survives rename/transfer.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RepositoryTarget {
    pub locator: String,
    pub node_id: Option<String>,
}

/// Bounded source progress. No repository-wide identity map or event array is stored.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AcquisitionCheckpoint {
    pub repository: Option<RepositoryFact>,
    pub checked_ms: Option<u64>,
    pub full_ms: Option<u64>,
    pub pass: Option<AcquisitionPass>,
    pub active: Option<TimelineCheckpoint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct AcquisitionPass {
    pub started_ms: u64,
    pub full: bool,
    pub overlap_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TimelineCheckpoint {
    pub pull_request_id: String,
    pub upstream_updated_at_ms: u64,
    pub after: Option<String>,
    pub first_transition: Option<LifecycleKind>,
}

/// Publish atomically with the final PR image and checkpoint. Lookup must prove that
/// an image at least this new was committed in this pass, not merely requested.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureReceipt {
    pub pull_request_id: String,
    pub upstream_updated_at_ms: u64,
    pub pass_started_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquisitionStatus {
    Continue,
    Complete,
    BudgetStopped,
    Unavailable,
    Failed,
}

/// At most 100 lifecycle images plus one PR and one merge image per page.
/// Errors retain the shared transport's typed refusal/budget causes.
#[derive(Debug)]
pub struct AcquisitionPage {
    pub facts: Vec<FactImage>,
    pub checkpoint: AcquisitionCheckpoint,
    pub receipt: Option<CaptureReceipt>,
    pub coverage: Vec<Coverage>,
    pub status: AcquisitionStatus,
    pub error: Option<anyhow::Error>,
}

pub struct CaptureAcquisition {
    target: RepositoryTarget,
    checkpoint: AcquisitionCheckpoint,
    pending: VecDeque<(String, u64)>,
    listing_after: Option<String>,
    listing_finished: bool,
    repository_checked: bool,
    complete: bool,
}

impl CaptureAcquisition {
    /// Restoring discards mutable list positions, but resumes the bounded active
    /// timeline. Pass identity remains fixed until the whole traversal succeeds.
    pub fn resume(target: RepositoryTarget, mut checkpoint: AcquisitionCheckpoint, now_ms: u64) -> Self {
        if checkpoint.pass.is_none() {
            checkpoint.pass = Some(AcquisitionPass {
                started_ms: now_ms,
                full: checkpoint.full_ms.is_none_or(|last| now_ms.saturating_sub(last) >= DAY_MS),
                overlap_ms: checkpoint.checked_ms.map(|last| last.saturating_sub(OVERLAP_MS)),
            });
        }
        Self { target, checkpoint, pending: VecDeque::new(), listing_after: None,
            listing_finished: false, repository_checked: false, complete: false }
    }

    /// One bounded batch through the shared authenticated GraphQL send/admission path.
    /// `captured_since(id, upstream_updated_at_ms, pass_started_ms)` consults committed
    /// claim projection receipts. Persist the returned checkpoint only with its facts.
    /// On BudgetStopped, preserve progress and let the shared retry deadline schedule
    /// continuation; no request is retried here. Reconstruct after any other failure.
    pub async fn next_page<F>(&mut self, api_base: &str, auth: &GithubAuth, mut captured_since: F) -> AcquisitionPage
    where F: FnMut(&str, u64, u64) -> bool {
        let result = self.acquire(api_base, auth, &mut captured_since).await;
        match result {
            Ok((facts, receipt)) => self.page(facts, receipt, if self.complete {
                AcquisitionStatus::Complete
            } else { AcquisitionStatus::Continue }, None),
            Err(error) => {
                let status = if error.downcast_ref::<LocalObserverBudget>().is_some()
                    || error.downcast_ref::<ProviderRateLimit>().is_some() {
                    AcquisitionStatus::BudgetStopped
                } else if error.downcast_ref::<Unavailable>().is_some()
                    || error.downcast_ref::<ProviderForbidden>().is_some()
                    || error.downcast_ref::<ProviderUnauthenticated>().is_some() {
                    AcquisitionStatus::Unavailable
                } else { AcquisitionStatus::Failed };
                self.page(Vec::new(), None, status, Some(error))
            }
        }
    }

    fn page(&self, facts: Vec<FactImage>, receipt: Option<CaptureReceipt>, status: AcquisitionStatus,
        error: Option<anyhow::Error>) -> AcquisitionPage {
        let known = self.repository_checked && error.is_none();
        let coverage = self.checkpoint.repository.as_ref().map_or_else(Vec::new, |repo| {
            [FactFamily::Repository, FactFamily::PullRequest, FactFamily::Lifecycle, FactFamily::Merge]
                .into_iter().map(|family| Coverage {
                    repository_id: repo.id.clone(), family,
                    complete: known && self.complete && self.checkpoint.full_ms.is_some(),
                    authoritative: known,
                    history_from_ms: 0,
                    through_ms: if self.complete { self.checkpoint.checked_ms.unwrap_or(0) } else { 0 },
                    error: error.as_ref().map(ToString::to_string),
                }).collect()
        });
        AcquisitionPage { facts, checkpoint: self.checkpoint.clone(), receipt, coverage, status, error }
    }

    async fn graph(&self, api_base: &str, auth: &GithubAuth, query: &str, variables: Value) -> Result<Value> {
        let client = crate::resource::github_api_client();
        let (data, _) = crate::resource::github_graphql_with_cache_eligibility(
            &client, api_base, auth, query, variables).await?;
        Ok(data)
    }

    async fn acquire<F>(&mut self, api_base: &str, auth: &GithubAuth, captured_since: &mut F)
        -> Result<(Vec<FactImage>, Option<CaptureReceipt>)>
    where F: FnMut(&str, u64, u64) -> bool {
        if self.complete { return Ok((Vec::new(), None)); }
        if !self.repository_checked {
            let id = self.checkpoint.repository.as_ref().map(|repo| &repo.id).or(self.target.node_id.as_ref());
            let (query, variables, key) = if let Some(id) = id {
                (format!("query($id:ID!) {{node(id:$id) {{... on Repository {{{REPOSITORY_FIELDS}}}}}}}"), json!({"id":id}), "node")
            } else {
                let (owner, name) = self.target.locator.split_once('/').context("invalid repository locator")?;
                (format!("query($owner:String!,$name:String!) {{repository(owner:$owner,name:$name) {{{REPOSITORY_FIELDS}}}}}"),
                    json!({"owner":owner,"name":name}), "repository")
            };
            let data = self.graph(api_base, auth, &query, variables).await?;
            let node = &data[key];
            if node.is_null() { return Err(Unavailable.into()); }
            let repo = RepositoryFact { id: text(node, "id")?.to_owned(),
                locator: text(node, "nameWithOwner")?.to_owned(),
                is_private: node["isPrivate"].as_bool().context("repository omitted privacy")? };
            self.checkpoint.repository = Some(repo.clone());
            self.repository_checked = true;
            return Ok((vec![FactImage::Repository(repo)], None));
        }
        if self.checkpoint.active.is_some() { return self.timeline(api_base, auth).await; }
        let pass = self.checkpoint.pass.context("acquisition has no active pass")?;
        while let Some((id, updated)) = self.pending.pop_front() {
            if !captured_since(&id, updated, pass.started_ms) {
                self.checkpoint.active = Some(TimelineCheckpoint { pull_request_id: id,
                    upstream_updated_at_ms: updated, after: None, first_transition: None });
                return self.timeline(api_base, auth).await;
            }
        }
        if self.listing_finished {
            self.checkpoint.checked_ms = Some(pass.started_ms);
            if pass.full { self.checkpoint.full_ms = Some(pass.started_ms); }
            self.checkpoint.pass = None;
            self.complete = true;
            return Ok((Vec::new(), None));
        }
        let repo = self.checkpoint.repository.as_ref().context("repository unresolved")?;
        let query = "query($id:ID!,$after:String) {node(id:$id) {... on Repository {pullRequests(first:100,after:$after,states:[OPEN,CLOSED,MERGED],orderBy:{field:UPDATED_AT,direction:DESC}) {nodes {id updatedAt} pageInfo {hasNextPage endCursor}}}}}";
        let data = self.graph(api_base, auth, query, json!({"id":repo.id,"after":self.listing_after})).await?;
        if data["node"].is_null() { return Err(Unavailable.into()); }
        let connection = &data["node"]["pullRequests"];
        let next = next_cursor(connection, self.listing_after.as_deref())?;
        let mut candidates = VecDeque::new();
        let mut before_overlap = false;
        for node in connection["nodes"].as_array().context("PR listing inaccessible")? {
            let updated = millis(node, "updatedAt")?;
            if !pass.full && pass.overlap_ms.is_some_and(|bound| updated < bound) {
                before_overlap = true;
                break;
            }
            candidates.push_back((text(node, "id")?.to_owned(), updated));
        }
        self.pending = candidates;
        self.listing_finished = before_overlap || next.is_none();
        self.listing_after = next;
        Ok((Vec::new(), None))
    }

    async fn timeline(&mut self, api_base: &str, auth: &GithubAuth)
        -> Result<(Vec<FactImage>, Option<CaptureReceipt>)> {
        let active = self.checkpoint.active.as_ref().context("no active PR")?;
        let query = format!("query($id:ID!,$after:String) {{node(id:$id) {{... on PullRequest {{{PULL_FIELDS} {TIMELINE}}}}}}}");
        let data = self.graph(api_base, auth, &query,
            json!({"id":active.pull_request_id,"after":active.after})).await?;
        self.apply_timeline(&data)
    }

    fn apply_timeline(&mut self, data: &Value) -> Result<(Vec<FactImage>, Option<CaptureReceipt>)> {
        let node = &data["node"];
        if node.is_null() { return Err(Unavailable.into()); }
        let mut active = self.checkpoint.active.clone().context("no active PR")?;
        ensure!(text(node, "id")? == active.pull_request_id, "PR node identity changed");
        let updated = millis(node, "updatedAt")?;
        // A timeline changed during pagination: restart before certifying its initial
        // state. Already emitted immutable event identities are harmless duplicates.
        if updated != active.upstream_updated_at_ms {
            active.upstream_updated_at_ms = updated;
            active.after = None;
            active.first_transition = None;
            self.checkpoint.active = Some(active);
            return Ok((Vec::new(), None));
        }
        let repo = self.checkpoint.repository.as_ref().context("repository unresolved")?;
        ensure!(text(&node["repository"], "id")? == repo.id, "PR moved outside the acquired repository");
        let connection = &node["timelineItems"];
        let next = next_cursor(connection, active.after.as_deref())?;
        let mut facts = Vec::new();
        for event in connection["nodes"].as_array().context("timeline inaccessible")? {
            let kind = match text(event, "__typename")? {
                "ReadyForReviewEvent" => LifecycleKind::Ready,
                "ConvertToDraftEvent" => LifecycleKind::Draft,
                other => bail!("unexpected lifecycle event {other}"),
            };
            active.first_transition.get_or_insert(kind);
            facts.push(FactImage::Lifecycle(LifecycleFact { id: text(event, "id")?.to_owned(),
                repository_id: repo.id.clone(), pull_request_id: active.pull_request_id.clone(),
                kind, occurred_at_ms: millis(event, "createdAt")? }));
        }
        if next.is_some() {
            active.after = next;
            self.checkpoint.active = Some(active);
            return Ok((facts, None));
        }
        let is_draft = node["isDraft"].as_bool().context("PR omitted draft status")?;
        let state = match text(node, "state")? {
            "OPEN" => PullRequestState::Open,
            "CLOSED" => PullRequestState::Closed,
            "MERGED" => PullRequestState::Merged,
            other => bail!("unexpected PR state {other}"),
        };
        let merged_at_ms = optional_millis(node, "mergedAt")?;
        facts.push(FactImage::PullRequest(PullRequestFact {
            id: active.pull_request_id.clone(), repository_id: repo.id.clone(),
            number: node["number"].as_u64().context("PR omitted number")?,
            author_login: node["author"]["login"].as_str().map(str::to_owned), state, is_draft,
            created_as_draft: Some(active.first_transition.map_or(is_draft, |kind| kind == LifecycleKind::Ready)),
            created_at_ms: millis(node, "createdAt")?, closed_at_ms: optional_millis(node, "closedAt")?, merged_at_ms,
        }));
        if let Some(merged_at_ms) = merged_at_ms {
            facts.push(FactImage::Merge(MergeFact { pull_request_id: active.pull_request_id.clone(),
                repository_id: repo.id.clone(), merged_at_ms }));
        }
        let receipt = CaptureReceipt { pull_request_id: active.pull_request_id,
            upstream_updated_at_ms: updated,
            pass_started_ms: self.checkpoint.pass.context("no active pass")?.started_ms };
        self.checkpoint.active = None;
        Ok((facts, Some(receipt)))
    }
}

#[derive(Debug)]
struct Unavailable;
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("GitHub scope is unavailable; history remains unknown") }
}
impl std::error::Error for Unavailable {}

fn text<'a>(node: &'a Value, key: &str) -> Result<&'a str> {
    node[key].as_str().with_context(|| format!("GitHub omitted {key}"))
}
fn millis(node: &Value, key: &str) -> Result<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(text(node, key)?)?.timestamp_millis();
    u64::try_from(at).context("GitHub timestamp precedes epoch")
}
fn optional_millis(node: &Value, key: &str) -> Result<Option<u64>> {
    ensure!(node.get(key).is_some(), "GitHub omitted {key}");
    if node[key].is_null() { Ok(None) } else { millis(node, key).map(Some) }
}
fn next_cursor(connection: &Value, previous: Option<&str>) -> Result<Option<String>> {
    if !connection["pageInfo"]["hasNextPage"].as_bool().context("GitHub omitted pagination status")? {
        return Ok(None);
    }
    let next = text(&connection["pageInfo"], "endCursor")?;
    ensure!(!next.is_empty() && Some(next) != previous, "GitHub pagination did not advance");
    Ok(Some(next.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::test_github::FakeGithub;

    const AT: &str = "2026-01-01T00:00:00Z";

    fn target() -> RepositoryTarget {
        RepositoryTarget { locator: "example/project".into(), node_id: Some("repo".into()) }
    }

    fn repository() -> RepositoryFact {
        RepositoryFact { id: "repo".into(), locator: "example/project".into(), is_private: true }
    }

    fn acquisition() -> CaptureAcquisition {
        CaptureAcquisition::resume(target(), AcquisitionCheckpoint {
            repository: Some(repository()), ..AcquisitionCheckpoint::default()
        }, DAY_MS * 100)
    }

    fn pull(id: &str, events: Vec<Value>, next: Option<&str>, draft: bool) -> Value {
        json!({"id":id,"number":1,"author":{"login":"author"},"state":"OPEN",
            "isDraft":draft,"createdAt":AT,"updatedAt":AT,"closedAt":null,"mergedAt":null,
            "repository":{"id":"repo"},"timelineItems":{"nodes":events,
                "pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}}})
    }

    fn event(index: usize, ready: bool) -> Value {
        json!({"id":format!("event-{index}"),"createdAt":AT,
            "__typename":if ready {"ReadyForReviewEvent"} else {"ConvertToDraftEvent"}})
    }

    fn activate(acquisition: &mut CaptureAcquisition, id: &str) {
        acquisition.checkpoint.active = Some(TimelineCheckpoint {
            pull_request_id: id.into(), upstream_updated_at_ms: millis(&json!({"at":AT}), "at").unwrap(),
            after: None, first_transition: None,
        });
    }

    fn serve(github: &FakeGithub, node: Value) {
        *github.graphql.lock() = json!({"data":{"node":node}}).to_string();
    }

    fn listing(ids: &[&str], next: Option<&str>) -> Value {
        json!({"pullRequests":{"nodes":ids.iter().map(|id| json!({"id":id,"updatedAt":AT})).collect::<Vec<_>>(),
            "pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}}})
    }

    #[test]
    fn timeline_over_one_hundred_is_bounded_and_resumes_with_creation_evidence() {
        let mut capture = acquisition();
        activate(&mut capture, "pull");
        let first = pull("pull", (0..100).map(|index| event(index, index % 2 == 0)).collect(), Some("events-100"), true);
        let (facts, receipt) = capture.apply_timeline(&json!({"node":first})).unwrap();
        assert_eq!(facts.len(), 100);
        assert!(receipt.is_none());
        assert!(facts.iter().all(|fact| matches!(fact, FactImage::Lifecycle(_))));
        let encoded = serde_json::to_vec(&capture.checkpoint).unwrap();
        assert!(encoded.len() < 512, "checkpoint does not retain the event array");
        let restored = serde_json::from_slice(&encoded).unwrap();
        let mut resumed = CaptureAcquisition::resume(target(), restored, DAY_MS * 101);
        assert_eq!(resumed.checkpoint.active.as_ref().unwrap().after.as_deref(), Some("events-100"));
        let last = pull("pull", vec![event(100, true)], None, false);
        let (facts, receipt) = resumed.apply_timeline(&json!({"node":last})).unwrap();
        assert!(receipt.is_some());
        let pr = facts.iter().find_map(|fact| match fact {
            FactImage::PullRequest(pr) => Some(pr), _ => None,
        }).unwrap();
        assert_eq!(pr.created_as_draft, Some(true));
        assert_eq!(pr.created_at_ms, millis(&json!({"at":AT}), "at").unwrap());
    }

    #[test]
    fn created_ready_requires_complete_timeline_including_empty_history() {
        for (events, draft, expected) in [
            (Vec::new(), false, false),
            (Vec::new(), true, true),
            (vec![event(1, false)], true, false),
            (vec![event(1, true)], false, true),
        ] {
            let mut capture = acquisition();
            activate(&mut capture, "pull");
            let (facts, _) = capture.apply_timeline(&json!({"node":pull("pull", events, None, draft)})).unwrap();
            assert!(facts.iter().any(|fact| matches!(fact,
                FactImage::PullRequest(pr) if pr.created_as_draft == Some(expected))));
        }
    }

    #[test]
    fn changed_timeline_restarts_instead_of_certifying_truncated_history() {
        let mut capture = acquisition();
        activate(&mut capture, "pull");
        capture.apply_timeline(&json!({"node":pull("pull", vec![event(0, true)], Some("next"), true)})).unwrap();
        let mut changed = pull("pull", vec![event(1, false)], None, true);
        changed["updatedAt"] = json!("2026-01-02T00:00:00Z");
        let (facts, receipt) = capture.apply_timeline(&json!({"node":changed})).unwrap();
        assert!(facts.is_empty());
        assert!(receipt.is_none());
        let active = capture.checkpoint.active.unwrap();
        assert!(active.after.is_none());
        assert!(active.first_transition.is_none());
    }

    #[test]
    fn rejected_timeline_page_keeps_committed_cursor_for_retry() {
        let mut capture = acquisition();
        activate(&mut capture, "pull");
        capture.apply_timeline(&json!({"node":pull("pull", vec![event(0, true)], Some("committed-next"), true)})).unwrap();
        let committed = serde_json::to_value(&capture.checkpoint).unwrap();
        let mut invalid = event(2, false);
        invalid["createdAt"] = json!("invalid date");
        // A later malformed event must not retain the preceding event's partial work.
        assert!(capture.apply_timeline(&json!({"node":pull("pull",
            vec![event(1, false), invalid], None, false)})).is_err());
        assert_eq!(serde_json::to_value(&capture.checkpoint).unwrap(), committed);
        let mut restored = CaptureAcquisition::resume(target(),
            serde_json::from_value(committed).unwrap(), DAY_MS * 101);
        let (facts, receipt) = restored.apply_timeline(&json!({"node":pull("pull",
            vec![event(1, false), event(2, true)], None, false)})).unwrap();
        assert_eq!(facts.iter().filter(|fact| matches!(fact, FactImage::Lifecycle(_))).count(), 2);
        assert!(facts.iter().any(|fact| matches!(fact,
            FactImage::PullRequest(pr) if pr.created_as_draft == Some(true))));
        assert_eq!(receipt.unwrap().pass_started_ms, DAY_MS * 100);
        assert!(restored.checkpoint.active.is_none());
    }

    #[tokio::test]
    async fn unavailable_repository_never_becomes_complete_or_absent() {
        let (github, base) = FakeGithub::start().await;
        serve(&github, Value::Null);
        let mut capture = acquisition();
        let page = capture.next_page(&base, &GithubAuth::test("test"), |_, _, _| false).await;
        assert_eq!(page.status, AcquisitionStatus::Unavailable);
        assert!(page.facts.is_empty());
        assert!(page.coverage.iter().all(|coverage| !coverage.complete && !coverage.authoritative));
        assert!(page.checkpoint.pass.is_some());
    }

    #[tokio::test]
    async fn budget_stop_keeps_timeline_progress_and_shared_refusal() {
        let (github, base) = FakeGithub::start().await;
        let auth = GithubAuth::test("test");
        let mut capture = acquisition();
        serve(&github, json!({"id":"repo","nameWithOwner":"renamed/project","isPrivate":true}));
        let page = capture.next_page(&base, &auth, |_, _, _| false).await;
        assert_eq!(page.checkpoint.repository.unwrap().locator, "renamed/project");
        activate(&mut capture, "pull");
        serve(&github, pull("pull", (0..100).map(|i| event(i, i % 2 == 0)).collect(), Some("next"), true));
        let page = capture.next_page(&base, &auth, |_, _, _| false).await;
        assert_eq!(page.facts.len(), 100);
        *github.graphql.lock() = json!({"errors":[{"type":"RATE_LIMITED","message":"budget exhausted"}]}).to_string();
        let page = capture.next_page(&base, &auth, |_, _, _| false).await;
        assert_eq!(page.status, AcquisitionStatus::BudgetStopped);
        assert!(page.error.unwrap().downcast_ref::<ProviderRateLimit>().is_some());
        assert_eq!(page.checkpoint.active.unwrap().after.as_deref(), Some("next"));
        assert!(page.coverage.iter().all(|coverage| !coverage.complete));
        assert_eq!(github.take_requests().len(), 3, "one request per call, no retry");
    }

    #[tokio::test]
    async fn restored_moving_listing_restarts_head_and_skips_only_committed_receipts() {
        let (github, base) = FakeGithub::start().await;
        let auth = GithubAuth::test("test");
        let mut capture = acquisition();
        serve(&github, json!({"id":"repo","nameWithOwner":"example/project","isPrivate":true}));
        capture.next_page(&base, &auth, |_, _, _| false).await;
        serve(&github, listing(&["a"], Some("old-offset")));
        capture.next_page(&base, &auth, |_, _, _| false).await;
        serve(&github, pull("a", Vec::new(), None, false));
        let page = capture.next_page(&base, &auth, |_, _, _| false).await;
        let receipt = page.receipt.unwrap();
        let mut restored = CaptureAcquisition::resume(target(), page.checkpoint, DAY_MS * 101);
        serve(&github, json!({"id":"repo","nameWithOwner":"example/project","isPrivate":true}));
        restored.next_page(&base, &auth, |_, _, _| false).await;
        serve(&github, listing(&["new", "a", "b"], None));
        restored.next_page(&base, &auth, |_, _, _| false).await;
        let requests = github.graphql_requests.lock().clone();
        assert!(requests.last().unwrap()["variables"]["after"].is_null());
        serve(&github, pull("new", Vec::new(), None, false));
        let page = restored.next_page(&base, &auth, |id, updated, started| {
            id == receipt.pull_request_id && updated <= receipt.upstream_updated_at_ms && started == receipt.pass_started_ms
        }).await;
        assert_eq!(page.receipt.unwrap().pull_request_id, "new");
        serve(&github, pull("b", Vec::new(), None, false));
        let page = restored.next_page(&base, &auth, |id, updated, started| {
            id == receipt.pull_request_id && updated <= receipt.upstream_updated_at_ms && started == receipt.pass_started_ms
        }).await;
        assert_eq!(page.receipt.unwrap().pull_request_id, "b", "a is skipped only by committed evidence");
        let page = restored.next_page(&base, &auth, |_, _, _| false).await;
        assert_eq!(page.status, AcquisitionStatus::Complete);
        assert!(page.coverage.iter().all(|coverage| coverage.complete));
    }

    #[test]
    fn full_reconciliation_is_daily_and_incremental_overlap_never_invents_history() {
        let checkpoint = AcquisitionCheckpoint { repository: Some(repository()), checked_ms: Some(DAY_MS * 2),
            full_ms: Some(DAY_MS * 2), ..AcquisitionCheckpoint::default() };
        let capture = CaptureAcquisition::resume(target(), checkpoint.clone(), DAY_MS * 2 + 1);
        let pass = capture.checkpoint.pass.unwrap();
        assert!(!pass.full);
        assert_eq!(pass.overlap_ms, Some(DAY_MS * 2 - OVERLAP_MS));
        assert!(CaptureAcquisition::resume(target(), checkpoint, DAY_MS * 3).checkpoint.pass.unwrap().full);
        assert!(next_cursor(&json!({"pageInfo":{"hasNextPage":true,"endCursor":"same"}}), Some("same")).is_err());
    }

    /// In-memory publication boundary for the observer integration contract.
    #[derive(Debug)]
    struct RecordingCapture {
        calls: std::sync::atomic::AtomicUsize,
        fail: bool,
        progress: CaptureProgress,
    }

    impl GitHubCaptureLedger for RecordingCapture {
        fn step<'a>(&'a self, api_base: &'a str, repository: &'a str, auth: &'a GithubAuth)
            -> Pin<Box<dyn Future<Output = Result<CaptureProgress>> + Send + 'a>> {
            Box::pin(async move {
                assert!(api_base.starts_with("http://127.0.0.1:"));
                assert_eq!(repository, "example/project", "source selection keeps the authored locator across redirects");
                assert!(auth.is_valid());
                self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if self.fail {
                    return Err(anyhow::anyhow!("the selected source writer has been fenced and cannot resume")
                        .context("capture publication rejected"));
                }
                Ok(self.progress)
            })
        }
    }

    fn observer_request(ledger: Option<std::sync::Arc<dyn GitHubCaptureLedger>>) -> crate::resource::ObservationRequest {
        crate::resource::ObservationRequest {
            provider: "github.repository".into(), locator: "example/project".into(),
            fields: std::collections::BTreeSet::new(), cursor: None, previous_facts: None,
            every_ms: Some(0), refresh: true, webhook_covered: false, workflow_runs: None,
            github_capture: ledger,
        }
    }

    #[tokio::test]
    async fn capture_uses_observer_deadline_without_changing_operational_facts() {
        let (github, base) = FakeGithub::start().await;
        github.route("/repos/example/project", json!({"id":7,"full_name":"renamed/project"}));
        let baseline = crate::resource::test_github::observe(observer_request(None), &base).await.unwrap();
        let ledger = std::sync::Arc::new(RecordingCapture {
            calls: std::sync::atomic::AtomicUsize::new(0), fail: false,
            progress: CaptureProgress { active: true, continue_after_ms: Some(100) },
        });
        let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let observed = crate::resource::test_github::observe(observer_request(Some(ledger.clone())), &base).await.unwrap();
        let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        assert_eq!(ledger.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(observed.facts, baseline.facts, "history never enters watch-facing facts");
        assert!(observed.next_check_unix_ms >= before + 100);
        assert!(observed.next_check_unix_ms <= after + 100);
        assert!(github.take_requests().iter().all(|request| !request.contains("/actions/")),
            "capture is independent of workflow performance fields");
    }

    type CaptureWarningFields = std::collections::BTreeMap<String, String>;

    #[derive(Clone, Default)]
    struct CaptureWarnings(std::sync::Arc<std::sync::Mutex<Vec<CaptureWarningFields>>>);

    impl tracing::field::Visit for CaptureWarnings {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.lock().unwrap().last_mut().unwrap()
                .insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    impl tracing::Subscriber for CaptureWarnings {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            self.0.lock().unwrap().push(std::collections::BTreeMap::new());
            event.record(&mut self.clone());
        }
    }

    #[tokio::test]
    async fn fenced_capture_reports_failure_without_discarding_changed_live_pull_requests() {
        use tracing::instrument::WithSubscriber as _;

        let (github, base) = FakeGithub::start().await;
        github.route("/repos/example/project", json!({"id":7,"full_name":"renamed/project"}));
        github.route("/repos/example/project/issues?state=open&per_page=100", json!([]));
        let live_pull = |head: &str| json!({
            "number": 7, "title": "Live pull request",
            "url": "https://github.com/example/project/pull/7",
            "createdAt": "2026-09-01T00:00:00Z", "isDraft": false,
            "headRefOid": head, "headRefName": "feature",
            "commits": {"nodes": []}
        });
        github.open_pull_requests(json!([live_pull("old-head")]));
        let mut initial = observer_request(None);
        initial.fields.insert("pull_requests".into());
        let baseline = crate::resource::test_github::observe(initial, &base).await.unwrap();
        assert_eq!(baseline.facts["pull_requests"][0]["head"], "old-head");
        let cursor: Value = serde_json::from_str(baseline.cursor.as_deref().unwrap()).unwrap();
        let since = chrono::DateTime::parse_from_rfc3339(cursor["items_since"].as_str().unwrap()).unwrap()
            - chrono::Duration::seconds(1);
        github.route(&format!(
            "/repos/example/project/issues?state=all&sort=updated&direction=asc&since={}&per_page=100",
            since.format("%Y-%m-%dT%H:%M:%SZ")
        ), json!([]));

        github.open_pull_requests(json!([live_pull("new-head")]));
        let ledger = std::sync::Arc::new(RecordingCapture {
            calls: std::sync::atomic::AtomicUsize::new(0), fail: true,
            progress: CaptureProgress { active: true, continue_after_ms: Some(100) },
        });
        let mut request = observer_request(Some(ledger.clone()));
        request.fields.insert("pull_requests".into());
        request.cursor = baseline.cursor;
        request.previous_facts = Some(baseline.facts);
        let warnings = CaptureWarnings::default();
        let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let observed = crate::resource::test_github::observe(request, &base)
            .with_subscriber(warnings.clone()).await.unwrap();
        let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        assert_eq!(observed.facts["pull_requests"][0]["head"], "new-head",
            "live watch facts survive a fenced capture publication");
        assert!(observed.facts.get("github_capture").is_none(),
            "capture status is separate from watch-facing facts");
        assert!(observed.cursor.is_some(), "live observation progress is retained");
        assert!(observed.next_check_unix_ms >= before + 300_000);
        assert!(observed.next_check_unix_ms <= after + 300_000,
            "a fenced source does not request accelerated continuation");
        assert_eq!(ledger.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let warnings = warnings.0.lock().unwrap();
        let failure = warnings.iter().find(|fields| fields.get("capture_status")
            .is_some_and(|status| status == "\"failed\"")).expect("capture failure is reported independently");
        assert_eq!(failure["repository"], "\"example/project\"");
        assert!(failure["error"].contains("capture publication rejected"));
        assert!(failure["error"].contains("the selected source writer has been fenced and cannot resume"),
            "the original cause is not hidden by its publication context");
    }
}
