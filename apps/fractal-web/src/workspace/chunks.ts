import * as React from 'react'
import { createLazyChunk, readLazyChunkAssets } from '@fractal/ui/lazy-chunk'
import type * as TerminalDetailModule from '../terminal/TerminalDetail.tsx'

// A static import would evaluate the kit Transcript/Markdown before the shell paints.
// Demand and speculative prefetch share one request; a rejection stays until a user retry.
export const conversationChunk = createLazyChunk({ load: () => import('../web/ConversationPane.tsx'), assets: () => readLazyChunkAssets('conversation') })

export type LazyConversationPane = React.LazyExoticComponent<(typeof import('../web/ConversationPane.tsx'))['ConversationPane']>

const lazyConversationPane = (load: () => ReturnType<typeof conversationChunk.load>): LazyConversationPane =>
  React.lazy(() => load().then(module => ({ default: module.ConversationPane })))

// An initially selected pane and later shell prefetch must retain the same lazy identity.
// A second wrapper suspends on an already resolved module during the first seat switch.
let currentConversationPane: LazyConversationPane = lazyConversationPane(() => conversationChunk.load())

export const conversationPane = (): LazyConversationPane => currentConversationPane

/** A retry takes a fresh lazy wrapper, not React's permanently rejected cached one. */
export const retryConversationPane = (): LazyConversationPane => {
  currentConversationPane = lazyConversationPane(() => conversationChunk.retry())
  return currentConversationPane
}

export const prefetchConversationPane = () => {
  // Preserve this attempt's failure for the requested pane's honest, local error surface.
  void conversationChunk.load().catch(() => {})
}

/** Set once the module resolves, so a warm reopen renders it directly instead of suspending a fresh lazy identity. */
let terminalDetailLoaded: typeof TerminalDetailModule | undefined

export const loadedTerminalDetail = () => terminalDetailLoaded?.TerminalDetail

/** Demand and idle intent share one request; a rejection stays until a user retry. */
export const terminalChunk = createLazyChunk({
  load: () => import('../terminal/TerminalDetail.tsx').then(module => {
    terminalDetailLoaded = module
    return module
  }),
  assets: () => readLazyChunkAssets('terminal'),
})

export const terminalDetail = (load: () => Promise<typeof TerminalDetailModule>) => React.lazy(() =>
  load().then(module => {
    void module.prepareTerminal()
    return { default: module.TerminalDetail }
  }),
)

export const prefetchTerminalDetail = () => {
  void terminalChunk.load().then(module => module.prepareTerminal()).catch(() => {})
}
