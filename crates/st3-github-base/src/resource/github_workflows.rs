//! The narrow main Performance failure field of the existing repository observer: a failed
//! `perf.yml` push run on main, or a daily long-lanes run on main whose `daily-perf-` lane failed.
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
