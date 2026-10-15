import * as React from 'react'
import { AssistantRuntimeProvider, useExternalStoreRuntime, type ExternalStoreAdapter } from '@assistant-ui/react'
import { convertConversationItem } from './embrace-converter'
import type { ConversationItem } from './embrace-data/model.ts'

/** App-owned snapshots and capabilities. No transport, synthetic state, or store is hidden here. */
export type ConversationRuntimeOptions = Omit<ExternalStoreAdapter<ConversationItem>, 'convertMessage'>
export function useConversationRuntime(options: ConversationRuntimeOptions) {
  return useExternalStoreRuntime<ConversationItem>({ ...options, convertMessage: convertConversationItem })
}
/**
 * Message ids held by the current runtime window or deferred outside it, or undefined before the
 * first commit. The source remains complete; only mounted virtual rows create client message
 * resources. A converter rejecting a requested item still becomes a visible fallback row.
 */
export interface RuntimeAdoption {
  readonly get: () => ReadonlySet<string> | undefined
  readonly subscribe: (listener: () => void) => () => void
}
export const RuntimeAdoptedIds = React.createContext<RuntimeAdoption | undefined>(undefined)
function createRuntimeAdoption() {
  let ids: ReadonlySet<string> | undefined
  const listeners = new Set<() => void>()
  return {
    get: () => ids,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    publish: (next: ReadonlySet<string>) => { ids = next; for (const listener of listeners) listener() },
  }
}
const noSubscription = () => () => undefined
const noMessages: readonly ConversationItem[] = []
/** Internal viewport demand, not a second pending-send or host-history API. */
export const RuntimeVisibleIds = React.createContext<((ids: readonly string[]) => void) | undefined>(undefined)
export function EmbraceRuntimeProvider({ options, virtualizeMessages = false, children }: { readonly options: ConversationRuntimeOptions; readonly virtualizeMessages?: boolean; readonly children: React.ReactNode }) {
  const measurable = React.useSyncExternalStore(noSubscription, () => true, () => false)
  const [visibleIds, setVisibleIds] = React.useState<ReadonlySet<string>>()
  const source = options.messages ?? noMessages
  const messages = React.useMemo(() => !virtualizeMessages || !measurable ? source : visibleIds === undefined ? noMessages : source.filter(item => visibleIds.has(item.id)), [source, measurable, visibleIds, virtualizeMessages])
  const visible = React.useCallback((ids: readonly string[]) => {
    setVisibleIds(previous => previous !== undefined && previous.size === ids.length && ids.every(id => previous.has(id)) ? previous : new Set(ids))
  }, [])
  const runtime = useConversationRuntime({ ...options, messages })
  const [adoption] = React.useState(createRuntimeAdoption)
  React.useEffect(() => {
    const requested = new Set(messages.map(item => item.id))
    const ids = new Set(runtime.thread.getState().messages.map(message => message.id))
    for (const item of source) if (!requested.has(item.id)) ids.add(item.id)
    adoption.publish(ids)
  }, [runtime, source, messages, adoption])
  return <AssistantRuntimeProvider runtime={runtime}><RuntimeAdoptedIds.Provider value={adoption}><RuntimeVisibleIds.Provider value={virtualizeMessages ? visible : undefined}>{children}</RuntimeVisibleIds.Provider></RuntimeAdoptedIds.Provider></AssistantRuntimeProvider>
}
