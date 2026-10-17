import * as React from 'react'
import type { TranscriptTurn } from './Transcript'

/** The Find shortcut exposes rendered history; message links additionally navigate to their target. */
export function TranscriptSearch({ turns, startIndex, revealRow, children, ...props }: React.HTMLAttributes<HTMLDivElement> & { readonly turns: readonly TranscriptTurn[]; readonly startIndex: number; readonly revealRow: (id: string, preservePosition?: boolean) => void }) {
  const root = React.useRef<HTMLDivElement>(null)
  const appliedHash = React.useRef<string | undefined>(undefined)
  const pendingHash = React.useRef<{ readonly id: string; readonly turnId: string } | undefined>(undefined)
  React.useEffect(() => {
    const element = root.current
    if (element === null) return
    const scrollPendingHash = () => {
      const pending = pendingHash.current
      if (pending === undefined || element.closest('[aria-hidden="true"], [hidden]') !== null) return
      const row = element.querySelector<HTMLElement>(`[data-item-id="${CSS.escape(pending.id)}"]`)
      if (row === null) return
      pendingHash.current = undefined
      // Chunk commits preserved the old view. Actual target navigation now takes ownership.
      revealRow(pending.turnId)
      row.scrollIntoView({ block: 'start' })
    }
    scrollPendingHash()
    const hash = () => {
      if (element.closest('[aria-hidden="true"], [hidden]') !== null) return
      let id: string
      try { id = decodeURIComponent(window.location.hash.slice(1)) } catch { return }
      if (id === '') return
      const turn = turns.find(turn => turn.id === id || turn.prompt?.id === id || turn.items.some(item => item.id === id))
      if (turn === undefined) return
      appliedHash.current = window.location.hash
      pendingHash.current = { id, turnId: turn.id }
      revealRow(turn.id)
      scrollPendingHash()
    }
    const find = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f' && startIndex > 0 && element.closest('[aria-hidden="true"], [hidden]') === null) {
        // Do not navigate or detach following merely because the reader opens (or cancels) Find.
        revealRow(turns[0]!.id, true)
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
