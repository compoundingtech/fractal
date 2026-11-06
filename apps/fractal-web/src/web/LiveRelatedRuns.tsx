import * as React from 'react'
import { useAtomValue } from '@effect/atom-react'
import type * as Native from '@fractal/client/schema/read'
import { SidebarRelatedRuns, type SidebarRelatedRunsState } from '@fractal/ui/assistant-ui/shell'
import { useDataSource } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'

const unavailableCopy = {
  unsupported: 'this server does not serve them yet',
  ungranted: 'this device is not allowed to read them',
} as const

/** The served projection verbatim. A retained value stays drawn while a pushed invalidation refetches or that read fails. */
export const relatedRunsState = (feed: Feed<Native.AgentRelatedRuns>): SidebarRelatedRunsState =>
  feed._tag === 'Observed' ? { _tag: 'Loaded', value: feed.value }
    : feed._tag === 'Waiting' ? { _tag: 'Loading' }
    : { _tag: 'Failed', message: feed.reason === 'failed' ? feed.detail : unavailableCopy[feed.reason] }

/** Open asks the server holds on the person across this agent's runs; the label announces them. */
export const relatedRunsLabel = (state: SidebarRelatedRunsState): string => {
  const asks = state._tag === 'Loaded' ? state.value.rows.filter(run => run.waiting_on_you).length : 0
  return asks === 0 ? 'Related missions' : `Related missions, ${asks} waiting on you`
}

/**
 * Mounted only by a rendered roster row, so virtualized rows read nothing. The reader owns invalidation; this never polls.
 * A served empty projection draws nothing beneath the row; loading and failure stay visible.
 */
export const LiveRelatedRuns = React.memo(function LiveRelatedRuns({ agentRef, now }: { readonly agentRef: string; readonly now?: number }) {
  const feed = useAtomValue(useDataSource().subjectReads.relatedRuns.feed(agentRef))
  const state = React.useMemo(() => relatedRunsState(feed), [feed])
  if (state._tag === 'Loaded' && state.value.rows.length === 0 && !state.value.truncated && feed._tag === 'Observed' && feed.error === undefined) return null
  return <>{feed._tag === 'Observed' && feed.error !== undefined
    ? <p role="status">Related missions unavailable: {feed.error.detail}</p> : null}
    <SidebarRelatedRuns state={state} label={relatedRunsLabel(state)} now={now} /></>
})
