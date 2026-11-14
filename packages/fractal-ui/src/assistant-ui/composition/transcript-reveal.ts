import * as React from 'react'

/** The entry transcript search has moved to. `deep` says the match lies past the row label, in the row's detail. */
export interface TranscriptReveal { readonly entryId?: string; readonly deep: boolean }
export interface TranscriptRevealStore {
  readonly get: () => TranscriptReveal
  readonly set: (next: TranscriptReveal) => void
  readonly subscribe: (listener: () => void) => () => void
}
const idle: TranscriptReveal = { deep: false }
export function createTranscriptRevealStore(): TranscriptRevealStore {
  let current = idle
  const listeners = new Set<() => void>()
  return {
    get: () => current,
    set: next => {
      if (next.entryId === current.entryId && next.deep === current.deep) return
      current = next
      for (const listener of listeners) listener()
    },
    subscribe: listener => { listeners.add(listener); return () => { listeners.delete(listener) } },
  }
}
export const TranscriptRevealContext = React.createContext<TranscriptRevealStore | undefined>(undefined)
const noSubscription = () => () => undefined

/**
 * Opens one disclosure while search stands on it, and closes it again only if search was what opened it. Only the
 * disclosures whose answer changes re-render, so stepping through matches stays cheap in a long transcript.
 */
export function useRevealOpen(matches: (reveal: TranscriptReveal) => boolean, open: boolean, setOpen: (open: boolean) => void) {
  const store = React.useContext(TranscriptRevealContext)
  const read = () => store !== undefined && matches(store.get())
  const revealed = React.useSyncExternalStore(store?.subscribe ?? noSubscription, read, () => false)
  const openedBySearch = React.useRef(false)
  React.useLayoutEffect(() => {
    if (revealed) {
      if (!open) { openedBySearch.current = true; setOpen(true) }
    } else if (openedBySearch.current) {
      openedBySearch.current = false
      setOpen(false)
    }
    // Only a change of target acts, so a reader can re-collapse the row search opened.
  }, [revealed])
}
