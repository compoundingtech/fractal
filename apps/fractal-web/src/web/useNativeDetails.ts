import { RegistryContext, useAtomValue } from '@effect/atom-react'
import { DateTime, Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { useAgent, useAgentRow, useDataSource, useNow, useSubjectList } from '../data/react.tsx'
import { projectNativeDetails, type NativeDetailsObservation } from '../data/nativeDetails.ts'
import { unavailable, type Feed } from '../data/source.ts'
import type * as Native from '@smalltalk/st3-client/schema'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'
import { sidebarAttention } from './sidebarAttention.ts'

export type { NativeDetailsObservation } from '../data/nativeDetails.ts'
const notReported = Atom.make(unavailable({ reason: 'unsupported', detail: 'The selected native fact is not reported.' }))

/** A session start is immutable within its native incarnation; a forward drift is not a fresh age. */
export const guardedSessionTiming = (input: Atom.Atom<Feed<Native.Session>>): Atom.Atom<Feed<Native.Session>> => {
  let earliest: { readonly id: string; readonly incarnation?: string; readonly start: number } | undefined
  return Atom.make(get => {
    const feed = get(input)
    if (feed._tag !== 'Observed') return feed
    const session = feed.value
    const start = DateTime.toEpochMillis(session.started_at)
    const incarnation = Option.getOrUndefined(session.runtime_incarnation)
    if (earliest === undefined || earliest.id !== session.id || earliest.incarnation !== incarnation || start < earliest.start)
      earliest = { id: session.id, incarnation, start }
    return start > earliest.start
      ? unavailable({ reason: 'failed', detail: 'Session start changed between observations; elapsed time and age are unreported.' })
      : feed
  })
}

/** Mount only with the Details surface: atom lifetimes release every selected getter demand. */
export const useNativeDetails = ({ agentRef }: { readonly agentRef: string }): NativeDetailsObservation => {
  const source = useDataSource()
  const registry = React.useContext(RegistryContext)
  const now = useNow()
  const agent = useAgent(agentRef)
  const row = useAgentRow(agentRef)
  const selected = row._tag === 'Observed' ? row.value : undefined
  const sessionRef = selected === undefined ? undefined : Option.getOrUndefined(selected.current_session_id)
  const sessionAtom: Atom.Atom<Feed<Native.Session>> = sessionRef === undefined ? notReported : source.subjectReads.session.feed(sessionRef)
  const timingSessionAtom = React.useMemo(() => guardedSessionTiming(sessionAtom), [sessionAtom])
  const session = useAtomValue(timingSessionAtom)
  const periodAtom: Atom.Atom<Feed<Native.UsagePeriod>> = source.nativeDetails?.usagePeriod ?? notReported
  const period = useAtomValue(periodAtom)
  const declarationAtom: Atom.Atom<Feed<NativeDeclarations>> = selected === undefined ? notReported : source.nativeDetails?.declarations({ agentRef, revision: selected.revision }) ?? notReported
  const declarations = useAtomValue(declarationAtom)
  const missions = useAtomValue(source.missions)
  const queueAtom = source.subjectReads.agentQueue.feed(agentRef)
  const queue = useAtomValue(queueAtom)
  const subjects = useSubjectList()
  const needsMe = agent._tag === 'Observed' && agent.error?.reason !== 'ungranted' && agent.value !== undefined
    ? sidebarAttention({ agents: [agent.value], subjects }).has(agentRef) : undefined
  const onRetry = () => {
    const retrySubject = (atom: Atom.Atom<Feed<unknown>>) => {
      const feed = registry.get(atom)
      if ((feed._tag === 'Unavailable' && feed.reason === 'failed') ||
        (feed._tag === 'Observed' && feed.error?.reason === 'failed')) registry.refresh(atom)
    }
    retrySubject(sessionAtom)
    retrySubject(queueAtom)
    source.nativeDetails?.retry(periodAtom)
    source.nativeDetails?.retry(declarationAtom)
    if ((row._tag === 'Unavailable' && row.reason === 'failed') ||
      (row._tag === 'Observed' && row.freshness === 'stale')) source.reconnect?.()
  }
  return projectNativeDetails({ agentRef, agent, row, session, period, declarations, missions, queue, needsMe, now, onRetry })
}
