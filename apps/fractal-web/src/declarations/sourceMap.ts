import type { SourceMap, SourceRevision, TextState } from '../../../../packages/fractal-ui/src/assistant-ui/declaration/SubjectInspector.tsx'
import { credentialExpiredMessage, type Feed } from '../data/source.ts'
import type { SourceMapLookup } from '../data/subjectReadPort.ts'

/** Why a source read did not answer, in the inspector's words. */
export type SourceUnready =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Unavailable'; readonly message: string }
  | { readonly _tag: 'Failed'; readonly message: string }

export type SourceMapState = SourceUnready | {
  readonly _tag: 'Ready'
  readonly map: SourceMap
  /** Canonical redacted KDL of `map.revision`. */
  readonly kdl: string
  readonly revisions: readonly SourceRevision[]
  /** The subject's current revision, which the picker marks live. */
  readonly current: string | undefined
}

const unread = (feed: Extract<Feed<unknown>, { readonly _tag: 'Unavailable' }>, noun: string): SourceUnready =>
  feed.reason === 'unauthorized' ? { _tag: 'Unavailable', message: credentialExpiredMessage }
    : feed.reason === 'ungranted' ? { _tag: 'Unavailable', message: 'Read access to declarations has not been granted to this connection.' }
      : feed.reason === 'unsupported' ? { _tag: 'Unavailable', message: `This connection cannot read ${noun}. ${feed.detail}` }
        : { _tag: 'Failed', message: `The ${noun} could not be loaded: ${feed.detail.replace(/\.$/, '')}. Nothing was changed; close and try again.` }

/** One revision's source map, or what the Source tab says instead; `revision` is undefined for the current one. */
export const sourceMapState = (feed: Feed<SourceMapLookup>, revision: string | undefined): SourceMapState => {
  if (feed._tag === 'Waiting') return { _tag: 'Loading' }
  if (feed._tag === 'Unavailable') return unread(feed, 'source map')
  const lookup = feed.value
  return lookup._tag === 'Unserved' ? { _tag: 'Unavailable', message: 'This Fractal does not serve source maps yet. The KDL tab shows the declaration.' }
    : lookup._tag === 'Unmapped' ? { _tag: 'Unavailable', message: `${revision === undefined ? 'The current revision' : `Revision ${revision.slice(0, 8)}`} was published without a source map, so Fractal cannot say which TypeScript declared it. The KDL tab shows the declaration.` }
      : { _tag: 'Ready', map: lookup.value.map, kdl: lookup.value.kdl, revisions: lookup.value.revisions, current: lookup.value.current }
}

/** File text by hash, or canonical KDL at a revision, as the inspector shows it. */
export const textState = <T>(feed: Feed<T>, text: (value: T) => string, noun: string): TextState => {
  if (feed._tag === 'Waiting') return { _tag: 'Loading' }
  if (feed._tag === 'Unavailable') {
    const state = unread(feed, noun)
    return state._tag === 'Loading' ? state : { _tag: 'Failed', message: state.message }
  }
  return { _tag: 'Ready', text: text(feed.value) }
}

/** The older side of Changes by default: the mapped revision just before `revision` (revisions are newest first). */
export const previousRevision = (revisions: readonly SourceRevision[], revision: string): string | undefined => {
  const at = revisions.findIndex(entry => entry.revision === revision)
  return at === -1 ? undefined : revisions[at + 1]?.revision
}
