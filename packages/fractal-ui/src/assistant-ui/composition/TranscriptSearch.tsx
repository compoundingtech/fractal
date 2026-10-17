import * as React from 'react'
import { flushSync } from 'react-dom'
import type { TranscriptTurn } from './Transcript'

/** The Find shortcut exposes rendered history; message links additionally navigate to their target. */
export function TranscriptSearch({ turns, startIndex, revealRow, children, ...props }: React.HTMLAttributes<HTMLDivElement> & { readonly turns: readonly TranscriptTurn[]; readonly startIndex: number; readonly revealRow: (id: string, preservePosition?: boolean) => void }) {
  const root = React.useRef<HTMLDivElement>(null)
  const appliedHash = React.useRef<string | undefined>(undefined)
  React.useEffect(() => {
    const element = root.current
    if (element === null) return
    const hash = () => {
      if (element.closest('[aria-hidden="true"], [hidden]') !== null) return
      let id: string
      try { id = decodeURIComponent(window.location.hash.slice(1)) } catch { return }
      if (id === '') return
      const turn = turns.find(turn => turn.id === id || turn.prompt?.id === id || turn.items.some(item => item.id === id))
      if (turn === undefined) return
      appliedHash.current = window.location.hash
      flushSync(() => revealRow(turn.id))
      const row = Array.from(element.querySelectorAll<HTMLElement>('[data-item-id]')).find(row => row.dataset.itemId === id)
      row?.scrollIntoView({ block: 'start' })
    }
    const find = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f' && startIndex > 0 && element.closest('[aria-hidden="true"], [hidden]') === null) {
        // Do not navigate or detach following merely because the reader opens (or cancels) Find.
        flushSync(() => revealRow(turns[0]!.id, true))
      }
    }
    window.addEventListener('hashchange', hash)
    document.addEventListener('keydown', find, true)
    const frame = window.location.hash === '' || appliedHash.current === window.location.hash ? undefined : requestAnimationFrame(hash)
    return () => {
      window.removeEventListener('hashchange', hash)
      document.removeEventListener('keydown', find, true)
      if (frame !== undefined) cancelAnimationFrame(frame)
    }
  }, [turns, startIndex, revealRow])
  return <div {...props} ref={root}>{children}</div>
}
