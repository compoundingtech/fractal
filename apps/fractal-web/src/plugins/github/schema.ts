/**
 * Wire shapes of the daemon's GitHub plugin reads, `GET /v1/client/plugins/github/*` (scope `read.projections`).
 * The daemon fetches GitHub through `fractal-github-base`'s fixed-host, no-redirect, bounded client; the browser
 * never talks to GitHub and never sends a URL, only the typed `{owner, repo, number | id}` in the path.
 *
 * The normative examples are `docs/st3/client-v0/fixtures/github-plugin-*.json`; `schema.unit.test.ts` decodes
 * every one of them through these schemas, and the Rust contract test serializes to the same files.
 *
 * Avatars and body images are not fetched (`img-src` is same-origin): people are logins, images are links.
 */
import { Schema } from 'effect'

const nullable = Schema.NullOr

/** RFC 3339 instant, as GitHub and the daemon write it. */
const Instant = Schema.String

export const GithubUser = Schema.Struct({ login: Schema.String, url: Schema.String }).annotate({ identifier: 'GithubUser' })
export type GithubUser = typeof GithubUser.Type

export const GithubLabel = Schema.Struct({ name: Schema.String, color: Schema.String }).annotate({ identifier: 'GithubLabel' })
export type GithubLabel = typeof GithubLabel.Type

/** A PR or issue another item links to, such as a closing issue. */
export const GithubItemLink = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  number: Schema.Int,
  title: Schema.String,
  state: Schema.String,
}).annotate({ identifier: 'GithubItemLink' })
export type GithubItemLink = typeof GithubItemLink.Type

export const PullRequestState = Schema.Literals(['open', 'closed', 'merged'])
export type PullRequestState = typeof PullRequestState.Type

/** GitHub's `mergeStateStatus`, lower-cased. `null` while GitHub is still computing it. */
export const MergeState = Schema.Literals(['clean', 'dirty', 'blocked', 'behind', 'unstable', 'has_hooks', 'draft', 'unknown'])
export type MergeState = typeof MergeState.Type

export const ReviewDecision = Schema.Literals(['approved', 'changes_requested', 'review_required'])
export type ReviewDecision = typeof ReviewDecision.Type

export const GithubPullRequest = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  number: Schema.Int,
  url: Schema.String,
  title: Schema.String,
  /** Markdown source; empty when the author wrote none. */
  body: Schema.String,
  state: PullRequestState,
  draft: Schema.Boolean,
  merge_state: nullable(MergeState),
  review_decision: nullable(ReviewDecision),
  author: nullable(GithubUser),
  base_ref: Schema.String,
  head_ref: Schema.String,
  head_sha: Schema.String,
  additions: Schema.Int,
  deletions: Schema.Int,
  changed_files: Schema.Int,
  commits: Schema.Int,
  comments: Schema.Int,
  created_at: Instant,
  updated_at: Instant,
  merged_at: nullable(Instant),
  closed_at: nullable(Instant),
  labels: Schema.Array(GithubLabel),
  requested_reviewers: Schema.Array(Schema.String),
  closing_issues: Schema.Array(GithubItemLink),
}).annotate({ identifier: 'GithubPullRequest' })
export type GithubPullRequest = typeof GithubPullRequest.Type

export const IssueState = Schema.Literals(['open', 'closed'])
export const IssueStateReason = Schema.Literals(['completed', 'not_planned', 'reopened'])

export const GithubIssue = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  number: Schema.Int,
  url: Schema.String,
  title: Schema.String,
  body: Schema.String,
  state: IssueState,
  state_reason: nullable(IssueStateReason),
  author: nullable(GithubUser),
  assignees: Schema.Array(Schema.String),
  labels: Schema.Array(GithubLabel),
  milestone: nullable(Schema.String),
  comments: Schema.Int,
  created_at: Instant,
  updated_at: Instant,
  closed_at: nullable(Instant),
  /** GitHub's issues API also answers for PRs; the daemon marks them so the client can open the PR kind. */
  is_pull_request: Schema.Boolean,
}).annotate({ identifier: 'GithubIssue' })
export type GithubIssue = typeof GithubIssue.Type

export const GithubReaction = Schema.Struct({ content: Schema.String, count: Schema.Int }).annotate({ identifier: 'GithubReaction' })

export const GithubComment = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  id: Schema.Int,
  /** The issue or PR the comment is on. */
  number: Schema.Int,
  url: Schema.String,
  author: nullable(GithubUser),
  author_association: Schema.String,
  body: Schema.String,
  created_at: Instant,
  updated_at: Instant,
  reactions: Schema.Array(GithubReaction),
}).annotate({ identifier: 'GithubComment' })
export type GithubComment = typeof GithubComment.Type

export const ReviewState = Schema.Literals(['approved', 'changes_requested', 'commented', 'dismissed', 'pending'])
export type ReviewState = typeof ReviewState.Type

export const GithubReview = Schema.Struct({
  id: Schema.String,
  author: nullable(GithubUser),
  state: ReviewState,
  body: Schema.String,
  submitted_at: nullable(Instant),
  url: Schema.String,
}).annotate({ identifier: 'GithubReview' })
export type GithubReview = typeof GithubReview.Type

export const GithubThreadComment = Schema.Struct({
  id: Schema.String,
  author: nullable(GithubUser),
  body: Schema.String,
  created_at: Instant,
  url: Schema.String,
}).annotate({ identifier: 'GithubThreadComment' })
export type GithubThreadComment = typeof GithubThreadComment.Type

export const GithubReviewThread = Schema.Struct({
  id: Schema.String,
  path: Schema.String,
  line: nullable(Schema.Int),
  resolved: Schema.Boolean,
  outdated: Schema.Boolean,
  comments: Schema.Array(GithubThreadComment),
}).annotate({ identifier: 'GithubReviewThread' })
export type GithubReviewThread = typeof GithubReviewThread.Type

/** A PR's conversation: latest opinionated reviews, review threads and issue comments, oldest first. */
export const GithubConversation = Schema.Struct({
  reviews: Schema.Array(GithubReview),
  review_threads: Schema.Array(GithubReviewThread),
  comments: Schema.Array(GithubComment),
  /** True when GitHub had more than the daemon's bound (100 of each); the tab links out for the rest. */
  truncated: Schema.Boolean,
}).annotate({ identifier: 'GithubConversation' })
export type GithubConversation = typeof GithubConversation.Type

export const CheckStatus = Schema.Literals(['queued', 'in_progress', 'completed', 'waiting', 'requested', 'pending'])
export type CheckStatus = typeof CheckStatus.Type

export const GithubCheckRun = Schema.Struct({
  name: Schema.String,
  /** The workflow (Actions) or app that produced the check, when GitHub names one. */
  workflow: nullable(Schema.String),
  status: CheckStatus,
  /** Lower-cased GitHub conclusion: success, failure, neutral, cancelled, skipped, timed_out, action_required, stale, startup_failure. */
  conclusion: nullable(Schema.String),
  started_at: nullable(Instant),
  completed_at: nullable(Instant),
  /** Permalink to the run on GitHub; logs are read there, never through Fractal. */
  url: Schema.String,
  summary: nullable(Schema.String),
}).annotate({ identifier: 'GithubCheckRun' })
export type GithubCheckRun = typeof GithubCheckRun.Type

export const GithubChecks = Schema.Struct({
  head_sha: Schema.String,
  runs: Schema.Array(GithubCheckRun),
  truncated: Schema.Boolean,
}).annotate({ identifier: 'GithubChecks' })
export type GithubChecks = typeof GithubChecks.Type

export const FileStatus = Schema.Literals(['added', 'removed', 'modified', 'renamed', 'copied', 'changed', 'unchanged'])

export const GithubFile = Schema.Struct({
  path: Schema.String,
  previous_path: nullable(Schema.String),
  status: FileStatus,
  additions: Schema.Int,
  deletions: Schema.Int,
  /** Unified diff hunks. `null` when GitHub omits it (binary or too large): the tab links out. */
  patch: nullable(Schema.String),
}).annotate({ identifier: 'GithubFile' })
export type GithubFile = typeof GithubFile.Type

/** GitHub lists at most 3,000 files for a PR; past that, or past the daemon's byte bound, `truncated` is set. */
export const GithubFiles = Schema.Struct({
  total: Schema.Int,
  files: Schema.Array(GithubFile),
  truncated: Schema.Boolean,
}).annotate({ identifier: 'GithubFiles' })
export type GithubFiles = typeof GithubFiles.Type

export const GithubJob = Schema.Struct({
  name: Schema.String,
  status: CheckStatus,
  conclusion: nullable(Schema.String),
  started_at: nullable(Instant),
  completed_at: nullable(Instant),
  url: Schema.String,
}).annotate({ identifier: 'GithubJob' })
export type GithubJob = typeof GithubJob.Type

export const GithubRun = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  id: Schema.Int,
  name: Schema.String,
  event: Schema.String,
  status: CheckStatus,
  conclusion: nullable(Schema.String),
  head_branch: nullable(Schema.String),
  head_sha: Schema.String,
  url: Schema.String,
  run_attempt: Schema.Int,
  started_at: nullable(Instant),
  completed_at: nullable(Instant),
  pull_requests: Schema.Array(Schema.Int),
  jobs: Schema.Array(GithubJob),
}).annotate({ identifier: 'GithubRun' })
export type GithubRun = typeof GithubRun.Type

/** Where a 301 from GitHub points, parsed back to a typed item; the daemon never follows it. */
export const GithubMovedTo = Schema.Struct({
  owner: Schema.String,
  repo: Schema.String,
  number: nullable(Schema.Int),
}).annotate({ identifier: 'GithubMovedTo' })
export type GithubMovedTo = typeof GithubMovedTo.Type

/**
 * One read's answer. `not_found` is GitHub's 404, which also means "no access" for a private repository.
 * `unavailable` is an upstream failure (rate limit, 5xx, over the byte bound) with GitHub's words.
 * `stale` is set when the daemon answers from its cache after a failed revalidation.
 */
export const githubRead = <S extends Schema.Top>(item: S) =>
  Schema.Union([
    Schema.Struct({ status: Schema.Literal('found'), fetched_at: Instant, stale: Schema.Boolean, item }),
    Schema.Struct({ status: Schema.Literal('moved'), to: GithubMovedTo }),
    Schema.Struct({ status: Schema.Literal('gone') }),
    Schema.Struct({ status: Schema.Literal('not_found') }),
    Schema.Struct({ status: Schema.Literal('unavailable'), reason: Schema.String, retry_after_s: nullable(Schema.Int) }),
  ])

export const PullRequestRead = githubRead(GithubPullRequest).annotate({ identifier: 'GithubPullRequestRead' })
export const IssueRead = githubRead(GithubIssue).annotate({ identifier: 'GithubIssueRead' })
export const CommentRead = githubRead(GithubComment).annotate({ identifier: 'GithubCommentRead' })
export const ConversationRead = githubRead(GithubConversation).annotate({ identifier: 'GithubConversationRead' })
export const ChecksRead = githubRead(GithubChecks).annotate({ identifier: 'GithubChecksRead' })
export const FilesRead = githubRead(GithubFiles).annotate({ identifier: 'GithubFilesRead' })
export const RunRead = githubRead(GithubRun).annotate({ identifier: 'GithubRunRead' })

export type GithubRead<T> =
  | { readonly status: 'found'; readonly fetched_at: string; readonly stale: boolean; readonly item: T }
  | { readonly status: 'moved'; readonly to: GithubMovedTo }
  | { readonly status: 'gone' }
  | { readonly status: 'not_found' }
  | { readonly status: 'unavailable'; readonly reason: string; readonly retry_after_s: number | null }
