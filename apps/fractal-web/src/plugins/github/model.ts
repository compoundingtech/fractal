/**
 * One decoded model per kind and its face: the small, view-independent projection every surface (row, hover
 * card, tab header) reads. Views stay thin over this, so adopting a shared "thing face" later is a mapping
 * change here, not a rewrite of the views.
 */
import type { GithubRef } from './refs.ts'
import { githubUrlOf, labelOf } from './refs.ts'
import type { GithubCheckRun, GithubComment, GithubIssue, GithubPullRequest, GithubRead, GithubRun, ReviewState } from './schema.ts'

/** The few inks a GitHub state uses; views map them to their own tokens. */
export type Tone = 'open' | 'draft' | 'merged' | 'closed' | 'success' | 'failure' | 'pending' | 'neutral'

export interface Face {
  readonly ref: GithubRef
  /** `acme/garden#7`. */
  readonly label: string
  readonly title: string
  /** The state word GitHub shows (`Open`, `Draft`, `Merged`, `Closed`, `Failed`). */
  readonly state: string
  readonly tone: Tone
  /** Login of the author; never an avatar. */
  readonly author: string | undefined
  /** Short facts, in reading order, for a card's meta line. */
  readonly facts: readonly string[]
  readonly url: string
  readonly updatedAt: string | undefined
}

export type GithubItem =
  | { readonly kind: 'pull-request'; readonly item: GithubPullRequest }
  | { readonly kind: 'issue'; readonly item: GithubIssue }
  | { readonly kind: 'comment'; readonly item: GithubComment }
  | { readonly kind: 'run'; readonly item: GithubRun }

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

export const pullRequestTone = (pr: Pick<GithubPullRequest, 'state' | 'draft'>): Tone =>
  pr.state === 'merged' ? 'merged' : pr.state === 'closed' ? 'closed' : pr.draft ? 'draft' : 'open'

export const pullRequestStateWord = (pr: Pick<GithubPullRequest, 'state' | 'draft'>): string =>
  pr.state === 'merged' ? 'Merged' : pr.state === 'closed' ? 'Closed' : pr.draft ? 'Draft' : 'Open'

const mergeStateWords: Readonly<Record<string, string>> = {
  clean: 'Ready to merge',
  dirty: 'Merge conflicts',
  blocked: 'Blocked',
  behind: 'Behind base',
  unstable: 'Checks failing',
  has_hooks: 'Ready, with hooks',
  draft: 'Draft',
  unknown: 'Merge state unknown',
}
export const mergeStateWord = (state: string | null): string | undefined => state === null ? undefined : mergeStateWords[state] ?? state

const reviewDecisionWords: Readonly<Record<string, string>> = {
  approved: 'Approved',
  changes_requested: 'Changes requested',
  review_required: 'Review required',
}
export const reviewDecisionWord = (decision: string | null): string | undefined => decision === null ? undefined : reviewDecisionWords[decision] ?? decision

const reviewWords: Readonly<Record<ReviewState, string>> = {
  approved: 'approved',
  changes_requested: 'requested changes',
  commented: 'commented',
  dismissed: 'review dismissed',
  pending: 'review pending',
}
export const reviewVerdict = (state: ReviewState): string => reviewWords[state]
export const reviewTone = (state: ReviewState): Tone => state === 'approved' ? 'success' : state === 'changes_requested' ? 'failure' : 'neutral'

/** A check or job: GitHub's words, success/failure/pending ink. */
export const checkTone = (run: Pick<GithubCheckRun, 'status' | 'conclusion'>): Tone => {
  if (run.status !== 'completed') return 'pending'
  switch (run.conclusion) {
    case 'success': return 'success'
    case 'failure':
    case 'timed_out':
    case 'startup_failure':
    case 'action_required': return 'failure'
    default: return 'neutral'
  }
}

export const checkWord = (run: Pick<GithubCheckRun, 'status' | 'conclusion'>): string => {
  if (run.status === 'in_progress') return 'In progress'
  if (run.status !== 'completed') return run.status === 'queued' ? 'Queued' : 'Pending'
  switch (run.conclusion) {
    case 'success': return 'Passed'
    case 'failure': return 'Failed'
    case 'timed_out': return 'Timed out'
    case 'cancelled': return 'Cancelled'
    case 'skipped': return 'Skipped'
    case 'neutral': return 'Neutral'
    case 'action_required': return 'Action required'
    case 'startup_failure': return 'Startup failure'
    case 'stale': return 'Stale'
    default: return run.conclusion ?? 'Completed'
  }
}

/** Seconds between two instants, or until `now` while still running; `undefined` before it starts. */
export const durationSeconds = (startedAt: string | null, completedAt: string | null, now: number): number | undefined => {
  if (startedAt === null) return undefined
  const end = completedAt === null ? now : Date.parse(completedAt)
  const seconds = Math.round((end - Date.parse(startedAt)) / 1000)
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : undefined
}

export const formatDurationShort = (seconds: number): string => {
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60), rest = seconds % 60
  if (minutes < 60) return rest === 0 ? `${minutes}m` : `${minutes}m ${rest}s`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ${minutes % 60}m`
}

/** Summary counts for a check list, as GitHub's merge box words them. */
export const checkCounts = (runs: readonly Pick<GithubCheckRun, 'status' | 'conclusion'>[]) => {
  const counts = { success: 0, failure: 0, pending: 0, neutral: 0 }
  for (const run of runs) {
    const tone = checkTone(run)
    if (tone === 'success' || tone === 'failure' || tone === 'pending') counts[tone]++
    else counts.neutral++
  }
  return counts
}

export const faceOf = (input: GithubItem): Face => {
  switch (input.kind) {
    case 'pull-request': {
      const pr = input.item
      const ref: GithubRef = { kind: 'pull-request', owner: pr.owner, repo: pr.repo, number: pr.number }
      const facts = [
        `${pr.head_ref} → ${pr.base_ref}`,
        `+${pr.additions} −${pr.deletions}`,
        plural(pr.changed_files, 'file'),
        reviewDecisionWord(pr.review_decision),
        pr.state === 'open' ? mergeStateWord(pr.merge_state) : undefined,
      ].filter((fact): fact is string => fact !== undefined)
      return { ref, label: labelOf(ref), title: pr.title, state: pullRequestStateWord(pr), tone: pullRequestTone(pr), author: pr.author?.login, facts, url: pr.url, updatedAt: pr.updated_at }
    }
    case 'issue': {
      const issue = input.item
      const ref: GithubRef = { kind: 'issue', owner: issue.owner, repo: issue.repo, number: issue.number }
      const closedWord = issue.state_reason === 'not_planned' ? 'Not planned' : 'Closed'
      const facts = [
        issue.assignees.length === 0 ? undefined : `Assigned ${issue.assignees.join(', ')}`,
        issue.milestone ?? undefined,
        plural(issue.comments, 'comment'),
      ].filter((fact): fact is string => fact !== undefined)
      return { ref, label: labelOf(ref), title: issue.title, state: issue.state === 'open' ? 'Open' : closedWord, tone: issue.state === 'open' ? 'open' : issue.state_reason === 'not_planned' ? 'neutral' : 'merged', author: issue.author?.login, facts, url: issue.url, updatedAt: issue.updated_at }
    }
    case 'comment': {
      const comment = input.item
      const ref: GithubRef = { kind: 'comment', owner: comment.owner, repo: comment.repo, comment_id: comment.id, number: comment.number }
      const reactions = comment.reactions.reduce((sum, reaction) => sum + reaction.count, 0)
      const facts = [comment.author_association === 'NONE' ? undefined : comment.author_association.toLowerCase(), reactions === 0 ? undefined : plural(reactions, 'reaction')].filter((fact): fact is string => fact !== undefined)
      const firstLine = comment.body.split('\n').find(line => line.trim() !== '')?.trim() ?? 'Comment'
      return { ref, label: labelOf(ref), title: firstLine, state: 'Comment', tone: 'neutral', author: comment.author?.login, facts, url: comment.url, updatedAt: comment.updated_at }
    }
    case 'run': {
      const run = input.item
      const ref: GithubRef = { kind: 'run', owner: run.owner, repo: run.repo, run_id: run.id }
      const facts = [run.head_branch ?? undefined, run.event, run.run_attempt > 1 ? `attempt ${run.run_attempt}` : undefined, plural(run.jobs.length, 'job')].filter((fact): fact is string => fact !== undefined)
      return { ref, label: labelOf(ref), title: run.name, state: checkWord(run), tone: checkTone(run), author: undefined, facts, url: run.url, updatedAt: run.completed_at ?? run.started_at ?? undefined }
    }
  }
}

/** What a card or tab says for a read that did not find the item. `undefined` when it did. */
export const readProblem = (read: GithubRead<unknown>, ref: GithubRef): { readonly title: string; readonly detail: string; readonly link: string } | undefined => {
  const link = githubUrlOf(ref)
  switch (read.status) {
    case 'found': return undefined
    case 'moved': return { title: 'Moved', detail: `Now at ${read.to.owner}/${read.to.repo}${read.to.number === null ? '' : `#${read.to.number}`}.`, link: githubUrlOf(read.to.number === null ? { ...ref, owner: read.to.owner, repo: read.to.repo } : { kind: ref.kind === 'issue' ? 'issue' : 'pull-request', owner: read.to.owner, repo: read.to.repo, number: read.to.number }) }
    case 'gone': return { title: 'Deleted', detail: 'GitHub says this was removed.', link }
    case 'not_found': return { title: 'Not found or no access', detail: 'GitHub has no such item, or Fractal\'s GitHub credential cannot see it.', link }
    case 'unavailable': return { title: 'GitHub unavailable', detail: read.retry_after_s === null ? read.reason : `${read.reason} Retry in ${formatDurationShort(read.retry_after_s)}.`, link }
  }
}

/** Two letters for a login, the card's stand-in for an avatar. */
export const initials = (login: string): string => {
  const parts = login.split(/[-_.\s]+/).filter(part => part !== '')
  const letters = parts.length >= 2 ? `${parts[0]![0]}${parts[1]![0]}` : login.slice(0, 2)
  return letters.toUpperCase()
}
