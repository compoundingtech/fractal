import * as React from 'react'
import { createLazyChunk, readLazyChunkAssets } from '@fractal/ui/lazy-chunk'

export type LazyPanelState<TModule> =
  | { readonly _tag: 'Idle' }
  | { readonly _tag: 'Pending' }
  | { readonly _tag: 'Ready'; readonly module: TModule }
  | { readonly _tag: 'Failed'; readonly recovery: 'retry' | 'reload' }

export interface LazyPanelRecovery {
  readonly label: 'Try again' | 'Reload'
  readonly onPress: () => void
}

/** One on-demand panel body: idle/intent prefetch and every open share the chunk's request. */
export interface LazyPanel<TModule> {
  /** Warms the body without spending the chunk's single in-document retry. */
  readonly prefetch: () => void
  /** Opening demand: loads, or retries a rejected attempt once before offering a reload. */
  readonly request: () => void
  readonly reload: () => void
  readonly subscribe: (listener: () => void) => () => void
  readonly getSnapshot: () => LazyPanelState<TModule>
}

/** `feature` is the build's lazy manifest key, so a retry can repair the emitted CSS. */
export const lazyPanel = <TModule,>(feature: string, load: () => Promise<TModule>): LazyPanel<TModule> => {
  const chunk = createLazyChunk({ load, assets: () => readLazyChunkAssets(feature) })
  const listeners = new Set<() => void>()
  let state: LazyPanelState<TModule> = { _tag: 'Idle' }
  const set = (next: LazyPanelState<TModule>) => {
    state = next
    for (const listener of listeners) listener()
  }
  const follow = (attempt: Promise<TModule>) => {
    set({ _tag: 'Pending' })
    attempt.then(
      module => { if (state._tag === 'Pending') set({ _tag: 'Ready', module }) },
      () => { if (state._tag === 'Pending') set({ _tag: 'Failed', recovery: chunk.recovery }) },
    )
  }
  return {
    prefetch: () => { if (state._tag === 'Idle') follow(chunk.load()) },
    request: () => {
      if (state._tag === 'Idle') follow(chunk.load())
      else if (state._tag === 'Failed' && state.recovery === 'retry') follow(chunk.retry())
    },
    reload: () => { void chunk.reload() },
    subscribe: listener => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    getSnapshot: () => state,
  }
}

/**
 * Mounting is demand. A warm body renders in the first commit; otherwise `pending` holds the
 * body's final geometry without a spinner, and a rejected chunk shows `fallback` with the
 * chunk's recovery (retry once in this document, then reload). A reopen retries by itself.
 */
export function LazyPanelBoundary<TModule>({ panel, pending, fallback, children }: {
  readonly panel: LazyPanel<TModule>
  readonly pending: React.ReactNode
  readonly fallback: (recovery: LazyPanelRecovery) => React.ReactNode
  readonly children: (module: TModule) => React.ReactNode
}) {
  const state = React.useSyncExternalStore(panel.subscribe, panel.getSnapshot, panel.getSnapshot)
  React.useLayoutEffect(() => panel.request(), [panel])
  if (state._tag === 'Ready') return children(state.module)
  if (state._tag !== 'Failed') return pending
  return fallback(state.recovery === 'retry'
    ? { label: 'Try again', onPress: panel.request }
    : { label: 'Reload', onPress: panel.reload })
}
