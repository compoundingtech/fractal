import { RegistryContext, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as React from 'react'

import { countsAsNew, itemAtMs, latestAtMs, unreadAfter, unreadMark, type UnreadItem, type UnreadMark } from '../conversation/unread.ts'
import type { Agent } from '../data/source.ts'
import { useDataSource } from '../data/react.tsx'
import { cellReadPositionStore, type ReadPositionStore } from '../state/cells/readPositions.ts'

/** Fixtures and scenarios have no signed-in person; their positions belong to this browser. */
const browserPerson = Atom.make<string | undefined>('local')

/**
 * The times of the new items each conversation's loaded window last held, in memory only: what lets
 * a tab or row count without loading a transcript of its own. Keyed `<gateway>/<conversation>`.
 */
const observedProse = Atom.family((_key: string) => Atom.make<readonly number[] | undefined>(undefined).pipe(Atom.keepAlive))
const proseTimes = (items: readonly UnreadItem[]) => items.filter(countsAsNew).map(itemAtMs).filter(Number.isFinite)
const sameTimes = (left: readonly number[] | undefined, right: readonly number[]) =>
  left !== undefined && left.length === right.length && left.every((time, index) => time === right[index])

const useReadPositionStore = (): { readonly store: ReadPositionStore | undefined; readonly gateway: string; readonly registry: AtomRegistry.AtomRegistry } => {
  const source = useDataSource()
  const registry = React.useContext(RegistryContext)
  const person = useAtomValue(source.person ?? browserPerson)
  const gateway = source.gateway ?? source.mode
  const store = React.useMemo(() => person === undefined ? undefined : cellReadPositionStore({ registry, person, gateway }), [registry, person, gateway])
  return { store, gateway, registry }
}

const usePosition = (store: ReadPositionStore | undefined, conversation: string) => {
  const read = () => store?.read(conversation)
  return React.useSyncExternalStore(
    React.useCallback((listener: () => void) => store?.subscribe(conversation, listener) ?? (() => {}), [store, conversation]),
    read, read,
  )
}

/**
 * A conversation pane's read position: where "New since you last looked" goes, and the seen report
 * that moves the position forward. The divider is placed from the position held when the pane
 * became visible, so it stays put while the person reads, and goes when they send.
 */
export const useReadPosition = ({ conversation, items, visible, enabled, sendKey }: {
  readonly conversation: string
  readonly items: readonly UnreadItem[] | undefined
  readonly visible: boolean
  /** Off for read-only subagent threads: they have no row or tab of their own. */
  readonly enabled: boolean
  /** Changes with each send from this pane. */
  readonly sendKey?: string
}): { readonly unreadFrom?: string; readonly onSeen?: (itemIds: readonly string[]) => void } => {
  const { store, gateway, registry } = useReadPositionStore()
  const position = usePosition(enabled ? store : undefined, conversation)
  // Captured on each reveal; a send clears it, since the person has just answered what was new.
  const [frozen, setFrozen] = React.useState<{ readonly atMs: number | undefined; readonly sendKey?: string }>()
  const shown = visible && enabled && store !== undefined
  React.useEffect(() => { if (!shown) setFrozen(undefined) }, [shown])
  if (shown && frozen === undefined && items !== undefined) setFrozen({ atMs: position, sendKey })
  if (frozen !== undefined && frozen.sendKey !== sendKey) setFrozen({ atMs: Number.POSITIVE_INFINITY, sendKey })
  React.useEffect(() => {
    if (!enabled || store === undefined || items === undefined) return
    const times = proseTimes(items)
    const key = observedProse(`${gateway}/${conversation}`)
    if (!sameTimes(registry.get(key), times)) registry.set(key, times)
    // The first look starts at the tail: nothing in a conversation never opened is "new".
    if (shown && store.read(conversation) === undefined) {
      const latest = latestAtMs(items)
      if (latest !== undefined) store.advance(conversation, latest)
    }
  }, [enabled, store, items, gateway, conversation, registry, shown])
  // Only a send made while this pane is mounted moves the position here; a send kept from
  // before it mounted is old news, and reading moves the position through onSeen.
  const lastSend = React.useRef(sendKey)
  React.useEffect(() => {
    if (lastSend.current === sendKey) return
    lastSend.current = sendKey
    if (shown && sendKey !== undefined && items !== undefined) {
      const latest = latestAtMs(items)
      if (latest !== undefined) store!.advance(conversation, latest)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sendKey])
  const byId = React.useMemo(() => new Map((items ?? []).map(item => [item.id, item] as const)), [items])
  const onSeen = React.useCallback((ids: readonly string[]) => {
    const latest = latestAtMs(ids.flatMap(id => { const item = byId.get(id); return item === undefined ? [] : [item] }))
    if (latest !== undefined) store?.advance(conversation, latest)
  }, [byId, store, conversation])
  if (!enabled || store === undefined) return {}
  const unread = items === undefined ? undefined : unreadAfter(items, frozen?.atMs)
  return { ...(unread?.firstId === undefined ? {} : { unreadFrom: unread.firstId }), onSeen }
}

/**
 * The newest prose the roster knows of: the daemon's `last_prose_at`, the same signal the TUI and
 * the phone mark from. Last activity also moves on tool calls and prompts, so it stands in for
 * nothing: without the field, only a loaded window's count marks a conversation.
 */
export const latestProseAt = (agent: Agent): number | undefined =>
  agent.lastProseAt?._tag === 'Known' ? agent.lastProseAt.value : undefined

/** Unread marks for conversations by agent ref: an exact count where a window saw them, else a dot. */
export const useUnreadMarks = (agents: readonly Agent[]): ReadonlyMap<string, UnreadMark> => {
  const { store, gateway, registry } = useReadPositionStore()
  const subscribe = React.useCallback((listener: () => void) => {
    if (store === undefined) return () => {}
    const stops = agents.flatMap(agent => [store.subscribe(agent.ref, listener), registry.subscribe(observedProse(`${gateway}/${agent.ref}`), listener)])
    return () => { for (const stop of stops) stop() }
  }, [store, registry, gateway, agents])
  const snapshot = () => {
    if (store === undefined) return ''
    return JSON.stringify(agents.flatMap(agent => {
      const mark = unreadMark({ seenAtMs: store.read(agent.ref), observed: registry.get(observedProse(`${gateway}/${agent.ref}`)), latestProseAt: latestProseAt(agent) })
      return mark === undefined ? [] : [[agent.ref, mark]]
    }))
  }
  const key = React.useSyncExternalStore(subscribe, snapshot, snapshot)
  return React.useMemo(() => new Map<string, UnreadMark>(key === '' ? [] : JSON.parse(key)), [key])
}

export { unreadLabel } from '../conversation/unread.ts'

/** Mark read from a row's menu: the position moves to the newest prose anyone here knows of. */
export const useMarkRead = (agent: Agent | undefined): (() => void) | undefined => {
  const { store, gateway, registry } = useReadPositionStore()
  return React.useMemo(() => {
    if (store === undefined || agent === undefined) return undefined
    return () => {
      const times = [latestProseAt(agent), ...(registry.get(observedProse(`${gateway}/${agent.ref}`)) ?? [])].filter((time): time is number => time !== undefined)
      if (times.length > 0) store.advance(agent.ref, Math.max(...times))
    }
  }, [store, gateway, registry, agent])
}
