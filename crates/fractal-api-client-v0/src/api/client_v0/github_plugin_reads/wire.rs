//! The wire shapes of `apps/fractal-web/src/plugins/github/schema.ts`, field for field.
use serde::Serialize;

/// One read's answer, tagged by `status`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GithubRead<T> {
    Found { fetched_at: String, stale: bool, item: T },
    Moved { to: GithubMovedTo },
    Gone,
    NotFound,
    Unavailable { reason: String, retry_after_s: Option<u64> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubMovedTo {
    pub owner: String,
    pub repo: String,
    pub number: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubUser {
    pub login: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubLabel {
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubItemLink {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubPullRequest {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub state: &'static str,
    pub draft: bool,
    pub merge_state: Option<&'static str>,
    pub review_decision: Option<&'static str>,
    pub author: Option<GithubUser>,
    pub base_ref: String,
    pub head_ref: String,
    pub head_sha: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub commits: u64,
    pub comments: u64,
    pub created_at: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub labels: Vec<GithubLabel>,
    pub requested_reviewers: Vec<String>,
    pub closing_issues: Vec<GithubItemLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubIssue {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub state: &'static str,
    pub state_reason: Option<&'static str>,
    pub author: Option<GithubUser>,
    pub assignees: Vec<String>,
    pub labels: Vec<GithubLabel>,
    pub milestone: Option<String>,
    pub comments: u64,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub is_pull_request: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubReaction {
    pub content: &'static str,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubComment {
    pub owner: String,
    pub repo: String,
    pub id: u64,
    pub number: u64,
    pub url: String,
    pub author: Option<GithubUser>,
    pub author_association: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub reactions: Vec<GithubReaction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubReview {
    pub id: String,
    pub author: Option<GithubUser>,
    pub state: &'static str,
    pub body: String,
    pub submitted_at: Option<String>,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubThreadComment {
    pub id: String,
    pub author: Option<GithubUser>,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubReviewThread {
    pub id: String,
    pub path: String,
    pub line: Option<u64>,
    pub resolved: bool,
    pub outdated: bool,
    pub comments: Vec<GithubThreadComment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubConversation {
    pub reviews: Vec<GithubReview>,
    pub review_threads: Vec<GithubReviewThread>,
    pub comments: Vec<GithubComment>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubCheckRun {
    pub name: String,
    pub workflow: Option<String>,
    pub status: &'static str,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: String,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubChecks {
    pub head_sha: String,
    pub runs: Vec<GithubCheckRun>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: &'static str,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubFiles {
    pub total: u64,
    pub files: Vec<GithubFile>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubJob {
    pub name: String,
    pub status: &'static str,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GithubRun {
    pub owner: String,
    pub repo: String,
    pub id: u64,
    pub name: String,
    pub event: String,
    pub status: &'static str,
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub url: String,
    pub run_attempt: u64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub pull_requests: Vec<u64>,
    pub jobs: Vec<GithubJob>,
}
