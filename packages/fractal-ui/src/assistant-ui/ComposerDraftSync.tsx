import { useAui } from '@assistant-ui/react'
import * as React from 'react'

/**
 * Where a host keeps a composer's unsent text. The kit only restores and reports text; the host
 * decides storage, staleness and when a draft from another window may replace this one.
 */
export interface ComposerDraftPort {
  /** The text to restore, or '' for none. */
  readonly text: () => string
  /** Fires when `text()` changes from outside this composer, e.g. another window's edit. */
  readonly subscribe: (listener: () => void) => () => void
  /** Every text change the composer makes, user edits and send clears alike. */
  readonly edit: (text: string) => void
}

/**
 * Restores a draft into an empty composer and reports its text changes. Render inside the
 * composer's runtime provider; renders nothing.
 */
export function ComposerDraftSync({ port }: { readonly port: ComposerDraftPort }) {
  const aui = useAui()
  React.useLayoutEffect(() => {
    let restoring = false
    const restore = () => {
      const text = port.text()
      if (text === '' || aui.composer().getState().text !== '') return
      restoring = true
      try { aui.composer().setText(text) } finally { restoring = false }
    }
    restore()
    const stopPort = port.subscribe(restore)
    let last = aui.composer().getState().text
    const stopComposer = aui.subscribe(() => {
      const text = aui.composer().getState().text
      if (text === last) return
      last = text
      if (!restoring) port.edit(text)
    })
    return () => { stopComposer(); stopPort() }
  }, [aui, port])
  return null
}
