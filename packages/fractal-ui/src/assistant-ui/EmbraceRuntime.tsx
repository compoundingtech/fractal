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
 * Message ids the runtime holds after adopting the latest snapshot, or undefined before the first
 * adoption. `Transcript` uses it to tell ids the runtime adopted (its store is still publishing them)
 * from ids it never adopted. It is a store rather than state, so readers re-render only when the
 * ids they are waiting for change, never merely because an adoption happened.
 */
export interface RuntimeAdoption {
  readonly get: () => ReadonlySet<string> | undefined
  /**
   * Whether the provider's current snapshot (its `options`) reached the runtime. It answers per
   * snapshot, not per id: a host may filter entries out of the adapter, so adoption must not be
   * inferred from which ids the snapshot carries.
   */
  readonly isSnapshotAdopted: () => boolean
  readonly subscribe: (listener: () => void) => () => void
}
export const RuntimeAdoptedIds = React.createContext<RuntimeAdoption | undefined>(undefined)
function createRuntimeAdoptionStore() {
  let ids: ReadonlySet<string> | undefined
  let adopted: ConversationRuntimeOptions | undefined
  const listeners = new Set<() => void>()
  return {
    get: () => ids,
    getAdopted: () => adopted,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    publish: (next: ReadonlySet<string>, snapshot: ConversationRuntimeOptions) => { ids = next; adopted = snapshot; for (const listener of listeners) listener() },
  }
}
const historyAdoptionBatchSize = 8
/**
 * The runtime eagerly builds message resources even for rows the transcript leaves unmounted.
 * Adopt an older prefix in small paint-sized batches, retaining the current suffix throughout.
 * Replacements and live tail changes remain immediate.
 */
function useHistoryAdoption(options: ConversationRuntimeOptions): ConversationRuntimeOptions {
  const [window, setWindow] = React.useState({ source: options, offset: 0 })
  let offset = window.offset
  if (window.source !== options) {
    const previous = window.source.messages?.slice(window.offset)
    const messages = options.messages
    const added = messages === undefined || previous === undefined ? 0 : messages.length - previous.length
    offset = messages !== undefined && previous !== undefined && added > 0 && previous.length > 0 && previous.every((item, index) => messages[added + index] === item)
      ? Math.max(0, added - historyAdoptionBatchSize)
      : 0
    setWindow({ source: options, offset })
  }
  React.useEffect(() => {
    if (offset === 0) return
    const frame = requestAnimationFrame(() => setWindow(current => current.source === options
      ? { source: options, offset: Math.max(0, current.offset - historyAdoptionBatchSize) }
      : current))
    return () => cancelAnimationFrame(frame)
  }, [options, offset])
  return React.useMemo(() => offset === 0 || options.messages === undefined ? options : { ...options, messages: options.messages.slice(offset) }, [options, offset])
}
export function EmbraceRuntimeProvider({ options, children }: { readonly options: ConversationRuntimeOptions; readonly children: React.ReactNode }) {
  const adoptedOptions = useHistoryAdoption(options)
  const runtime = useConversationRuntime(adoptedOptions)
  const [store] = React.useState(createRuntimeAdoptionStore)
  // Declared after the runtime hook, so it reads the runtime after that hook's adoption effect for the same snapshot.
  React.useEffect(() => store.publish(new Set(runtime.thread.getState().messages.map(message => message.id)), adoptedOptions), [runtime, adoptedOptions, store])
  // Each render's context compares against its own snapshot, so a discarded render never marks adoption.
  const adoption = React.useMemo<RuntimeAdoption>(() => ({ get: store.get, subscribe: store.subscribe, isSnapshotAdopted: () => store.getAdopted() === options }), [store, options])
  return <AssistantRuntimeProvider runtime={runtime}><RuntimeAdoptedIds.Provider value={adoption}>{children}</RuntimeAdoptedIds.Provider></AssistantRuntimeProvider>
}
