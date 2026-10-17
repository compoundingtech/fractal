import * as React from 'react'
import { flushSync } from 'react-dom'
import type { TranscriptTurn } from './Transcript'

// React treats `hidden` as boolean; the native until-found value must be set verbatim.
const searchable = (element: HTMLDivElement | null) => { element?.setAttribute('hidden', 'until-found') }

/** Native find keeps a cheap text index; only a matched turn mounts its rich transcript history. */
export function TranscriptSearch({ turns, startIndex, revealRow, children, ...props }: React.HTMLAttributes<HTMLDivElement> & { readonly turns: readonly TranscriptTurn[]; readonly startIndex: number; readonly revealRow: (id: string) => void }) {
  const root = React.useRef<HTMLDivElement>(null)
  const appliedHash = React.useRef<string | undefined>(undefined)
  React.useEffect(() => {
    const element = root.current
    if (element === null) return
    const reveal = (turn: TranscriptTurn, itemId = turn.id) => {
      flushSync(() => revealRow(turn.id))
      const row = Array.from(element.querySelectorAll<HTMLElement>('[data-item-id]')).find(row => row.dataset.itemId === itemId)
      row?.scrollIntoView({ block: 'start' })
    }
    const match = (event: Event) => {
      const id = (event.target as HTMLElement).dataset.transcriptSearchId
      const turn = turns.find(turn => turn.id === id)
      if (turn !== undefined) reveal(turn)
    }
    const hash = () => {
      if (element.closest('[aria-hidden="true"], [hidden]') !== null) return
      let id: string
      try { id = decodeURIComponent(window.location.hash.slice(1)) } catch { return }
      if (id === '') return
      const turn = turns.find(turn => turn.id === id || turn.prompt?.id === id || turn.items.some(item => item.id === id))
      if (turn !== undefined) { appliedHash.current = window.location.hash; reveal(turn, id) }
    }
    const find = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f' && startIndex > 0 && element.closest('[aria-hidden="true"], [hidden]') === null) {
        // The browser's Find command must see rich Markdown text, not its source syntax.
        flushSync(() => revealRow(turns[0]!.id))
      }
    }
    element.addEventListener('beforematch', match)
    window.addEventListener('hashchange', hash)
    document.addEventListener('keydown', find, true)
    // A hash can already name an older message when the conversation opens.
    const frame = window.location.hash === '' || appliedHash.current === window.location.hash ? undefined : requestAnimationFrame(hash)
    return () => {
      element.removeEventListener('beforematch', match)
      window.removeEventListener('hashchange', hash)
      document.removeEventListener('keydown', find, true)
      if (frame !== undefined) cancelAnimationFrame(frame)
    }
  }, [turns, startIndex, revealRow])
  return <div {...props} ref={root}>
    {turns.slice(0, startIndex).map(turn => <div key={turn.id} hidden ref={searchable} data-transcript-search-id={turn.id} style={{ position: 'absolute' }}>{[...(turn.prompt === undefined ? [] : [turn.prompt]), ...turn.items].map(item => 'text' in item ? item.text : item._tag === 'Message' ? item.title : '').join('\n\n')}</div>)}
    {children}
  </div>
}
