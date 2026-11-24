import type { CreateExtensions, ExtensionViews, ExtensionFixtures, UseDetailsSections, UseDetailsSupplement } from './contract.ts'
import type { DataSource, Feed } from '../data/source.ts'
import type { DetailsSupplement } from './contract.ts'
import { useAtomValue } from '@effect/atom-react'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { useAgentRow, useDataSource } from '../data/react.tsx'
import { createDetailsSupplementCache, type DetailsSupplementCache } from '../data/rootDetail.ts'
export const extensionViews: ExtensionViews = Object.freeze({})
export const extensionFixtures: ExtensionFixtures = Object.freeze({envelopes: [], subjects: []})

/** The public application has no injected renderer claims. */
export const createRendererExtensions: CreateExtensions = (_host) => Object.freeze({
  renderers: Object.freeze([]),
  native: Object.freeze([]),
})

export { createExtensions } from './public.ts'

const caches = new WeakMap<DataSource, DetailsSupplementCache>()
const undeclared = Atom.make<Feed<DetailsSupplement>>({ _tag: 'Unavailable', reason: 'unsupported', detail: 'Descendant observations are not supplied by this source.' })
const pending = Atom.make<Feed<DetailsSupplement>>({ _tag: 'Waiting' })

/**
 * Root detail from the source's declared relay, read only while the agent's current row agrees
 * with the queried incarnation, host and harness; a stale row pauses reads to the cached answer.
 */
export const useDetailsSupplement: UseDetailsSupplement = (query) => {
  const source = useDataSource()
  const current = useAgentRow(query.agentRef)
  const declared = source.rootDetail
  let cache = caches.get(source)
  if (cache === undefined && declared !== undefined) { cache = createDetailsSupplementCache(declared); caches.set(source, cache) }
  const identityMatches = current._tag === 'Observed' && current.error?.reason !== 'ungranted' && current.value !== undefined &&
    Option.getOrUndefined(current.value.incarnation_id) === query.incarnationId && Option.getOrUndefined(current.value.host_id) === query.host &&
    Option.getOrUndefined(current.value.driver) === query.harness
  const canRefresh = query.canRefresh !== false && current._tag === 'Observed' && current.freshness === 'live'
  const denied = current._tag === 'Unavailable' && current.reason === 'ungranted' || current._tag === 'Observed' && current.error?.reason === 'ungranted'
  const waiting = current._tag === 'Waiting'
  const atom = React.useMemo(() => cache === undefined ? undeclared
    : waiting ? pending
      : identityMatches ? cache({ ...query, canRefresh })
        : Atom.make<Feed<DetailsSupplement>>({ _tag: 'Unavailable', reason: denied ? 'ungranted' : 'unsupported', detail: 'Current native identity is unavailable for descendant observations.' }),
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by the query's fields, not its object identity
  [cache, identityMatches, denied, waiting, canRefresh, query.agentRef, query.nativeSessionId, query.incarnationId, query.host, query.harness, query.nativeRootPending])
  return useAtomValue(atom)
}

export const useDetailsSections: UseDetailsSections = () => []
