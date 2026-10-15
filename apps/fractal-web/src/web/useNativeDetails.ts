import { RegistryContext, useAtomValue } from '@effect/atom-react'
import { Option } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { useAgent, useAgentRow, useDataSource, useNow } from '../data/react.tsx'
import { projectNativeDetails, type NativeDetailsObservation } from '../data/nativeDetails.ts'
import { unavailable, type Feed } from '../data/source.ts'
import type * as Native from '@smalltalk/st3-client/schema'
import type { NativeDeclarations } from '../data/nativeDeclarations.ts'

export type { NativeDetailsObservation } from '../data/nativeDetails.ts'
const notReported = Atom.make(unavailable({ reason: 'unsupported', detail: 'The selected native fact is not reported.' }))

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
  const session = useAtomValue(sessionAtom)
  const periodAtom: Atom.Atom<Feed<Native.UsagePeriod>> = source.nativeDetails?.usagePeriod ?? notReported
  const period = useAtomValue(periodAtom)
  const declarationAtom: Atom.Atom<Feed<NativeDeclarations>> = selected === undefined ? notReported : source.nativeDetails?.declarations({ agentRef, revision: selected.revision }) ?? notReported
  const declarations = useAtomValue(declarationAtom)
  const missions = useAtomValue(source.missions)
  const onRetry = () => {
    const retrySubject = (atom: Atom.Atom<Feed<unknown>>) => {
      const feed = registry.get(atom)
      if ((feed._tag === 'Unavailable' && feed.reason === 'failed') ||
        (feed._tag === 'Observed' && feed.error?.reason === 'failed')) registry.refresh(atom)
    }
    retrySubject(sessionAtom)
    source.nativeDetails?.retry(periodAtom)
    source.nativeDetails?.retry(declarationAtom)
    if ((row._tag === 'Unavailable' && row.reason === 'failed') ||
      (row._tag === 'Observed' && row.freshness === 'stale')) source.reconnect?.()
  }
  return projectNativeDetails({ agentRef, agent, row, session, period, declarations, missions, now, onRetry })
}
