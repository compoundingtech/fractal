//! The narrow main Performance failure field of the existing repository observer: a failed
//! `perf.yml` push run on main, or a daily long-lanes run on main whose `daily-perf-` lane failed.
//!
//! The same observation, when its caller passes a [`WorkflowRunLedger`], records each recent
//! completed Workspace CI run (`fleet.yml`) once, as one `github.workflow-run` claim: CI's declared
//! objectives read those claims, never the observed facts. Before a run is written its artifacts
//! are listed once, for the tests nextest passed only on a retry (`nextest-flaky-<lane>-<N>`).
use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::github_http::GithubAuth;

pub const PERFORMANCE_FAILURES_FIELD: &str = "main_performance_failures";
const WORKFLOW_PATH: &str = ".github/workflows/perf.yml";
const DAILY_WORKFLOW: &str = "Daily long lanes";
const DAILY_WORKFLOW_PATH: &str = ".github/workflows/daily-long.yml";
/// The daily lanes that are performance lanes, by job name.
const DAILY_PERF_JOB_PREFIX: &str = "daily-perf-";
/// The newest failed daily runs whose jobs are read in one observation. The daily lanes fail at
/// most a few times a day, so a new failure is always among them; older ones were already
/// delivered or baselined, and receipts keep them from being sent again.
const DAILY_RUNS_INSPECTED: usize = 10;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PerformanceFailure {
    pub repository: String,
    pub run_id: u64,
    pub run_attempt: u64,
    pub head_sha: String,
    pub workflow: String,
    pub workflow_path: String,
    pub event: String,
    pub head_branch: String,
    pub status: String,
    pub conclusion: String,
    pub url: String,
    /// The failed `daily-perf-` jobs of a daily long-lanes attempt; empty for `perf.yml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_jobs: Vec<String>,
}

impl PerformanceFailure {
    fn valid(&self) -> bool {
        let Some((owner, repo)) = self.repository.split_once('/') else {
            return false;
        };
        let component = |value: &str| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        };
        component(owner)
            && component(repo)
            && self.run_id > 0
            && self.run_attempt > 0
            && self.head_sha.len() == 40
            && self.head_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            && (self.performance_push() || self.daily_perf())
            && self.head_branch == "main"
            && self.status == "completed"
            && self.conclusion == "failure"
            && self.url
                == format!(
                    "https://github.com/{}/actions/runs/{}",
                    self.repository, self.run_id
                )
    }

    fn performance_push(&self) -> bool {
        self.workflow == "Performance"
            && self.workflow_path == WORKFLOW_PATH
            && self.event == "push"
            && self.failed_jobs.is_empty()
    }

    fn daily_perf(&self) -> bool {
        self.workflow == DAILY_WORKFLOW
            && self.workflow_path == DAILY_WORKFLOW_PATH
            && matches!(self.event.as_str(), "schedule" | "workflow_dispatch")
            && !self.failed_jobs.is_empty()
            && self.failed_jobs.iter().all(|job| {
                job.starts_with(DAILY_PERF_JOB_PREFIX)
                    && job.len() <= 200
                    && !job.chars().any(char::is_control)
            })
    }

    /// What failed, for a message title: the Performance workflow, or the daily perf lanes.
    pub fn failed(&self) -> String {
        if self.failed_jobs.is_empty() {
            self.workflow.clone()
        } else {
            self.failed_jobs.join(", ")
        }
    }

    pub fn from_facts(facts: &Value) -> Option<Self> {
        let failure: Self = serde_json::from_value(facts.clone()).ok()?;
        failure.valid().then_some(failure)
    }

    /// A daily long-lanes run that is a failure once a `daily-perf-` job of it failed.
    fn daily_run(&self) -> bool {
        Self {
            failed_jobs: vec![DAILY_PERF_JOB_PREFIX.to_owned()],
            ..self.clone()
        }
        .valid()
    }

    /// The run's metadata, not yet validated: a daily run's failed jobs are read separately.
    fn from_run(repository: &str, run: &Value) -> Option<Self> {
        if run
            .pointer("/repository/full_name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.eq_ignore_ascii_case(repository))
        {
            return None;
        }
        let text = |field: &str| run.get(field).and_then(Value::as_str).map(str::to_owned);
        let run_id = run.get("id")?.as_u64()?;
        let path = text("path")?;
        // GitHub may suffix the workflow file path with its ref.
        let workflow_path = path
            .split_once('@')
            .map_or(path.as_str(), |(path, _)| path)
            .to_owned();
        let failure = Self {
            repository: repository.to_ascii_lowercase(),
            run_id,
            run_attempt: run.get("run_attempt")?.as_u64()?,
            head_sha: text("head_sha")?.to_ascii_lowercase(),
            workflow: text("name")?,
            workflow_path,
            event: text("event")?,
            head_branch: text("head_branch")?,
            status: text("status")?,
            conclusion: text("conclusion")?,
            url: format!(
                "https://github.com/{}/actions/runs/{run_id}",
                repository.to_ascii_lowercase()
            ),
            failed_jobs: Vec::new(),
        };
        Some(failure)
    }
}

/// Workspace CI, whose completed runs are recorded as `github.workflow-run` claims.
const WORKSPACE_WORKFLOW: &str = "Workspace CI";
const WORKSPACE_WORKFLOW_PATH: &str = ".github/workflows/fleet.yml";
/// A merge group's head branch: `gh-readonly-queue/BASE/pr-N-SHA`.
const MERGE_GROUP_BRANCH_PREFIX: &str = "gh-readonly-queue/";
/// The claims' subject family and field version, as `fractal_schema::github_plugin`'s
/// `WORKFLOW_RUN_FAMILY` and `VERSION` name them.
const WORKFLOW_RUN_FAMILY: &str = "github-workflow-run";
const WORKFLOW_RUN_VERSION: i64 = 1;
const DAY_MS: u64 = 86_400_000;
const HOUR_MS: u64 = 3_600_000;
/// How long after it completed a run is still written. Its flaky artifacts are kept for a day,
/// so a run written later could no longer name its flaky tests.
const WRITE_WINDOW_MS: u64 = 20 * HOUR_MS;
/// A test lane's artifact whose name carries nextest's flaky test count: `nextest-flaky-<lane>-<N>`.
const FLAKY_ARTIFACT_PREFIX: &str = "nextest-flaky-";
/// The most runs one poll writes, each after one artifacts listing, so a restart after a busy
/// day cannot burst; the rest are written on the next polls while they are in the window.
const WRITES_PER_POLL: usize = 20;

/// Where a repository's poller records each completed Workspace CI run, once.
pub trait WorkflowRunLedger: Send + Sync {
    /// Whether `subject` already holds a claim (this poller's or a replicated one).
    fn written(&self, subject: &str) -> Result<bool>;
    /// Record one run: its subject and its claim fields. The ledger adds the `observer` field,
    /// the observer whose poll read the run.
    fn write(&self, subject: &str, fields: &BTreeMap<String, Value>) -> Result<()>;
}

impl std::fmt::Debug for dyn WorkflowRunLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkflowRunLedger")
    }
}

/// The subject of `repository`'s run `run_id`: `github-workflow-run/OWNER/REPO/RUN_ID`, the
/// repository lowercased as in `resource/github/OWNER/REPO`.
pub fn workflow_run_subject(repository: &str, run_id: u64) -> String {
    format!("{WORKFLOW_RUN_FAMILY}/{}/{run_id}", repository.to_ascii_lowercase())
}

/// One completed Workspace CI run: a pull request's or a merge group's, with GitHub's run-level
/// timestamps as it wrote them. `completed_at` is the run's `updated_at`, which a completed run
/// last changes when it completes; it stands in for the last job's completion, which would need
/// a jobs read per run.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowRun {
    /// The repository as GitHub names it.
    repository: String,
    run_id: u64,
    /// 1 for the first attempt. A re-run restarts `run_started_at` and reruns only some jobs.
    attempt: u64,
    /// `pull_request` or `merge_group`.
    event: String,
    conclusion: String,
    head_sha: String,
    created_at: String,
    started_at: String,
    completed_at: String,
    completed_unix_ms: u64,
    duration_ms: u64,
}

impl WorkflowRun {
    /// A completed Workspace CI run of `repository` from the list-runs answer, or None for any
    /// other run.
    fn from_run(repository: &str, run: &Value) -> Option<Self> {
        let named = run.pointer("/repository/full_name").and_then(Value::as_str);
        if named.is_some_and(|name| !name.eq_ignore_ascii_case(repository)) {
            return None;
        }
        let text = |field: &str| run.get(field).and_then(Value::as_str);
        let at = |field: &str| {
            let written = text(field)?;
            let at = chrono::DateTime::parse_from_rfc3339(written).ok()?;
            Some((written.to_owned(), u64::try_from(at.timestamp_millis()).ok()?))
        };
        let path = text("path")?;
        let path = path.split_once('@').map_or(path, |(path, _)| path);
        let event = text("event")?;
        let queued = text("head_branch").is_some_and(|branch| branch.starts_with(MERGE_GROUP_BRANCH_PREFIX));
        let workspace = text("name")? == WORKSPACE_WORKFLOW && path == WORKSPACE_WORKFLOW_PATH;
        let kind = match event {
            "pull_request" => !queued,
            "merge_group" => queued,
            _ => false,
        };
        if !workspace || !kind || text("status")? != "completed" {
            return None;
        }
        let (created_at, created_unix_ms) = at("created_at")?;
        let (started_at, started_unix_ms) = at("run_started_at")?;
        let (completed_at, completed_unix_ms) = at("updated_at")?;
        if started_unix_ms > completed_unix_ms || created_unix_ms > completed_unix_ms {
            return None;
        }
        Some(Self {
            repository: named.unwrap_or(repository).to_owned(),
            run_id: run.get("id")?.as_u64()?,
            attempt: run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1),
            event: event.to_owned(),
            conclusion: text("conclusion")?.to_owned(),
            head_sha: text("head_sha")?.to_owned(),
            created_at,
            started_at,
            completed_at,
            completed_unix_ms,
            duration_ms: completed_unix_ms - started_unix_ms,
        })
    }

    fn subject(&self) -> String {
        workflow_run_subject(&self.repository, self.run_id)
    }

    /// The run's `github.workflow-run` fields but `observer`, with the flaky tests its artifacts
    /// named. Its flakes are those tests, and one more for a merge group it ejected by failing.
    fn fields(&self, flaky_tests: u64) -> BTreeMap<String, Value> {
        let ejected = self.event == "merge_group" && self.conclusion == "failure";
        BTreeMap::from([
            ("version".to_owned(), Value::from(WORKFLOW_RUN_VERSION)),
            ("repository".to_owned(), Value::from(self.repository.as_str())),
            ("workflow".to_owned(), Value::from(WORKSPACE_WORKFLOW)),
            ("run_id".to_owned(), Value::from(self.run_id)),
            ("attempt".to_owned(), Value::from(self.attempt)),
            ("event".to_owned(), Value::from(self.event.as_str())),
            ("conclusion".to_owned(), Value::from(self.conclusion.as_str())),
            ("head_sha".to_owned(), Value::from(self.head_sha.as_str())),
            ("created_at".to_owned(), Value::from(self.created_at.as_str())),
            ("started_at".to_owned(), Value::from(self.started_at.as_str())),
            ("completed_at".to_owned(), Value::from(self.completed_at.as_str())),
            ("duration_ms".to_owned(), Value::from(self.duration_ms)),
            ("flaky_tests".to_owned(), Value::from(flaky_tests)),
            ("flakes".to_owned(), Value::from(flaky_tests + u64::from(ejected))),
        ])
    }
}

/// The count a `nextest-flaky-<lane>-<N>` artifact names, or None for any other artifact.
fn flaky_count(name: &str) -> Option<u64> {
    let (lane, count) = name.strip_prefix(FLAKY_ARTIFACT_PREFIX)?.rsplit_once('-')?;
    if lane.is_empty() || count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    count.parse().ok()
}

/// The flaky tests a run's artifacts name, summed over its lanes (and attempts).
fn flaky_tests(artifacts: &[Value]) -> u64 {
    artifacts
        .iter()
        .filter_map(|artifact| artifact.get("name").and_then(Value::as_str))
        .filter_map(flaky_count)
        .sum()
}

/// The runs this poll writes: those that completed in the write window and that `ledger` does
/// not hold yet, newest completed first, at most `WRITES_PER_POLL`.
fn unwritten<'a>(
    runs: &'a [WorkflowRun],
    ledger: &dyn WorkflowRunLedger,
    now_unix_ms: u64,
) -> Result<Vec<&'a WorkflowRun>> {
    let since = now_unix_ms.saturating_sub(WRITE_WINDOW_MS);
    let mut recent = runs.iter().filter(|run| run.completed_unix_ms > since).collect::<Vec<_>>();
    recent.sort_unstable_by_key(|run| std::cmp::Reverse((run.completed_unix_ms, run.run_id)));
    let mut unwritten = Vec::new();
    for run in recent {
        if unwritten.len() == WRITES_PER_POLL {
            break;
        }
        if !ledger.written(&run.subject())? {
            unwritten.push(run);
        }
    }
    Ok(unwritten)
}

/// The completed pull request and merge group runs of Workspace CI created since the start of
/// the UTC day before `now_unix_ms`'s, which covers the write window. Two list-runs reads on the
/// workflow the observer already reads runs from, through the same conditional cache; the date
/// in the query changes once a day, so an unchanged listing still revalidates for free.
async fn completed_workspace_runs(
    client: &reqwest::Client,
    base: &str,
    repository: &str,
    auth: &GithubAuth,
    cache_for: Duration,
    now_unix_ms: u64,
) -> Result<Vec<WorkflowRun>> {
    let since = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(now_unix_ms.saturating_sub(DAY_MS)).unwrap_or_default(),
    )
    .unwrap_or_default()
    .format("%Y-%m-%d")
    .to_string();
    let mut listed = Vec::new();
    for filter in ["event=pull_request&status=completed", "event=merge_group&status=completed"] {
        // Ten pages is GitHub's 1,000-result search bound. An overflow fails rather than truncates.
        let listing = super::github_listing_at_field(
            client,
            format!("{base}/actions/workflows/fleet.yml/runs?{filter}&created=%3E%3D{since}&per_page=100"),
            auth, cache_for, 10, false, Some("workflow_runs"),
        ).await?;
        // A page short of what the listing's total says (the same filter's other failure) fails the read too.
        let expected = listing.total_count.unwrap_or_default().min(1_000);
        anyhow::ensure!(
            listing.values.len() as u64 >= expected,
            "fleet.yml listed {} {filter} runs created since {since} of the {expected} its total names",
            listing.values.len()
        );
        if listing.values.is_empty() {
            // The created= filter now and then answers an empty listing while the runs exist (2026-10-22: two
            // requests in ten during a burst). The same filter without created= names its newest run, and one
            // inside the window fails the read instead of reading as an idle day.
            let newest = super::github_json(
                client, format!("{base}/actions/workflows/fleet.yml/runs?{filter}&per_page=1"), auth, cache_for,
            ).await?;
            let created = newest.value.pointer("/workflow_runs/0/created_at").and_then(Value::as_str).unwrap_or_default();
            anyhow::ensure!(
                created < since.as_str(),
                "fleet.yml listed no {filter} runs created since {since}, but its newest was created at {created}"
            );
        }
        listed.extend(listing.values);
    }
    let mut runs = workspace_runs(repository, &listed)?;
    runs.sort_by_key(|run| run.run_id);
    runs.dedup_by_key(|run| run.run_id);
    Ok(runs)
}

/// The Workspace CI runs among `listed`. Runs listed but none of them Workspace CI's means the
/// workflow's name or path changed: that fails, with a warning, instead of reading as an idle day.
fn workspace_runs(repository: &str, listed: &[Value]) -> Result<Vec<WorkflowRun>> {
    let runs = listed.iter().filter_map(|run| WorkflowRun::from_run(repository, run)).collect::<Vec<_>>();
    anyhow::ensure!(
        listed.is_empty() || !runs.is_empty(),
        "fleet.yml listed {} completed runs, none named {WORKSPACE_WORKFLOW:?} at {WORKSPACE_WORKFLOW_PATH}",
        listed.len()
    );
    Ok(runs)
}

/// Record `repository`'s recent completed Workspace CI runs in `ledger`, each once: list each
/// unwritten run's artifacts once for its flaky tests, then write its claim. The ledger is the
/// only memory of what was written. A failed read, listing or ledger call is logged and never
/// fails the observation; it ends this poll's writes, and a later poll writes the rest.
pub(crate) async fn record_workflow_runs(
    client: &reqwest::Client,
    base: &str,
    repository: &str,
    auth: &GithubAuth,
    cache_for: Duration,
    ledger: &dyn WorkflowRunLedger,
    now_unix_ms: u64,
) {
    let runs = match completed_workspace_runs(client, base, repository, auth, cache_for, now_unix_ms).await {
        Ok(runs) => runs,
        Err(error) => {
            tracing::warn!(repository, error = %error, "could not read Workspace CI runs");
            return;
        }
    };
    let unwritten = match unwritten(&runs, ledger, now_unix_ms) {
        Ok(unwritten) => unwritten,
        Err(error) => {
            tracing::warn!(repository, error = %error, "could not read which Workspace CI runs are recorded");
            return;
        }
    };
    for run in unwritten {
        // At most 100 artifacts: a longer listing fails rather than truncates.
        let url = format!("{base}/actions/runs/{}/artifacts?per_page=100", run.run_id);
        let artifacts = match super::github_listing_at_field(client, url.clone(), auth, cache_for, 1, false, Some("artifacts")).await {
            Ok(artifacts) => artifacts,
            Err(error) => {
                tracing::warn!(run_id = run.run_id, error = %error, "could not list a Workspace CI run's artifacts");
                break;
            }
        };
        // Each run is listed once, so its cached answer would never be revalidated.
        super::github_forget(&url).await;
        if let Err(error) = ledger.write(&run.subject(), &run.fields(flaky_tests(&artifacts.values))) {
            tracing::warn!(run_id = run.run_id, error = %error, "could not record a Workspace CI run");
            break;
        }
    }
}

pub async fn main_performance_failures(
    client: &reqwest::Client,
    base: &str,
    repository: &str,
    auth: &GithubAuth,
    cache_for: Duration,
) -> Result<Vec<PerformanceFailure>> {
    // These are metadata reads only, through the repository's existing conditional HTTP cache.
    // Ten pages is GitHub's 1,000-result search bound. An overflow fails rather than truncates.
    let runs = |query: &str| super::github_listing_at_field(
        client,
        format!("{base}/actions/workflows/{query}&per_page=100"),
        auth, cache_for, 10, false, Some("workflow_runs"),
    );
    let performance = runs("perf.yml/runs?branch=main&event=push&status=failure").await?;
    let mut failures = performance
        .values
        .iter()
        .filter_map(|run| PerformanceFailure::from_run(repository, run))
        .filter(PerformanceFailure::valid)
        .collect::<Vec<_>>();
    // The daily lanes run by schedule or dispatch; the event is checked per run.
    let daily = runs("daily-long.yml/runs?branch=main&status=failure").await?;
    let mut daily = daily
        .values
        .iter()
        .filter_map(|run| PerformanceFailure::from_run(repository, run))
        .filter(PerformanceFailure::daily_run)
        .collect::<Vec<_>>();
    daily.sort_by_key(|failure| std::cmp::Reverse(failure.run_id));
    daily.dedup_by_key(|failure| failure.run_id);
    for mut failure in daily.into_iter().take(DAILY_RUNS_INSPECTED) {
        // One attempt's jobs, at most 100: a longer listing fails rather than truncates.
        let jobs = super::github_listing_at_field(
            client,
            format!("{base}/actions/runs/{}/attempts/{}/jobs?per_page=100",
                failure.run_id, failure.run_attempt),
            auth, cache_for, 1, false, Some("jobs"),
        ).await?;
        failure.failed_jobs = jobs
            .values
            .iter()
            .filter(|job| job.get("conclusion").and_then(Value::as_str) == Some("failure"))
            .filter_map(|job| job.get("name").and_then(Value::as_str))
            .filter(|name| name.starts_with(DAILY_PERF_JOB_PREFIX))
            .map(str::to_owned)
            .collect();
        failure.failed_jobs.sort();
        failure.failed_jobs.dedup();
        if failure.valid() {
            failures.push(failure);
        }
    }
    failures.sort_by(|left, right| {
        (left.run_id, left.run_attempt, &left.head_sha).cmp(&(
            right.run_id,
            right.run_attempt,
            &right.head_sha,
        ))
    });
    failures.dedup();
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn daily(failed_jobs: Value) -> Value {
        json!({"repository": "acme/garden", "run_id": 9, "run_attempt": 1,
            "head_sha": "d".repeat(40), "workflow": "Daily long lanes",
            "workflow_path": ".github/workflows/daily-long.yml", "event": "schedule",
            "head_branch": "main", "status": "completed", "conclusion": "failure",
            "url": "https://github.com/acme/garden/actions/runs/9", "failed_jobs": failed_jobs})
    }

    /// A completed Workspace CI run as GitHub's list-runs answer has it: created at `created`,
    /// started 30 seconds later and last updated `minutes` after its start.
    fn workspace(id: u64, event: &str, conclusion: &str, created: &str, minutes: i64) -> Value {
        let created_at = chrono::DateTime::parse_from_rfc3339(created).unwrap();
        let started = created_at + chrono::TimeDelta::seconds(30);
        let updated = started + chrono::TimeDelta::minutes(minutes);
        let branch = if event == "merge_group" { format!("gh-readonly-queue/main/pr-{id}-{}", "e".repeat(40)) } else { "feature".to_owned() };
        json!({"id": id, "name": "Workspace CI", "path": ".github/workflows/fleet.yml@refs/heads/main",
            "event": event, "head_branch": branch, "status": "completed", "conclusion": conclusion,
            "head_sha": "a".repeat(40), "created_at": created, "run_started_at": started.to_rfc3339(),
            "updated_at": updated.to_rfc3339(), "repository": {"full_name": "Acme/Garden"}})
    }

    #[test]
    fn only_completed_pull_request_and_merge_group_runs_of_the_workspace_workflow_are_runs() {
        let run = workspace(1, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        let parsed = WorkflowRun::from_run("acme/garden", &run).unwrap();
        assert_eq!((parsed.repository.as_str(), parsed.attempt, parsed.duration_ms), ("Acme/Garden", 1, 540_000));
        assert_eq!(parsed.created_at, "2026-10-20T10:00:00Z", "GitHub's times are kept as it wrote them");
        assert!(WorkflowRun::from_run("ACME/garden", &workspace(2, "merge_group", "failure", "2026-10-20T10:00:00Z", 9)).is_some());
        let mut rerun = workspace(5, "pull_request", "success", "2026-10-20T10:00:00Z", 3);
        rerun["run_attempt"] = json!(2);
        assert_eq!(WorkflowRun::from_run("acme/garden", &rerun).unwrap().attempt, 2);
        for (field, wrong) in [
            ("name", json!("Performance")), ("path", json!(".github/workflows/perf.yml")),
            ("event", json!("push")), ("event", json!("workflow_dispatch")),
            ("status", json!("in_progress")), ("conclusion", json!(null)), ("head_sha", json!(null)),
            ("head_branch", json!("gh-readonly-queue/main/pr-1-abc")),
            ("run_started_at", json!("not a time")), ("updated_at", json!("2026-10-20T09:00:00Z")),
            ("repository", json!({"full_name": "another/repository"})),
        ] {
            let mut excluded = run.clone();
            excluded[field] = wrong;
            assert!(WorkflowRun::from_run("acme/garden", &excluded).is_none(), "{field}");
        }
        let mut unqueued = workspace(4, "merge_group", "success", "2026-10-20T10:00:00Z", 9);
        unqueued["head_branch"] = json!("main");
        assert!(WorkflowRun::from_run("acme/garden", &unqueued).is_none());
    }

    #[test]
    fn a_run_claim_carries_its_times_and_counts_an_ejected_group_as_a_flake() {
        let group = WorkflowRun::from_run("acme/garden", &workspace(9001, "merge_group", "failure", "2026-10-20T10:00:00Z", 7)).unwrap();
        assert_eq!(group.subject(), "github-workflow-run/acme/garden/9001");
        let fields = group.fields(2);
        assert_eq!(Value::Object(fields.clone().into_iter().collect()), json!({
            "version": 1, "repository": "Acme/Garden", "workflow": "Workspace CI", "run_id": 9001,
            "attempt": 1, "event": "merge_group", "conclusion": "failure", "head_sha": "a".repeat(40),
            "created_at": "2026-10-20T10:00:00Z", "started_at": "2026-10-20T10:00:30+00:00",
            "completed_at": "2026-10-20T10:07:30+00:00", "duration_ms": 420_000,
            "flaky_tests": 2, "flakes": 3,
        }));
        let green = WorkflowRun::from_run("acme/garden", &workspace(9002, "merge_group", "success", "2026-10-20T10:00:00Z", 7)).unwrap();
        assert_eq!(green.fields(0)["flakes"], 0);
        let pull = WorkflowRun::from_run("acme/garden", &workspace(9003, "pull_request", "failure", "2026-10-20T10:00:00Z", 7)).unwrap();
        assert_eq!(pull.fields(1)["flakes"], 1, "a failed pull request run ejects no group");
    }

    #[test]
    fn a_renamed_workflow_fails_the_read() {
        assert!(workspace_runs("acme/garden", &[]).unwrap().is_empty(), "an idle day reads as no runs");
        let mut renamed = workspace(1, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        renamed["name"] = json!("Workspace checks");
        let error = workspace_runs("acme/garden", &[renamed.clone()]).unwrap_err().to_string();
        assert!(error.contains("none named \"Workspace CI\""), "{error}");
        let kept = workspace(2, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        assert_eq!(workspace_runs("acme/garden", &[renamed, kept]).unwrap().len(), 1);
    }

    const PULLS: &str = "event=pull_request&status=completed";
    const GROUPS: &str = "event=merge_group&status=completed";

    /// A Workspace CI run that completed about `minutes_ago` before `now`, ten minutes after it
    /// was created.
    fn completed(id: u64, event: &str, conclusion: &str, now: chrono::DateTime<chrono::Utc>, minutes_ago: i64) -> Value {
        workspace(id, event, conclusion, &(now - chrono::TimeDelta::minutes(minutes_ago + 10)).to_rfc3339(), 10)
    }

    /// The fake GitHub and the repository's API base on it.
    async fn fake_repository() -> (crate::resource::test_github::FakeGithub, String) {
        let (github, base) = crate::resource::test_github::FakeGithub::start().await;
        (github, format!("{base}/repos/acme/garden"))
    }

    fn listing(filter: &str, now: chrono::DateTime<chrono::Utc>) -> String {
        let since = (now - chrono::TimeDelta::days(1)).format("%Y-%m-%d");
        format!("/repos/acme/garden/actions/workflows/fleet.yml/runs?{filter}&created=%3E%3D{since}&per_page=100")
    }

    fn newest(filter: &str) -> String {
        format!("/repos/acme/garden/actions/workflows/fleet.yml/runs?{filter}&per_page=1")
    }

    fn artifacts(id: u64) -> String {
        format!("/repos/acme/garden/actions/runs/{id}/artifacts?per_page=100")
    }

    /// The artifacts listings among `requests`, in the order they were made.
    fn artifact_listings(requests: Vec<String>) -> Vec<String> {
        requests.into_iter().filter(|request| request.contains("/artifacts?")).collect()
    }

    fn get(path: &str) -> String {
        format!("GET {path} HTTP/1.1")
    }

    async fn poll(base: &str, ledger: &crate::resource::test_github::MemoryLedger, now: chrono::DateTime<chrono::Utc>) {
        let auth = GithubAuth::test("fixture-auth");
        record_workflow_runs(&reqwest::Client::new(), base, "acme/garden", &auth, Duration::ZERO, ledger,
            now.timestamp_millis() as u64).await;
    }

    #[tokio::test]
    async fn an_empty_or_short_listing_fails_the_read_and_a_quiet_one_does_not() {
        let (github, base) = fake_repository().await;
        let now = chrono::Utc::now();
        // The created= filter answers nothing for pull requests, but their newest run is from today.
        github.route(&listing(PULLS, now), json!({"total_count": 0, "workflow_runs": []}));
        github.route(&listing(GROUPS, now), json!({"total_count": 0, "workflow_runs": []}));
        github.route(&newest(PULLS), json!({"total_count": 9, "workflow_runs": [{"id": 1, "created_at": now.to_rfc3339()}]}));
        github.route(&newest(GROUPS), json!({"total_count": 9, "workflow_runs": [{"id": 2, "created_at": "2020-01-01T00:00:00Z"}]}));
        let client = reqwest::Client::new();
        let auth = GithubAuth::test("fixture-auth");
        let now_ms = now.timestamp_millis() as u64;
        let error = completed_workspace_runs(&client, &base, "acme/garden", &auth, Duration::ZERO, now_ms).await.unwrap_err();
        assert!(error.to_string().contains("listed no event=pull_request&status=completed runs"), "{error}");
        // Newest runs from before the window: the empty listing is a quiet day.
        github.route(&newest(PULLS), json!({"total_count": 9, "workflow_runs": [{"id": 1, "created_at": "2020-01-01T00:00:00Z"}]}));
        let quiet = completed_workspace_runs(&client, &base, "acme/garden", &auth, Duration::ZERO, now_ms).await.unwrap();
        assert!(quiet.is_empty());
        // A listing short of its own total fails too.
        github.route(&listing(PULLS, now), json!({"total_count": 2, "workflow_runs": [{"id": 3, "created_at": now.to_rfc3339()}]}));
        let error = completed_workspace_runs(&client, &base, "acme/garden", &auth, Duration::ZERO, now_ms).await.unwrap_err();
        assert!(error.to_string().contains("listed 1 event=pull_request&status=completed runs") && error.to_string().contains("of the 2"), "{error}");
    }

    #[tokio::test]
    async fn a_failed_artifact_listing_writes_nothing_for_its_run_and_ends_the_poll() {
        let (github, base) = fake_repository().await;
        let now = chrono::Utc::now();
        github.route(&listing(PULLS, now), json!({"total_count": 1, "workflow_runs": [
            completed(3, "pull_request", "success", now, 3)]}));
        github.route(&listing(GROUPS, now), json!({"total_count": 2, "workflow_runs": [
            completed(1, "merge_group", "failure", now, 1), completed(2, "merge_group", "success", now, 2)]}));
        github.route(&artifacts(1), json!({"total_count": 1, "artifacts": [{"name": "nextest-flaky-linux-tests-2"}]}));
        github.route(&artifacts(3), json!({"total_count": 0, "artifacts": []}));
        let ledger = crate::resource::test_github::MemoryLedger::default();
        poll(&base, &ledger, now).await;
        assert_eq!(artifact_listings(github.take_requests()), [get(&artifacts(1)), get(&artifacts(2))],
            "newest first, and the failed listing ends the poll");
        assert_eq!(ledger.runs.lock().keys().cloned().collect::<Vec<_>>(), [workflow_run_subject("acme/garden", 1)]);
        assert_eq!(ledger.runs.lock()[&workflow_run_subject("acme/garden", 1)]["flakes"], 3, "two flaky tests and the ejected group");
        // A later poll writes the rest; a written run is never listed or written again.
        github.route(&artifacts(2), json!({"total_count": 0, "artifacts": []}));
        poll(&base, &ledger, now).await;
        assert_eq!(artifact_listings(github.take_requests()), [get(&artifacts(2)), get(&artifacts(3))]);
        assert_eq!(ledger.runs.lock().len(), 3);
        poll(&base, &ledger, now).await;
        assert!(artifact_listings(github.take_requests()).is_empty());
        assert_eq!(ledger.runs.lock().len(), 3);
    }

    #[tokio::test]
    async fn a_poll_writes_at_most_twenty_runs_and_none_that_completed_over_twenty_hours_ago() {
        let (github, base) = fake_repository().await;
        let now = chrono::Utc::now();
        // Thirty runs completed a minute apart, run 30 the newest, and one 21 hours ago.
        let runs = (1..=30)
            .map(|id| completed(id, "pull_request", "success", now, 60 - id as i64))
            .chain([completed(31, "pull_request", "success", now, 21 * 60)])
            .collect::<Vec<_>>();
        github.route(&listing(PULLS, now), json!({"total_count": 31, "workflow_runs": runs}));
        github.route(&listing(GROUPS, now), json!({"total_count": 0, "workflow_runs": []}));
        github.route(&newest(GROUPS), json!({"total_count": 9, "workflow_runs": [{"id": 99, "created_at": "2020-01-01T00:00:00Z"}]}));
        for id in 1..=31 {
            github.route(&artifacts(id), json!({"total_count": 0, "artifacts": []}));
        }
        let ledger = crate::resource::test_github::MemoryLedger::default();
        poll(&base, &ledger, now).await;
        assert_eq!(artifact_listings(github.take_requests()), (11..=30).rev().map(|id| get(&artifacts(id))).collect::<Vec<_>>(),
            "newest first, at most twenty");
        poll(&base, &ledger, now).await;
        assert_eq!(artifact_listings(github.take_requests()), (1..=10).rev().map(|id| get(&artifacts(id))).collect::<Vec<_>>(),
            "the rest on the next poll");
        poll(&base, &ledger, now).await;
        assert!(artifact_listings(github.take_requests()).is_empty());
        assert_eq!(ledger.runs.lock().len(), 30);
        assert!(!ledger.runs.lock().contains_key(&workflow_run_subject("acme/garden", 31)), "its flaky artifacts may be gone");
    }

    #[test]
    fn flaky_artifacts_name_their_lane_and_count() {
        assert_eq!(flaky_count("nextest-flaky-linux-tests-1"), Some(1));
        assert_eq!(flaky_count("nextest-flaky-linux-tests-shard-3-12"), Some(12));
        assert_eq!(flaky_count("nextest-flaky-lane-0"), Some(0));
        for other in ["linux-tests-logs", "nextest-flaky-", "nextest-flaky-7", "nextest-flaky--7",
            "nextest-flaky-linux-tests-", "nextest-flaky-linux-tests-x", "nextest-flaky-linux-tests-+3",
            "nextest-flaky-linux-tests-99999999999999999999999", "evidence-note-nextest-flaky-a-1"] {
            assert_eq!(flaky_count(other), None, "{other}");
        }
        let artifacts = [json!({"name": "linux-tests-logs"}), json!({"name": "nextest-flaky-linux-tests-2"}),
            json!({"name": "nextest-flaky-linux-tests-shard-4-1"}), json!({"id": 3}), json!({"name": 7})];
        assert_eq!(flaky_tests(&artifacts), 3);
        assert_eq!(flaky_tests(&[]), 0);
    }

    #[test]
    fn stored_performance_facts_without_failed_jobs_still_parse() {
        let stored = json!({"repository": "acme/garden", "run_id": 7, "run_attempt": 1,
            "head_sha": "a".repeat(40), "workflow": "Performance",
            "workflow_path": ".github/workflows/perf.yml", "event": "push", "head_branch": "main",
            "status": "completed", "conclusion": "failure",
            "url": "https://github.com/acme/garden/actions/runs/7"});
        let failure = PerformanceFailure::from_facts(&stored).unwrap();
        assert!(failure.failed_jobs.is_empty());
        assert_eq!(failure.failed(), "Performance");
        assert_eq!(serde_json::to_value(&failure).unwrap(), stored);
        let mut with_jobs = stored;
        with_jobs["failed_jobs"] = json!(["daily-perf-load"]);
        assert!(PerformanceFailure::from_facts(&with_jobs).is_none());
    }

    #[test]
    fn daily_failures_need_failed_perf_lanes_only() {
        let failure = PerformanceFailure::from_facts(&daily(json!(["daily-perf-load", "daily-perf-cost"])))
            .unwrap();
        assert_eq!(failure.failed(), "daily-perf-load, daily-perf-cost");
        let mut dispatched = daily(json!(["daily-perf-load"]));
        dispatched["event"] = json!("workflow_dispatch");
        assert!(PerformanceFailure::from_facts(&dispatched).is_some());
        for failed_jobs in [json!([]), json!(null), json!(["daily-tests"]),
            json!(["daily-perf-load", "daily-tests"])] {
            assert!(PerformanceFailure::from_facts(&daily(failed_jobs)).is_none());
        }
        let mut missing = daily(json!(null));
        missing.as_object_mut().unwrap().remove("failed_jobs");
        assert!(PerformanceFailure::from_facts(&missing).is_none());
        for (field, wrong) in [("event", json!("push")), ("head_branch", json!("feature")),
            ("workflow", json!("Performance")), ("workflow_path", json!(".github/workflows/perf.yml")),
            ("status", json!("in_progress")), ("conclusion", json!("success"))] {
            let mut excluded = daily(json!(["daily-perf-load"]));
            excluded[field] = wrong;
            assert!(PerformanceFailure::from_facts(&excluded).is_none());
        }
    }
}
