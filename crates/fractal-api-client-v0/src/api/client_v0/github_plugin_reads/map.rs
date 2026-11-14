//! GitHub's REST and GraphQL documents as the plugin's wire shapes. Pure: no I/O, no clock.
use super::wire::*;
use super::{Failure, LIST_BOUND, Repo};
use fractal_api_core::api::client_v0::assistant::reads::encoded_len;
use serde_json::Value;

pub(crate) const PULL_QUERY: &str = r#"query FractalPluginPull($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      mergeStateStatus
      reviewDecision
      closingIssuesReferences(first: 25) {
        nodes { number title state repository { name owner { login } } }
      }
    }
  }
}"#;

pub(crate) const CONVERSATION_QUERY: &str = r#"query FractalPluginConversation($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      latestOpinionatedReviews(first: 100) {
        totalCount
        nodes { id state body submittedAt url author { login url } }
      }
      reviewThreads(first: 100) {
        totalCount
        nodes {
          id path line isResolved isOutdated
          comments(first: 100) {
            totalCount
            nodes { id body createdAt url author { login url } }
          }
        }
      }
    }
  }
}"#;

pub(crate) const CHECKS_QUERY: &str = r#"query FractalPluginChecks($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      headRefOid
      commits(last: 1) {
        nodes {
          commit {
            oid
            statusCheckRollup {
              contexts(first: 100) {
                totalCount
                nodes {
                  __typename
                  ... on CheckRun {
                    name status conclusion startedAt completedAt detailsUrl url summary
                    checkSuite { app { name } workflowRun { workflow { name } } }
                  }
                  ... on StatusContext { context state targetUrl description createdAt }
                }
              }
            }
          }
        }
      }
    }
  }
}"#;

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn maybe(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn count(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn nodes<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    items(&value[key]["nodes"])
}

/// More exist than were listed.
fn more(value: &Value, key: &str) -> bool {
    count(&value[key], "totalCount") > nodes(value, key).len() as u64
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value, Failure> {
    value
        .get(key)
        .filter(|value| !value.is_null())
        .ok_or_else(|| Failure::unavailable(format!("GitHub's answer has no {key}")))
}

fn required_text(value: &Value, key: &str) -> Result<String, Failure> {
    required(value, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Failure::unavailable(format!("GitHub's {key} is not text")))
}

/// A REST user: its profile page, never its API URL or avatar.
fn rest_user(value: &Value) -> Option<GithubUser> {
    let login = value.get("login")?.as_str()?;
    Some(GithubUser { login: login.to_owned(), url: maybe(value, "html_url").unwrap_or_else(|| format!("https://github.com/{login}")) })
}

/// A GraphQL actor: `url` is already its profile page.
fn graph_user(value: &Value) -> Option<GithubUser> {
    let login = value.get("login")?.as_str()?;
    Some(GithubUser { login: login.to_owned(), url: maybe(value, "url").unwrap_or_else(|| format!("https://github.com/{login}")) })
}

fn labels(value: &Value) -> Vec<GithubLabel> {
    items(&value["labels"]).iter().map(|label| GithubLabel { name: text(label, "name"), color: text(label, "color") }).collect()
}

fn logins(value: &Value) -> Vec<String> {
    items(value).iter().filter_map(|user| maybe(user, "login")).collect()
}

/// One of `allowed`, matched lower-cased; GitHub's other values become `None`.
fn known(value: Option<&str>, allowed: &[&'static str]) -> Option<&'static str> {
    let value = value?.to_ascii_lowercase();
    allowed.iter().copied().find(|known| *known == value)
}

fn lower(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_ascii_lowercase)
}

const CHECK_STATUSES: &[&str] = &["queued", "in_progress", "completed", "waiting", "requested", "pending"];

fn check_status(value: Option<&str>) -> &'static str {
    known(value, CHECK_STATUSES).unwrap_or("pending")
}

/// The GraphQL `pullRequest`, or `NotFound` when GitHub has none.
fn graph_pull(data: &Value) -> Result<&Value, Failure> {
    let pull = &data["data"]["repository"]["pullRequest"];
    if pull.is_object() { Ok(pull) } else { Err(Failure::NotFound) }
}

pub(crate) fn pull(repo: &Repo, number: u64, rest: &Value, graph: &Value) -> Result<GithubPullRequest, Failure> {
    let graph = graph_pull(graph)?;
    let merged_at = maybe(rest, "merged_at");
    let state = if merged_at.is_some() { "merged" } else if text(rest, "state") == "closed" { "closed" } else { "open" };
    let mut requested_reviewers = logins(&rest["requested_reviewers"]);
    requested_reviewers.extend(items(&rest["requested_teams"]).iter().filter_map(|team| maybe(team, "slug")).map(|slug| format!("{}/{slug}", repo.owner)));
    Ok(GithubPullRequest {
        owner: repo.owner.clone(),
        repo: repo.repo.clone(),
        number,
        url: required_text(rest, "html_url")?,
        title: text(rest, "title"),
        body: text(rest, "body"),
        state,
        draft: rest.get("draft").and_then(Value::as_bool).unwrap_or(false),
        merge_state: known(
            graph.get("mergeStateStatus").and_then(Value::as_str),
            &["clean", "dirty", "blocked", "behind", "unstable", "has_hooks", "draft", "unknown"],
        ),
        review_decision: known(graph.get("reviewDecision").and_then(Value::as_str), &["approved", "changes_requested", "review_required"]),
        author: rest_user(&rest["user"]),
        base_ref: text(&rest["base"], "ref"),
        head_ref: text(&rest["head"], "ref"),
        head_sha: text(&rest["head"], "sha"),
        additions: count(rest, "additions"),
        deletions: count(rest, "deletions"),
        changed_files: count(rest, "changed_files"),
        commits: count(rest, "commits"),
        comments: count(rest, "comments"),
        created_at: text(rest, "created_at"),
        updated_at: text(rest, "updated_at"),
        merged_at,
        closed_at: maybe(rest, "closed_at"),
        labels: labels(rest),
        requested_reviewers,
        closing_issues: nodes(graph, "closingIssuesReferences")
            .iter()
            .filter_map(|issue| {
                Some(GithubItemLink {
                    owner: maybe(&issue["repository"]["owner"], "login")?,
                    repo: maybe(&issue["repository"], "name")?,
                    number: issue.get("number")?.as_u64()?,
                    title: text(issue, "title"),
                    state: lower(issue, "state").unwrap_or_default(),
                })
            })
            .collect(),
    })
}

pub(crate) fn issue(repo: &Repo, number: u64, rest: &Value) -> Result<GithubIssue, Failure> {
    Ok(GithubIssue {
        owner: repo.owner.clone(),
        repo: repo.repo.clone(),
        number,
        url: required_text(rest, "html_url")?,
        title: text(rest, "title"),
        body: text(rest, "body"),
        state: if text(rest, "state") == "closed" { "closed" } else { "open" },
        state_reason: known(rest.get("state_reason").and_then(Value::as_str), &["completed", "not_planned", "reopened"]),
        author: rest_user(&rest["user"]),
        assignees: logins(&rest["assignees"]),
        labels: labels(rest),
        milestone: maybe(&rest["milestone"], "title"),
        comments: count(rest, "comments"),
        created_at: text(rest, "created_at"),
        updated_at: text(rest, "updated_at"),
        closed_at: maybe(rest, "closed_at"),
        is_pull_request: rest.get("pull_request").is_some(),
    })
}

const REACTIONS: &[&str] = &["+1", "-1", "laugh", "hooray", "confused", "heart", "rocket", "eyes"];

pub(crate) fn comment(repo: &Repo, rest: &Value) -> Result<GithubComment, Failure> {
    let number = maybe(rest, "issue_url")
        .and_then(|url| url.rsplit('/').next().and_then(|n| super::parse_id(n, i32::MAX as u64)))
        .ok_or_else(|| Failure::unavailable("GitHub's comment names no issue"))?;
    Ok(GithubComment {
        owner: repo.owner.clone(),
        repo: repo.repo.clone(),
        id: required(rest, "id")?.as_u64().ok_or_else(|| Failure::unavailable("GitHub's comment id is not a number"))?,
        number,
        url: required_text(rest, "html_url")?,
        author: rest_user(&rest["user"]),
        author_association: text(rest, "author_association"),
        body: text(rest, "body"),
        created_at: text(rest, "created_at"),
        updated_at: text(rest, "updated_at"),
        reactions: REACTIONS
            .iter()
            .filter_map(|&content| {
                let count = rest["reactions"].get(content)?.as_u64().filter(|count| *count > 0)?;
                Some(GithubReaction { content, count })
            })
            .collect(),
    })
}

pub(crate) fn conversation(repo: &Repo, graph: &Value, comments: &Value, more_comments: bool) -> Result<GithubConversation, Failure> {
    let pull = graph_pull(graph)?;
    let mut reviews = nodes(pull, "latestOpinionatedReviews")
        .iter()
        .map(|review| GithubReview {
            id: text(review, "id"),
            author: graph_user(&review["author"]),
            state: known(review.get("state").and_then(Value::as_str), &["approved", "changes_requested", "commented", "dismissed", "pending"])
                .unwrap_or("commented"),
            body: text(review, "body"),
            submitted_at: maybe(review, "submittedAt"),
            url: text(review, "url"),
        })
        .collect::<Vec<_>>();
    reviews.sort_by(|a, b| a.submitted_at.cmp(&b.submitted_at));
    let mut truncated = more(pull, "latestOpinionatedReviews") || more(pull, "reviewThreads") || more_comments;
    let review_threads = nodes(pull, "reviewThreads")
        .iter()
        .map(|thread| {
            truncated |= more(thread, "comments");
            GithubReviewThread {
                id: text(thread, "id"),
                path: text(thread, "path"),
                line: thread.get("line").and_then(Value::as_u64),
                resolved: thread.get("isResolved").and_then(Value::as_bool).unwrap_or(false),
                outdated: thread.get("isOutdated").and_then(Value::as_bool).unwrap_or(false),
                comments: nodes(thread, "comments")
                    .iter()
                    .map(|comment| GithubThreadComment {
                        id: text(comment, "id"),
                        author: graph_user(&comment["author"]),
                        body: text(comment, "body"),
                        created_at: text(comment, "createdAt"),
                        url: text(comment, "url"),
                    })
                    .collect(),
            }
        })
        .collect();
    let listed = items(comments);
    truncated |= listed.len() > LIST_BOUND;
    Ok(GithubConversation {
        reviews,
        review_threads,
        comments: listed.iter().take(LIST_BOUND).map(|value| comment(repo, value)).collect::<Result<_, _>>()?,
        truncated,
    })
}

pub(crate) fn checks(graph: &Value) -> Result<GithubChecks, Failure> {
    let pull = graph_pull(graph)?;
    let commit = items(&pull["commits"]["nodes"]).first().map(|node| &node["commit"]);
    let head_sha = commit.and_then(|commit| maybe(commit, "oid")).unwrap_or_else(|| text(pull, "headRefOid"));
    let contexts = commit.map(|commit| &commit["statusCheckRollup"]).filter(|rollup| rollup.is_object());
    let runs = contexts
        .map(|rollup| nodes(rollup, "contexts"))
        .unwrap_or_default()
        .iter()
        .map(|context| {
            if context["__typename"] == "StatusContext" {
                let state = lower(context, "state");
                let (status, conclusion) = match state.as_deref() {
                    Some("success") => ("completed", Some("success".to_owned())),
                    Some("failure" | "error") => ("completed", Some("failure".to_owned())),
                    _ => ("pending", None),
                };
                return GithubCheckRun {
                    name: text(context, "context"),
                    workflow: None,
                    status,
                    conclusion,
                    started_at: maybe(context, "createdAt"),
                    completed_at: None,
                    url: text(context, "targetUrl"),
                    summary: maybe(context, "description").filter(|text| !text.is_empty()),
                };
            }
            let suite = &context["checkSuite"];
            GithubCheckRun {
                name: text(context, "name"),
                workflow: maybe(&suite["workflowRun"]["workflow"], "name").or_else(|| maybe(&suite["app"], "name")),
                status: check_status(context.get("status").and_then(Value::as_str)),
                conclusion: lower(context, "conclusion"),
                started_at: maybe(context, "startedAt"),
                completed_at: maybe(context, "completedAt"),
                url: maybe(context, "detailsUrl").filter(|url| !url.is_empty()).unwrap_or_else(|| text(context, "url")),
                summary: maybe(context, "summary").filter(|text| !text.is_empty()),
            }
        })
        .collect();
    Ok(GithubChecks { head_sha, runs, truncated: contexts.is_some_and(|rollup| more(rollup, "contexts")) })
}

pub(crate) fn files(total: u64, pages: &[std::sync::Arc<Value>], cut_short: bool) -> Result<GithubFiles, Failure> {
    // Match the existing bounded client reads: reserve 128,000 bytes for the read wrapper and Envelope.
    let budget = super::CLIENT_MAX_RESPONSE_BYTES - 128_000;
    let available = pages.iter().map(|page| items(page).len()).sum::<usize>();
    let total = total.max(available as u64);
    let mut result = GithubFiles { total, files: Vec::new(), truncated: cut_short };
    let mut bytes = json_bytes(&result)?;
    for source in pages.iter().flat_map(|page| items(page).iter()).take(super::MAX_FILES) {
        let mut file = GithubFile {
            path: text(source, "filename"),
            previous_path: maybe(source, "previous_filename"),
            status: known(
                source.get("status").and_then(Value::as_str),
                &["added", "removed", "modified", "renamed", "copied", "changed", "unchanged"],
            ).unwrap_or("changed"),
            additions: count(source, "additions"),
            deletions: count(source, "deletions"),
            patch: None,
        };
        let metadata = json_bytes(&file)?;
        let comma = usize::from(!result.files.is_empty());
        if bytes + metadata + comma > budget {
            result.truncated = true;
            break;
        }
        let mut size = metadata;
        if let Some(patch) = source.get("patch").and_then(Value::as_str) {
            // Replace the serialized null with the escaped string, not its raw UTF-8 length.
            let with_patch = metadata - 4 + encoded_len(patch) + 2;
            if bytes + with_patch + comma <= budget {
                file.patch = Some(patch.to_owned());
                size = with_patch;
            } else {
                result.truncated = true;
            }
        }
        bytes += size + comma;
        result.files.push(file);
    }
    result.truncated |= (result.files.len() as u64) < total;
    Ok(result)
}

fn json_bytes(value: &impl serde::Serialize) -> Result<usize, Failure> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| Failure::unavailable("GitHub files could not be encoded"))?;
    Ok(counter.0)
}

pub(crate) fn run(repo: &Repo, run: &Value, jobs: &Value) -> Result<GithubRun, Failure> {
    let status = check_status(run.get("status").and_then(Value::as_str));
    Ok(GithubRun {
        owner: repo.owner.clone(),
        repo: repo.repo.clone(),
        id: required(run, "id")?.as_u64().ok_or_else(|| Failure::unavailable("GitHub's run id is not a number"))?,
        name: maybe(run, "name").or_else(|| maybe(run, "display_title")).unwrap_or_default(),
        event: text(run, "event"),
        status,
        conclusion: lower(run, "conclusion"),
        head_branch: maybe(run, "head_branch"),
        head_sha: text(run, "head_sha"),
        url: required_text(run, "html_url")?,
        run_attempt: run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1),
        started_at: maybe(run, "run_started_at"),
        // REST names no end for a run; a completed run last changed when it ended.
        completed_at: if status == "completed" { maybe(run, "updated_at") } else { None },
        pull_requests: items(&run["pull_requests"]).iter().filter_map(|pull| pull.get("number")?.as_u64()).collect(),
        jobs: items(&jobs["jobs"])
            .iter()
            .map(|job| GithubJob {
                name: text(job, "name"),
                status: check_status(job.get("status").and_then(Value::as_str)),
                conclusion: lower(job, "conclusion"),
                started_at: maybe(job, "started_at"),
                completed_at: maybe(job, "completed_at"),
                url: text(job, "html_url"),
            })
            .collect(),
    })
}
