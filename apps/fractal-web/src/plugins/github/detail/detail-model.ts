/**
 * Pure projections the detail tabs render: patch → diff lines, the Files link-out rule, one chronological
 * conversation, tab counts and the stale word. No React, no fetching; the views stay thin over these.
 */
import type { GithubRef } from '../refs.ts'
import { githubUrlOf } from '../refs.ts'
import type { GithubComment, GithubConversation, GithubFile, GithubFiles, GithubRead, GithubReview, GithubReviewThread } from '../schema.ts'

/** The kit DiffPanel's file shape, restated so this module stays free of the kit's runtime. */
export interface DetailDiffFile { readonly path: string; readonly diff: readonly string[]; readonly added: number; readonly removed: number }

/** GitHub lists at most this many files for a PR. */
export const githubFileListLimit = 3000

/** A unified patch → its lines. Only the final newline's empty tail is dropped; a lone `+` line is content. */
export const diffLinesOf = (patch: string): readonly string[] => {
  const lines = patch.split('\n')
  if (lines.at(-1) === '') lines.pop()
  return lines
}

export interface FilesView {
  /** Files with a patch, shaped for the DiffPanel. */
  readonly diffs: readonly DetailDiffFile[]
  /** Files GitHub sent without a patch (binary or too large): linked out. */
  readonly withoutPatch: readonly GithubFile[]
  /** True when the list itself is partial: the whole Files view links out for the rest. */
  readonly partial: boolean
  readonly total: number
}

export const filesView = (files: GithubFiles): FilesView => {
  const diffs: DetailDiffFile[] = []
  const withoutPatch: GithubFile[] = []
  for (const file of files.files) {
    if (file.patch === null) withoutPatch.push(file)
    else diffs.push({ path: file.path, diff: diffLinesOf(file.patch), added: file.additions, removed: file.deletions })
  }
  return { diffs, withoutPatch, partial: files.truncated || files.total > githubFileListLimit || files.total > files.files.length, total: files.total }
}

/** `github.com/O/R/pull/N/files`, built from typed parts. */
export const pullRequestFilesUrl = (ref: { readonly owner: string; readonly repo: string; readonly number: number }): string =>
  `${githubUrlOf({ kind: 'pull-request', owner: ref.owner, repo: ref.repo, number: ref.number })}/files`

/** `github.com/O/R/pull/N/checks`. */
export const pullRequestChecksUrl = (ref: { readonly owner: string; readonly repo: string; readonly number: number }): string =>
  `${githubUrlOf({ kind: 'pull-request', owner: ref.owner, repo: ref.repo, number: ref.number })}/checks`

export type TimelineEntry =
  | { readonly kind: 'review'; readonly at: string | null; readonly review: GithubReview }
  | { readonly kind: 'thread'; readonly at: string | null; readonly thread: GithubReviewThread }
  | { readonly kind: 'comment'; readonly at: string | null; readonly comment: GithubComment }

const instant = (at: string | null): number => {
  if (at === null) return Number.POSITIVE_INFINITY
  const parsed = Date.parse(at)
  return Number.isFinite(parsed) ? parsed : Number.POSITIVE_INFINITY
}

/** Reviews, review threads (by their first comment) and issue comments, oldest first; undated entries last, in input order. */
export const conversationTimeline = (conversation: Pick<GithubConversation, 'reviews' | 'review_threads' | 'comments'>): readonly TimelineEntry[] => {
  const entries: TimelineEntry[] = [
    ...conversation.reviews.map(review => ({ kind: 'review' as const, at: review.submitted_at, review })),
    ...conversation.review_threads.map(thread => ({ kind: 'thread' as const, at: thread.comments[0]?.created_at ?? null, thread })),
    ...conversation.comments.map(comment => ({ kind: 'comment' as const, at: comment.created_at, comment })),
  ]
  return entries.map((entry, index) => ({ entry, index })).sort((a, b) => instant(a.entry.at) - instant(b.entry.at) || a.index - b.index).map(({ entry }) => entry)
}

/** A sub-read's count once found, else the fallback the PR itself carries (or none while loading). */
export const countOf = <T>(read: GithubRead<T> | undefined, count: (item: T) => number, fallback?: number): number | undefined =>
  read?.status === 'found' ? count(read.item) : fallback

/** `Conversation 4`, or the bare word when the count is unknown. */
export const tabLabel = (word: string, count: number | undefined): string => count === undefined ? word : `${word} ${count}`

/** `just now`, `5m ago`, `2h ago`, `3d ago` between an instant and `now`. */
export const agoWord = (at: string, now: number): string => {
  const seconds = Math.max(0, Math.round((now - Date.parse(at)) / 1000))
  if (!Number.isFinite(seconds) || seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 48) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

/** What a found read says about its freshness: `stale · fetched 5m ago`, or nothing when fresh. */
export const staleWord = (read: GithubRead<unknown> | undefined, now: number): string | undefined =>
  read?.status === 'found' && read.stale ? `stale · fetched ${agoWord(read.fetched_at, now)}` : undefined

/** A label colour from GitHub (`a2eeef`) as CSS, or `undefined` for anything that is not six hex digits. */
export const labelColor = (color: string): string | undefined => /^[0-9a-fA-F]{6}$/.test(color) ? `#${color}` : undefined

/** The PR ref a GitHub pull-request item names, for problem links when only the target is known. */
export const pullRequestRefOf = (item: { readonly owner: string; readonly repo: string; readonly number: number }): GithubRef =>
  ({ kind: 'pull-request', owner: item.owner, repo: item.repo, number: item.number })
