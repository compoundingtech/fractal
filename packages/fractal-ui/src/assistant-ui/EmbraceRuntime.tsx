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
  readonly getSource: () => readonly ConversationItem[] | undefined
  readonly subscribe: (listener: () => void) => () => void
}
export const RuntimeAdoptedIds = React.createContext<RuntimeAdoption | undefined>(undefined)
/** Source identity lets rows distinguish settled absence from an older adoption publication. */
export const RuntimeSourceItems = React.createContext<readonly ConversationItem[] | undefined>(undefined)
function createRuntimeAdoption() {
  let ids: ReadonlySet<string> | undefined
  let source: readonly ConversationItem[] | undefined
  const listeners = new Set<() => void>()
  return {
    get: () => ids,
    getSource: () => source,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    publish: (next: ReadonlySet<string>, snapshot: readonly ConversationItem[]) => { ids = next; source = snapshot; for (const listener of listeners) listener() },
  }
}
const noSubscription = () => () => undefined
const noMessages: readonly ConversationItem[] = []
/** Internal viewport demand, not a second pending-send or host-history API. */
export const RuntimeVisibleIds = React.createContext<((viewport: symbol, ids: readonly string[] | undefined) => void) | undefined>(undefined)
export function EmbraceRuntimeProvider({ options, virtualizeMessages = false, children }: { readonly options: ConversationRuntimeOptions; readonly virtualizeMessages?: boolean; readonly children: React.ReactNode }) {
  const measurable = React.useSyncExternalStore(noSubscription, () => true, () => false)
  const [visibleIds, setVisibleIds] = React.useState<ReadonlySet<string>>()
  const windows = React.useRef(new Map<symbol, readonly string[]>())
  const source = options.messages ?? noMessages
  const messages = React.useMemo(() => !virtualizeMessages || !measurable ? source : visibleIds === undefined ? noMessages : source.filter(item => visibleIds.has(item.id)), [source, measurable, visibleIds, virtualizeMessages])
  const visible = React.useCallback((viewport: symbol, ids: readonly string[] | undefined) => {
    if (ids === undefined) windows.current.delete(viewport)
    else windows.current.set(viewport, ids)
    const union = new Set<string>()
    for (const window of windows.current.values()) for (const id of window) union.add(id)
    setVisibleIds(previous => previous !== undefined && previous.size === union.size && [...union].every(id => previous.has(id)) ? previous : union)
  }, [])
  const runtime = useConversationRuntime({ ...options, messages })
  const [adoption] = React.useState(createRuntimeAdoption)
  React.useEffect(() => {
    const requested = new Set(messages.map(item => item.id))
    const ids = new Set(runtime.thread.getState().messages.map(message => message.id))
    for (const item of source) if (!requested.has(item.id)) ids.add(item.id)
    adoption.publish(ids, source)
  }, [runtime, source, messages, adoption])
  return <AssistantRuntimeProvider runtime={runtime}><RuntimeSourceItems.Provider value={source}><RuntimeAdoptedIds.Provider value={adoption}><RuntimeVisibleIds.Provider value={virtualizeMessages ? visible : undefined}>{children}</RuntimeVisibleIds.Provider></RuntimeAdoptedIds.Provider></RuntimeSourceItems.Provider></AssistantRuntimeProvider>
}
