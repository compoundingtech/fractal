import * as React from 'react'
import { activeTab, type Command, type WorkspaceState } from './model.ts'

const isTyping = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))

const inTerminal = (target: EventTarget | null) =>
  target instanceof Element && target.closest('[data-testid="terminal-pane"]') !== null

/**
 * The TUI's tab keys the shared primitives' keymap does not bind, as far as a browser lets a page have them:
 * `]` and `[`, Ctrl+^, Alt+1 to Alt+9, Alt+Shift+W, Ctrl+W and Ctrl+H. Splits, focus, zoom, Alt+W, Alt+Shift+T,
 * Alt+[ / Alt+] and Ctrl+PageUp/PageDown are the primitives' (`useWorkspaceKeys` from `@fractal/ui/workspace/react`). A bare `[`/`]` or Ctrl+H only applies outside text. Modified chords also work from the composer but
 * never over a terminal, where attached programs must receive their input unchanged.
 * Cmd+T/W and Ctrl+Tab belong to the browser; `event.code` keeps Alt chords working on Mac layouts.
 */
export const useWorkspaceKeys = ({ dispatch, getState, openInbox }: {
  readonly dispatch: (command: Command) => void
  readonly getState: () => WorkspaceState
  readonly openInbox: () => void
}) => {
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return
      const typing = isTyping(event.target), terminal = inTerminal(event.target)
      const plain = !event.metaKey && !event.ctrlKey && !event.altKey
      const run = (command: Command) => {
        event.preventDefault()
        event.stopPropagation()
        dispatch(command)
      }
      const active = activeTab(getState())?.id
      if (plain && !event.shiftKey && !typing && !terminal) {
        if (event.key === ']') return run({ _tag: 'Cycle', by: 1 })
        if (event.key === '[') return run({ _tag: 'Cycle', by: -1 })
      }
      if (terminal) return
      if (event.ctrlKey && !event.metaKey && !event.altKey) {
        if (event.key === '^' || event.key === '6') return run({ _tag: 'ActivatePrevious' })
        if (event.key === 'w' && active !== undefined) return run({ _tag: 'Close', tab: active })
        if (event.key === 'h' && !typing) {
          event.preventDefault()
          return openInbox()
        }
      }
      if (event.altKey && !event.metaKey && !event.ctrlKey) {
        if (event.code === 'KeyW' && event.shiftKey && active !== undefined) return run({ _tag: 'CloseOthers', tab: active })
        const digit = event.code.match(/^Digit([1-9])$/)?.[1]
        if (digit !== undefined) return run({ _tag: 'ActivateIndex', index: Number(digit) - 1 })
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [dispatch, getState, openInbox])
}
