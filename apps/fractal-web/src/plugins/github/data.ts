/**
 * How the plugin's views get GitHub data: one `GithubReader` port, provided once by the host. The gateway
 * reader calls the daemon's `/v1/client/plugins/github/*` reads (typed path parts, never a URL), unwraps the
 * client-v0 envelope and decodes `value` through the plugin's schemas. Stories provide a fixture reader.
 *
 * The daemon owns GitHub caching (TTL + ETag, own cap). Here, one in-flight request per read is shared by
 * every card and tab asking for it, and a settled answer is reused for `reuseMs` so moving the pointer
 * across the same link does not ask again.
 */
import { Schema } from 'effect'
import * as React from 'react'

import type { GithubRef } from './refs.ts'
import { ChecksRead, CommentRead, ConversationRead, FilesRead, IssueRead, PullRequestRead, RunRead, type GithubChecks, type GithubComment, type GithubConversation, type GithubFiles, type GithubIssue, type GithubPullRequest, type GithubRead, type GithubRun } from './schema.ts'

export type GithubPart = 'item' | 'conversation' | 'checks' | 'files'

export interface GithubReadMap {
  readonly 'pull-request': GithubPullRequest
  readonly issue: GithubIssue
  readonly comment: GithubComment
  readonly run: GithubRun
}
export type ItemOf<R extends GithubRef> = GithubReadMap[R['kind']]

/** A read that failed before GitHub answered: the daemon refused, or the network did. */
export class GithubReadFailure extends Error {
  readonly _tag = 'GithubReadFailure'
}

/**
 * The daemon has no GitHub plugin route (404 on the route itself; GitHub's own 404 arrives inside the
 * envelope as `not_found`). An older daemon before rollout: the host shows plain links, not an error card.
 */
export class GithubReadUnsupported extends Error {
  readonly _tag = 'GithubReadUnsupported'
}

export interface GithubReader {
  readonly item: (ref: GithubRef, signal?: AbortSignal) => Promise<GithubRead<GithubPullRequest | GithubIssue | GithubComment | GithubRun>>
  readonly conversation: (ref: Extract<GithubRef, { kind: 'pull-request' }>, signal?: AbortSignal) => Promise<GithubRead<GithubConversation>>
  readonly checks: (ref: Extract<GithubRef, { kind: 'pull-request' }>, signal?: AbortSignal) => Promise<GithubRead<GithubChecks>>
  readonly files: (ref: Extract<GithubRef, { kind: 'pull-request' }>, signal?: AbortSignal) => Promise<GithubRead<GithubFiles>>
}

const pathOf = (ref: GithubRef, part: GithubPart): string => {
  const base = `/v1/client/plugins/github/${encodeURIComponent(ref.owner)}/${encodeURIComponent(ref.repo)}`
  switch (ref.kind) {
    case 'pull-request': return part === 'item' ? `${base}/pulls/${ref.number}` : `${base}/pulls/${ref.number}/${part}`
    case 'issue': return `${base}/issues/${ref.number}`
    case 'comment': return `${base}/comments/${ref.comment_id}`
    case 'run': return `${base}/runs/${ref.run_id}`
  }
}

const itemSchema = (ref: GithubRef) => {
  switch (ref.kind) {
    case 'pull-request': return PullRequestRead
    case 'issue': return IssueRead
    case 'comment': return CommentRead
    case 'run': return RunRead
  }
}

const Envelope = Schema.Struct({ value: Schema.Unknown })

/** The live reader over the daemon. `fetchImpl` is the host's authorized client fetch. */
export const gatewayGithubReader = ({ baseUrl, fetchImpl }: { readonly baseUrl: string; readonly fetchImpl: typeof fetch }): GithubReader => {
  const get = async <T>(path: string, schema: Schema.Top, signal: AbortSignal | undefined): Promise<GithubRead<T>> => {
    let response: Response
    try {
      response = await fetchImpl(new URL(path, baseUrl), { method: 'GET', headers: { accept: 'application/json' }, ...(signal === undefined ? {} : { signal }) })
    } catch (cause) {
      throw new GithubReadFailure('Fractal could not be reached', { cause })
    }
    if (response.status === 404) throw new GithubReadUnsupported('This Fractal daemon has no GitHub plugin reads')
    if (!response.ok) throw new GithubReadFailure(`Fractal refused the GitHub read (${response.status})`)
    const body: unknown = await response.json()
    const envelope = Schema.decodeUnknownSync(Envelope)(body)
    return Schema.decodeUnknownSync(schema as Schema.Codec<GithubRead<T>, unknown>)(envelope.value)
  }
  return {
    item: (ref, signal) => get(pathOf(ref, 'item'), itemSchema(ref), signal),
    conversation: (ref, signal) => get(pathOf(ref, 'conversation'), ConversationRead, signal),
    checks: (ref, signal) => get(pathOf(ref, 'checks'), ChecksRead, signal),
    files: (ref, signal) => get(pathOf(ref, 'files'), FilesRead, signal),
  }
}

/** Shares in-flight reads and reuses settled ones for `reuseMs`. */
export const sharedGithubReader = (reader: GithubReader, { reuseMs = 30_000, now = () => Date.now() }: { readonly reuseMs?: number; readonly now?: () => number } = {}): GithubReader => {
  const entries = new Map<string, { readonly at: number; readonly answer: Promise<GithubRead<never>> }>()
  const once = <T>(key: string, start: () => Promise<GithubRead<T>>): Promise<GithubRead<T>> => {
    const entry = entries.get(key)
    if (entry !== undefined && now() - entry.at < reuseMs) return entry.answer as Promise<GithubRead<T>>
    const answer = start()
    entries.set(key, { at: now(), answer: answer as Promise<GithubRead<never>> })
    // A failure is not reused: the next hover asks again. A daemon without the route stays without it.
    answer.catch((error: unknown) => { if (!(error instanceof GithubReadUnsupported) && entries.get(key)?.answer === answer) entries.delete(key) })
    return answer
  }
  return {
    item: ref => once(`item:${pathOf(ref, 'item')}`, () => reader.item(ref)),
    conversation: ref => once(`conversation:${pathOf(ref, 'item')}`, () => reader.conversation(ref)),
    checks: ref => once(`checks:${pathOf(ref, 'item')}`, () => reader.checks(ref)),
    files: ref => once(`files:${pathOf(ref, 'item')}`, () => reader.files(ref)),
  }
}

const ReaderContext = React.createContext<GithubReader | undefined>(undefined)
export const GithubReaderProvider = ReaderContext.Provider
export const useGithubReader = (): GithubReader | undefined => React.useContext(ReaderContext)

/** `undefined` while loading; a `GithubReadFailure` message when Fractal itself failed. */
export type Loaded<T> = { readonly _tag: 'Loading' } | { readonly _tag: 'Read'; readonly read: GithubRead<T> } | { readonly _tag: 'Failed'; readonly message: string } | { readonly _tag: 'Unsupported' }

/** Reads while `enabled`; a card enables it only once it opens, so a link never reads on render. */
export const useGithubRead = <T>(key: string | undefined, start: ((reader: GithubReader) => Promise<GithubRead<T>>) | undefined, enabled = true): Loaded<T> => {
  const reader = useGithubReader()
  const [state, setState] = React.useState<{ readonly key: string; readonly loaded: Loaded<T> } | undefined>(undefined)
  React.useEffect(() => {
    if (!enabled || key === undefined || start === undefined || reader === undefined) return
    let live = true
    start(reader).then(
      read => { if (live) setState({ key, loaded: { _tag: 'Read', read } }) },
      (error: unknown) => { if (live) setState({ key, loaded: error instanceof GithubReadUnsupported ? { _tag: 'Unsupported' } : { _tag: 'Failed', message: error instanceof Error ? error.message : String(error) } }) },
    )
    return () => { live = false }
    // `start` is derived from `key`; keying on it alone keeps an inline lambda from re-reading every render.
  }, [enabled, key, reader])
  if (reader === undefined) return { _tag: 'Unsupported' }
  return state !== undefined && state.key === key ? state.loaded : { _tag: 'Loading' }
}
