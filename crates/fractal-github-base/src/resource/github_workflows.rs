//! The narrow main Performance failure field of the existing repository observer: a failed
//! `perf.yml` push run on main, or a daily long-lanes run on main whose `daily-perf-` lane failed.
//!
//! The same observation reads the recent completed Workspace CI runs (`fleet.yml`) and keeps them
//! in this process only, for CI's objectives in `st slo`: they never enter the observed facts.
use std::sync::{Mutex, OnceLock, PoisonError};
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

/// Workspace CI, whose completed runs feed CI's objectives.
const WORKSPACE_WORKFLOW: &str = "Workspace CI";
const WORKSPACE_WORKFLOW_PATH: &str = ".github/workflows/fleet.yml";
/// A merge group's head branch: `gh-readonly-queue/BASE/pr-N-SHA`.
const MERGE_GROUP_BRANCH_PREFIX: &str = "gh-readonly-queue/";
/// The series these runs feed, as `slo/targets.toml` names them.
pub const CRITICAL_PATH_SERIES: &str = "ci/workspace-critical-path";
pub const MERGE_QUEUE_SERIES: &str = "ci/merge-queue";
pub const FLAKES_SERIES: &str = "ci/flakes";
const DAY_MS: u64 = 86_400_000;

/// One completed Workspace CI run: a pull request's or a merge group's, with GitHub's run-level
/// timestamps. `updated_unix_ms` is the run's `updated_at`, which a completed run last changes
/// when it completes; it stands in for the last job's completion, which would need a jobs read
/// per run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRun {
    pub run_id: u64,
    /// 1 for the first attempt. A re-run restarts `run_started_at` and reruns only some jobs, so
    /// a later attempt's timestamps do not measure the run.
    pub run_attempt: u64,
    /// `pull_request` or `merge_group`.
    pub event: String,
    pub conclusion: String,
    pub created_unix_ms: u64,
    pub started_unix_ms: u64,
    pub updated_unix_ms: u64,
}

impl WorkspaceRun {
    /// A completed Workspace CI run of `repository` from the list-runs answer, or None for any
    /// other run.
    pub fn from_run(repository: &str, run: &Value) -> Option<Self> {
        if run
            .pointer("/repository/full_name")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.eq_ignore_ascii_case(repository))
        {
            return None;
        }
        let text = |field: &str| run.get(field).and_then(Value::as_str);
        let at = |field: &str| {
            let at = chrono::DateTime::parse_from_rfc3339(text(field)?).ok()?;
            u64::try_from(at.timestamp_millis()).ok()
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
        let run = Self {
            run_id: run.get("id")?.as_u64()?,
            run_attempt: run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1),
            event: event.to_owned(),
            conclusion: text("conclusion")?.to_owned(),
            created_unix_ms: at("created_at")?,
            started_unix_ms: at("run_started_at")?,
            updated_unix_ms: at("updated_at")?,
        };
        (run.started_unix_ms <= run.updated_unix_ms && run.created_unix_ms <= run.updated_unix_ms)
            .then_some(run)
    }

    /// The duration this run adds to `series`, if it adds one: a successful pull request run
    /// from its start to its last update (`ci/workspace-critical-path`), or a successful merge
    /// group run from its creation, when the group entered the queue, to its last update, when it
    /// went green and the group could merge (`ci/merge-queue`). Only a first attempt adds one.
    fn duration_ms(&self, series: &str) -> Option<u64> {
        let from = match (series, self.event.as_str()) {
            (CRITICAL_PATH_SERIES, "pull_request") => self.started_unix_ms,
            (MERGE_QUEUE_SERIES, "merge_group") => self.created_unix_ms,
            _ => return None,
        };
        (self.conclusion == "success" && self.run_attempt == 1).then(|| self.updated_unix_ms - from)
    }
}

/// The recent completed Workspace CI runs as this process last read them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceCi {
    /// The repository the runs belong to. A daemon keeps one repository's runs: the first
    /// observer to read them.
    pub repository: String,
    pub observed_at_unix_ms: u64,
    pub runs: Vec<WorkspaceRun>,
}

impl WorkspaceCi {
    /// The durations `series` holds from runs that finished after `since_unix_ms` and by
    /// `until_unix_ms`, or None when these runs do not feed `series`.
    pub fn durations(&self, series: &str, since_unix_ms: u64, until_unix_ms: u64) -> Option<Vec<u64>> {
        if ![CRITICAL_PATH_SERIES, MERGE_QUEUE_SERIES].contains(&series) {
            return None;
        }
        Some(
            self.within(since_unix_ms, until_unix_ms)
                .filter_map(|run| run.duration_ms(series))
                .collect(),
        )
    }

    /// For `ci/flakes`: the merge groups whose Workspace CI run failed, ejecting the group, and
    /// all merge groups whose run finished, after `since_unix_ms` and by `until_unix_ms`. None
    /// when these runs do not feed `series`.
    pub fn ejected_groups(&self, series: &str, since_unix_ms: u64, until_unix_ms: u64) -> Option<(u64, u64)> {
        if series != FLAKES_SERIES {
            return None;
        }
        let groups = self.within(since_unix_ms, until_unix_ms).filter(|run| run.event == "merge_group");
        let (mut ejected, mut finished) = (0, 0);
        for run in groups {
            finished += 1;
            ejected += u64::from(run.conclusion == "failure");
        }
        Some((ejected, finished))
    }

    fn within(&self, since_unix_ms: u64, until_unix_ms: u64) -> impl Iterator<Item = &WorkspaceRun> {
        self.runs
            .iter()
            .filter(move |run| run.updated_unix_ms > since_unix_ms && run.updated_unix_ms <= until_unix_ms)
    }
}

fn workspace_ci_store() -> &'static Mutex<Option<WorkspaceCi>> {
    static STORE: OnceLock<Mutex<Option<WorkspaceCi>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// The Workspace CI runs this process last read, or None when it has read none: only the member
/// that runs a repository observer with `main_performance_failures` reads them.
pub fn workspace_ci() -> Option<WorkspaceCi> {
    workspace_ci_store().lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The completed pull request and merge group runs of Workspace CI created since the start of
/// the UTC day before `now_unix_ms`'s, which covers the last day. Two list-runs reads on the
/// workflow the observer already reads runs from, through the same conditional cache; the date
/// in the query changes once a day, so an unchanged listing still revalidates for free.
pub async fn workspace_ci_runs(
    client: &reqwest::Client,
    base: &str,
    repository: &str,
    auth: &GithubAuth,
    cache_for: Duration,
    now_unix_ms: u64,
) -> Result<WorkspaceCi> {
    let since = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(now_unix_ms.saturating_sub(DAY_MS)).unwrap_or_default(),
    )
    .unwrap_or_default()
    .format("%Y-%m-%d");
    let mut listed = Vec::new();
    for filter in ["event=pull_request&status=success", "event=merge_group&status=completed"] {
        // Ten pages is GitHub's 1,000-result search bound. An overflow fails rather than truncates.
        let listing = super::github_listing_at_field(
            client,
            format!("{base}/actions/workflows/fleet.yml/runs?{filter}&created=%3E%3D{since}&per_page=100"),
            auth, cache_for, 10, false, Some("workflow_runs"),
        ).await?;
        listed.extend(listing.values);
    }
    let mut runs = workspace_runs(repository, &listed)?;
    runs.sort_by_key(|run| run.run_id);
    runs.dedup_by_key(|run| run.run_id);
    Ok(WorkspaceCi { repository: repository.to_owned(), observed_at_unix_ms: now_unix_ms, runs })
}

/// The Workspace CI runs among `listed`. Runs listed but none of them Workspace CI's means the
/// workflow's name or path changed: that fails, so the last runs age out to no-data with a
/// warning instead of reading as an idle day.
fn workspace_runs(repository: &str, listed: &[Value]) -> Result<Vec<WorkspaceRun>> {
    let runs = listed.iter().filter_map(|run| WorkspaceRun::from_run(repository, run)).collect::<Vec<_>>();
    anyhow::ensure!(
        listed.is_empty() || !runs.is_empty(),
        "fleet.yml listed {} completed runs, none named {WORKSPACE_WORKFLOW:?} at {WORKSPACE_WORKFLOW_PATH}",
        listed.len()
    );
    Ok(runs)
}

/// Whether this daemon reads `repository`'s Workspace CI runs: it keeps one repository's, the
/// first it read.
fn reads_workspace_ci(kept: Option<&str>, repository: &str) -> bool {
    kept.is_none_or(|kept| kept.eq_ignore_ascii_case(repository))
}

/// Read the recent Workspace CI runs and keep them for `workspace_ci`. A failed read keeps the
/// last runs and never fails the observation: the objectives' windows age them out. One
/// repository per daemon: once one observer's runs are kept, another repository's are not read.
pub async fn observe_workspace_ci(
    client: &reqwest::Client,
    base: &str,
    repository: &str,
    auth: &GithubAuth,
    cache_for: Duration,
    now_unix_ms: u64,
) {
    let kept = workspace_ci().map(|ci| ci.repository);
    if !reads_workspace_ci(kept.as_deref(), repository) {
        tracing::warn!(repository, kept, "st slo keeps one repository's Workspace CI runs; not reading this one");
        return;
    }
    match workspace_ci_runs(client, base, repository, auth, cache_for, now_unix_ms).await {
        Ok(ci) => *workspace_ci_store().lock().unwrap_or_else(PoisonError::into_inner) = Some(ci),
        Err(error) => tracing::warn!(repository, error = %error, "could not read Workspace CI runs for st slo"),
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
            "created_at": created, "run_started_at": started.to_rfc3339(), "updated_at": updated.to_rfc3339(),
            "repository": {"full_name": "acme/garden"}})
    }

    fn unix_ms(at: &str) -> u64 {
        chrono::DateTime::parse_from_rfc3339(at).unwrap().timestamp_millis() as u64
    }

    #[test]
    fn workspace_runs_are_completed_pull_request_and_merge_group_runs_of_workspace_ci() {
        let run = workspace(1, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        let parsed = WorkspaceRun::from_run("acme/garden", &run).unwrap();
        assert_eq!(parsed.started_unix_ms - parsed.created_unix_ms, 30_000);
        assert_eq!(parsed.duration_ms(CRITICAL_PATH_SERIES), Some(540_000));
        assert_eq!(parsed.duration_ms(MERGE_QUEUE_SERIES), None);
        let group = WorkspaceRun::from_run("ACME/garden", &workspace(2, "merge_group", "success", "2026-10-20T10:00:00Z", 9)).unwrap();
        // A merge group is measured from its run's creation, when it entered the queue.
        assert_eq!(group.duration_ms(MERGE_QUEUE_SERIES), Some(570_000));
        assert_eq!(group.duration_ms(CRITICAL_PATH_SERIES), None);
        let failed = WorkspaceRun::from_run("acme/garden", &workspace(3, "merge_group", "failure", "2026-10-20T10:00:00Z", 9)).unwrap();
        assert_eq!(failed.duration_ms(MERGE_QUEUE_SERIES), None, "only a green group has a latency");
        let mut rerun = workspace(5, "pull_request", "success", "2026-10-20T10:00:00Z", 3);
        rerun["run_attempt"] = json!(2);
        let rerun = WorkspaceRun::from_run("acme/garden", &rerun).unwrap();
        assert_eq!(rerun.duration_ms(CRITICAL_PATH_SERIES), None, "a re-run's timestamps measure only the re-run");
        for (field, wrong) in [
            ("name", json!("Performance")), ("path", json!(".github/workflows/perf.yml")),
            ("event", json!("push")), ("event", json!("workflow_dispatch")),
            ("status", json!("in_progress")), ("conclusion", json!(null)),
            ("head_branch", json!("gh-readonly-queue/main/pr-1-abc")),
            ("run_started_at", json!("not a time")), ("updated_at", json!("2026-10-20T09:00:00Z")),
            ("repository", json!({"full_name": "another/repository"})),
        ] {
            let mut excluded = run.clone();
            excluded[field] = wrong;
            assert!(WorkspaceRun::from_run("acme/garden", &excluded).is_none(), "{field}");
        }
        let mut unqueued = workspace(4, "merge_group", "success", "2026-10-20T10:00:00Z", 9);
        unqueued["head_branch"] = json!("main");
        assert!(WorkspaceRun::from_run("acme/garden", &unqueued).is_none());
    }

    #[test]
    fn a_renamed_workflow_fails_the_read_and_one_repository_is_kept() {
        assert!(workspace_runs("acme/garden", &[]).unwrap().is_empty(), "an idle day reads as no runs");
        let mut renamed = workspace(1, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        renamed["name"] = json!("Workspace checks");
        let error = workspace_runs("acme/garden", &[renamed.clone()]).unwrap_err().to_string();
        assert!(error.contains("none named \"Workspace CI\""), "{error}");
        let kept = workspace(2, "pull_request", "success", "2026-10-20T10:00:00Z", 9);
        assert_eq!(workspace_runs("acme/garden", &[renamed, kept]).unwrap().len(), 1);
        assert!(reads_workspace_ci(None, "acme/garden"));
        assert!(reads_workspace_ci(Some("acme/garden"), "ACME/garden"));
        assert!(!reads_workspace_ci(Some("acme/garden"), "acme/orchard"));
    }

    #[test]
    fn series_take_the_runs_that_finished_inside_the_window() {
        let runs = [
            workspace(1, "pull_request", "success", "2026-10-20T09:00:00Z", 9),
            workspace(2, "pull_request", "success", "2026-10-21T08:00:00Z", 14),
            workspace(3, "pull_request", "failure", "2026-10-21T08:00:00Z", 2),
            workspace(4, "merge_group", "success", "2026-10-21T08:00:00Z", 7),
            workspace(5, "merge_group", "failure", "2026-10-21T09:10:00Z", 3),
            workspace(6, "merge_group", "failure", "2026-10-21T07:00:00Z", 3),
            workspace(7, "merge_group", "cancelled", "2026-10-21T09:20:00Z", 1),
        ];
        let ci = WorkspaceCi {
            repository: "acme/garden".to_owned(),
            observed_at_unix_ms: unix_ms("2026-10-21T09:30:00Z"),
            runs: runs.iter().filter_map(|run| WorkspaceRun::from_run("acme/garden", run)).collect(),
        };
        let now = unix_ms("2026-10-21T09:30:00Z");
        // Run 1 finished at 09:09:30 the day before, just outside the day.
        assert_eq!(ci.durations(CRITICAL_PATH_SERIES, now - DAY_MS, now), Some(vec![840_000]));
        assert_eq!(ci.durations(CRITICAL_PATH_SERIES, now - DAY_MS - 3_600_000, now), Some(vec![540_000, 840_000]));
        assert_eq!(ci.durations(MERGE_QUEUE_SERIES, now - DAY_MS, now), Some(vec![450_000]));
        assert_eq!(ci.durations(FLAKES_SERIES, now - DAY_MS, now), None);
        assert_eq!(ci.durations("ci/other", now - DAY_MS, now), None);
        // In the last hour: run 5 ejected, run 7 cancelled; run 4 finished at 08:07:30, outside.
        assert_eq!(ci.ejected_groups(FLAKES_SERIES, now - 3_600_000, now), Some((1, 2)));
        assert_eq!(ci.ejected_groups(FLAKES_SERIES, now - DAY_MS, now), Some((2, 4)));
        assert_eq!(ci.ejected_groups(MERGE_QUEUE_SERIES, now - 3_600_000, now), None);
        // A window with nothing in it holds no samples.
        let later = now + 2 * DAY_MS;
        assert_eq!(ci.durations(CRITICAL_PATH_SERIES, later - DAY_MS, later), Some(vec![]));
        assert_eq!(ci.ejected_groups(FLAKES_SERIES, later - 3_600_000, later), Some((0, 0)));
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
