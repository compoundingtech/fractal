/**
 * The normative wire examples (`docs/st3/client-v0/fixtures/github-plugin-*.json`), decoded through the plugin's
 * schemas. Stories and tests read these, so the Storybook views, the TS decoder and the Rust serializer share one
 * artifact. Synthetic data only (`acme/garden`).
 */
import { Schema } from 'effect'

import checks from '../../../../../docs/st3/client-v0/fixtures/github-plugin-checks.json' with { type: 'json' }
import comment from '../../../../../docs/st3/client-v0/fixtures/github-plugin-comment.json' with { type: 'json' }
import conversation from '../../../../../docs/st3/client-v0/fixtures/github-plugin-conversation.json' with { type: 'json' }
import files from '../../../../../docs/st3/client-v0/fixtures/github-plugin-files.json' with { type: 'json' }
import gone from '../../../../../docs/st3/client-v0/fixtures/github-plugin-gone.json' with { type: 'json' }
import issue from '../../../../../docs/st3/client-v0/fixtures/github-plugin-issue.json' with { type: 'json' }
import moved from '../../../../../docs/st3/client-v0/fixtures/github-plugin-moved.json' with { type: 'json' }
import notFound from '../../../../../docs/st3/client-v0/fixtures/github-plugin-not-found.json' with { type: 'json' }
import pullRequest from '../../../../../docs/st3/client-v0/fixtures/github-plugin-pull-request.json' with { type: 'json' }
import run from '../../../../../docs/st3/client-v0/fixtures/github-plugin-run.json' with { type: 'json' }
import stale from '../../../../../docs/st3/client-v0/fixtures/github-plugin-stale.json' with { type: 'json' }
import unavailable from '../../../../../docs/st3/client-v0/fixtures/github-plugin-unavailable.json' with { type: 'json' }
import { ChecksRead, CommentRead, ConversationRead, FilesRead, IssueRead, PullRequestRead, RunRead, type GithubChecks, type GithubComment, type GithubConversation, type GithubFiles, type GithubIssue, type GithubPullRequest, type GithubRead, type GithubRun } from './schema.ts'

const decode = <T>(schema: Schema.Codec<GithubRead<T>, unknown>, raw: unknown, name: string): GithubRead<T> => {
  try {
    return Schema.decodeUnknownSync(schema)(raw)
  } catch (cause) {
    throw new Error(`GitHub plugin fixture ${name} does not decode`, { cause })
  }
}

const found = <T>(read: GithubRead<T>): T => {
  if (read.status !== 'found') throw new Error('expected a found fixture')
  return read.item
}

export const fixtureNow = Date.parse('2026-11-07T06:45:00Z')

export const pullRequestRead = decode<GithubPullRequest>(PullRequestRead as never, pullRequest, 'pull-request')
export const stalePullRequestRead = decode<GithubPullRequest>(PullRequestRead as never, stale, 'stale')
export const movedRead = decode<GithubPullRequest>(PullRequestRead as never, moved, 'moved')
export const goneRead = decode<GithubPullRequest>(PullRequestRead as never, gone, 'gone')
export const notFoundRead = decode<GithubPullRequest>(PullRequestRead as never, notFound, 'not-found')
export const unavailableRead = decode<GithubPullRequest>(PullRequestRead as never, unavailable, 'unavailable')
export const issueRead = decode<GithubIssue>(IssueRead as never, issue, 'issue')
export const commentRead = decode<GithubComment>(CommentRead as never, comment, 'comment')
export const conversationRead = decode<GithubConversation>(ConversationRead as never, conversation, 'conversation')
export const checksRead = decode<GithubChecks>(ChecksRead as never, checks, 'checks')
export const filesRead = decode<GithubFiles>(FilesRead as never, files, 'files')
export const runRead = decode<GithubRun>(RunRead as never, run, 'run')

export const pullRequestFixture = found(pullRequestRead)
export const issueFixture = found(issueRead)
export const commentFixture = found(commentRead)
export const conversationFixture = found(conversationRead)
export const checksFixture = found(checksRead)
export const filesFixture = found(filesRead)
export const runFixture = found(runRead)

/** A draft PR with no reviews yet and every check green, for the card and header variants. */
export const draftPullRequestFixture: GithubPullRequest = {
  ...pullRequestFixture,
  number: 12,
  url: 'https://github.com/acme/garden/pull/12',
  title: 'Draft: compost rotation',
  body: '',
  draft: true,
  merge_state: 'draft',
  review_decision: null,
  labels: [],
  requested_reviewers: [],
  closing_issues: [],
}

export const mergedPullRequestFixture: GithubPullRequest = found(stalePullRequestRead)
