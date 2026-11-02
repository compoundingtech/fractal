import * as React from 'react'
import type { TranscriptTurn } from './Transcript'

/** The Find shortcut exposes rendered history; message links additionally navigate to their target. `rowIds` are the viewport's source rows; `startIndex` is the first mounted one. */
export function TranscriptSearch({ turns, rowIds, startIndex, revealRow, children, ...props }: React.HTMLAttributes<HTMLDivElement> & { readonly turns: readonly TranscriptTurn[]; readonly rowIds: readonly string[]; readonly startIndex: number; readonly revealRow: (id: string, preservePosition?: boolean) => void }) {
  const root = React.useRef<HTMLDivElement>(null)
  const appliedHash = React.useRef<string | undefined>(undefined)
  const hashGeneration = React.useRef(0)
  const pendingHash = React.useRef<{ readonly id: string; readonly rowId: string; readonly hash: string; readonly generation: number } | undefined>(undefined)
  React.useEffect(() => {
    const element = root.current
    if (element === null) return
    const scrollPendingHash = () => {
      const pending = pendingHash.current
      if (pending === undefined) return
      if (pending.generation !== hashGeneration.current || pending.hash !== window.location.hash) {
        pendingHash.current = undefined
        const first = rowIds[startIndex]
        if (first !== undefined) revealRow(first, true)
        return
      }
      if (element.closest('[aria-hidden="true"], [hidden]') !== null) return
      const row = element.querySelector<HTMLElement>(`[data-item-id="${CSS.escape(pending.id)}"]`)
      if (row === null) return
      pendingHash.current = undefined
      // Chunk commits preserved the old view. Actual target navigation now takes ownership.
      revealRow(pending.rowId)
      if (pending.generation === hashGeneration.current && pending.hash === window.location.hash && row.isConnected) row.scrollIntoView({ block: 'start' })
    }
    scrollPendingHash()
    const hash = () => {
      const generation = ++hashGeneration.current
      const previous = pendingHash.current
      pendingHash.current = undefined
      const currentHash = window.location.hash
      appliedHash.current = currentHash
      if (previous !== undefined && rowIds[startIndex] !== undefined) revealRow(rowIds[startIndex]!, true)
      if (element.closest('[aria-hidden="true"], [hidden]') !== null) return
      let id: string
      try { id = decodeURIComponent(currentHash.slice(1)) } catch { return }
      if (id === '') return
      const turn = turns.find(turn => turn.id === id || turn.prompt?.id === id || turn.items.some(item => item.id === id))
      // A turn link reveals from the turn's first source row; an item link from the item itself.
      const rowId = turn === undefined ? undefined : turn.id === id ? turn.prompt?.id ?? turn.items[0]?.id : id
      if (rowId === undefined) return
      pendingHash.current = { id, rowId, hash: currentHash, generation }
      // Preparing a cancellable link does not navigate until its current target is mounted.
      revealRow(rowId, true)
      scrollPendingHash()
    }
    const find = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f' && startIndex > 0 && element.closest('[aria-hidden="true"], [hidden]') === null) {
        // Do not navigate or detach following merely because the reader opens (or cancels) Find.
        revealRow(rowIds[0]!, true)
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
  }, [turns, rowIds, startIndex, revealRow])
  return <div {...props} ref={root}>{children}</div>
}
