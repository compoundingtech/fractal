import * as React from 'react'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as Option from 'effect/Option'
import { useClockLabel } from '../../../../packages/fractal-ui/src/time/clock.tsx'
import { useDataSource } from '../data/react.tsx'
import type { FeedSync } from '../data/feedSync.ts'

const noObservation = Atom.make<number | undefined>(undefined)
/** The source's most recent actual observation of a feed, never a status transition or a render time. */
const useLastObservedAt = <TValue,>(sync: Atom.Atom<FeedSync<TValue>> | undefined): number | undefined => {
  const observedAt = React.useMemo(
    () => sync === undefined ? noObservation : Atom.make((get) => Option.getOrUndefined(get(sync).last)?.observedAt),
    [sync],
  )
  return useAtomValue(observedAt)
}
/** When the roster was last actually observed; undefined when the source reports no sync sidecar or never observed it. */
export const useAgentsLastObservedAt = (): number | undefined => useLastObservedAt(useDataSource().sync?.agents)
/** When this conversation was last actually observed. */
export const useConversationLastObservedAt = (agentRef: string): number | undefined => {
  const sync = useDataSource().sync
  return useLastObservedAt(React.useMemo(() => sync?.conversation(agentRef), [sync, agentRef]))
}

/** `18 s ago`, `4 min ago`, `2 h ago`: whole units elapsed since a real observation. */
export const observedAge = (at: number, now: number): string => {
  const seconds = Math.max(0, Math.floor((now - at) / 1000))
  return seconds < 60 ? `${seconds} s ago` : seconds < 3600 ? `${Math.floor(seconds / 60)} min ago` : `${Math.floor(seconds / 3600)} h ago`
}

/** The only ticking node: one leaf subscribes to the shared source clock, so a tick never re-renders rows or panes. */
export const ObservedAge = ({ prefix, at }: { readonly prefix: string; readonly at: number }) =>
  <>{useClockLabel(now => `${prefix}${observedAge(at, now)}`)}</>
