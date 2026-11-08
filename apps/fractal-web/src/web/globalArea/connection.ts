import * as Atom from 'effect/reactivity/Atom'
import { useAtomValue } from '@effect/atom-react'
import { formatDuration } from '@fractal/ui/human-time/format'
import type { ConnectionState } from '../../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { syncLine } from '../../../../../packages/fractal-ui/src/assistant-ui/sync-presentation/sync-line.ts'
import type { FeedSyncObservation } from '../../data/feedSync.ts'
import type { NetworkReachability } from '../../data/source.ts'
import { useDataSource, useGatewaySync, useNow } from '../../data/react.tsx'

/** The gateway connection as the global area says it: its state, the detail, and whether Reconnect can help. */
export interface ConnectionView {
  readonly state: ConnectionState
  readonly text?: string
  readonly reconnect: boolean
}

/** Decoded gateway sync, never the socket's raw lifecycle tag. An offline network outranks whatever was last observed. */
export const connectionView = ({ observation, network, now, canReconnect }: {
  readonly observation: FeedSyncObservation | undefined
  readonly network: NetworkReachability
  readonly now: number
  /** The source has a way to reconnect at all. */
  readonly canReconnect: boolean
}): ConnectionView => {
  if (observation === undefined) return { state: 'live', reconnect: false }
  const status = observation.status
  const line = syncLine({ ...observation, label: 'connection', now, socket: status._tag !== 'Failed' })
  const reconnecting = status._tag === 'Stale' && status.reason._tag === 'Reconnecting'
  const offline = network._tag === 'Offline'
  const state: ConnectionState = offline ? 'offline' : reconnecting ? 'reconnecting' : status._tag === 'Stale' ? 'stale' : status._tag === 'Failed' ? 'failed' : 'live'
  // The title already says "Reconnecting"; the detail is the next try, after syncLine's grace delay.
  const text = line === undefined ? undefined
    : status._tag === 'Stale' && status.reason._tag === 'Reconnecting'
      ? `Next try in ${formatDuration(Math.ceil(Math.max(0, status.reason.nextAt - now) / 1000) * 1000)} · attempt ${status.reason.attempt}`
      : line.text
  return { state, ...(text === undefined ? {} : { text }), reconnect: canReconnect && (reconnecting || offline) }
}

/**
 * The footer owns the connection; the focused pane gets a band only once it is gone, not while it recovers, so a
 * phone with its drawer closed, or a collapsed sidebar, still hears it where the person is looking.
 */
export const paneBandShown = (state: ConnectionState): boolean => state === 'failed' || state === 'offline'

const assumedOnline = Atom.make({ _tag: 'Online' } as const)

/** The live connection view, plus the source's reconnect for whichever surface offers it. */
export const useConnectionView = (): ConnectionView & { readonly onReconnect?: () => void } => {
  const source = useDataSource()
  const network = useAtomValue(source.network ?? assumedOnline)
  const view = connectionView({ observation: useGatewaySync(), network, now: useNow(), canReconnect: source.reconnect !== undefined })
  return view.reconnect && source.reconnect !== undefined ? { ...view, onReconnect: source.reconnect } : view
}
