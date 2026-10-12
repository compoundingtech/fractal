import * as React from 'react'
import { useAui } from '@assistant-ui/react'

/** The host owns scope, durability, failures and storage. The kit owns native composer state. */
export interface ComposerDraftStorage {
  readonly read: () => string
  readonly write: (text: string) => void
  readonly flush: () => void
}
export interface DraftComposer {
  readonly getText: () => string
  readonly setText: (text: string) => void
  readonly subscribe: (notify: () => void) => () => void
}

/** Hydrate before subscribing; native edits, recalls and accepted send-clears share one path. */
export const bindComposerDraft = ({ composer, storage }: { readonly composer: DraftComposer; readonly storage: ComposerDraftStorage }): (() => void) => {
  let previous = storage.read()
  composer.setText(previous)
  const save = () => {
    const text = composer.getText()
    if (text === previous) return
    previous = text
    storage.write(text)
  }
  const unsubscribe = composer.subscribe(save)
  const pagehide = () => { save(); storage.flush() }
  window.addEventListener('pagehide', pagehide)
  return () => { unsubscribe(); window.removeEventListener('pagehide', pagehide); save(); storage.flush() }
}

/** Stable storage identity is the host's conversation boundary; no browser storage is hidden here. */
export const useComposerDraft = (storage: ComposerDraftStorage): void => {
  const aui = useAui()
  React.useLayoutEffect(() => bindComposerDraft({
    composer: {
      getText: () => (aui.composer.__internal_getRuntime?.().getState() ?? aui.composer().getState()).text,
      setText: text => aui.composer.setText(text),
      subscribe: notify => aui.subscribe(notify),
    },
    storage,
  }), [aui, storage])
}
