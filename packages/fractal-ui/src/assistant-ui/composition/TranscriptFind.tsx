import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { borderVars as border, elevationVars as elevation, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from './Icons'
import type { TranscriptRevealStore } from './transcript-reveal'
import { findMatches, nearestMatch, normalizeQuery, searchEntries, searchMatchKey, stepMatch, type SearchableTurn, type SearchMatch } from './transcript-search-index'
import { rangesIn } from './transcript-search-text.ts'

type History = { readonly _tag: 'Complete' } | { readonly _tag: 'HasOlder'; readonly onLoadEarlier?: () => void; readonly older?: { readonly _tag: 'Loading' | 'Failed' } }
/** What a conversation keeps across tab switches when one transcript instance serves several tabs. */
export interface TranscriptFindState { readonly open: boolean; readonly query: string; readonly activeKey?: string }
export type TranscriptFindMemory = Map<string, TranscriptFindState>

interface Candidate {
  readonly frame: () => HTMLElement | null
  readonly usable: () => boolean
  readonly isOpen: () => boolean
  readonly open: () => void
  readonly close: () => void
}
const candidates = new Set<Candidate>()
/** How far up from focus this transcript's pane is shared; the composer's own conversation always shares the most. */
const proximity = (focus: Element | null, frame: HTMLElement) => {
  let depth = 0
  for (let node = focus; node !== null; node = node.parentElement) {
    if (node.contains(frame)) { for (let up = node; up.parentElement !== null; up = up.parentElement) depth++; return depth }
  }
  return -1
}
const nearest = (focus: Element | null) => {
  let best: Candidate | undefined, bestDepth = -2
  for (const candidate of candidates) {
    const frame = candidate.frame()
    if (frame === null || !candidate.usable()) continue
    const depth = frame.contains(focus) ? Number.MAX_SAFE_INTEGER : proximity(focus, frame)
    if (depth > bestDepth) { best = candidate; bestDepth = depth }
  }
  return best
}
const onKeyDown = (event: KeyboardEvent) => {
  const focus = document.activeElement
  if (event.key === 'Escape') {
    const candidate = nearest(focus)
    if (candidate?.isOpen() && candidate.frame()?.contains(focus)) { event.preventDefault(); candidate.close() }
    return
  }
  if (!(event.ctrlKey || event.metaKey) || event.shiftKey || event.altKey || event.key.toLowerCase() !== 'f') return
  // A modal or a focused terminal owns the keyboard; a second press while search is open is the way to browser Find.
  if (focus?.closest('[aria-modal="true"], [role="dialog"], [aria-roledescription="terminal"], [data-testid="terminal-pane"]') != null) return
  const candidate = nearest(focus)
  if (candidate === undefined || candidate.isOpen()) return
  event.preventDefault()
  candidate.open()
}
const register = (candidate: Candidate) => {
  if (candidates.size === 0) window.addEventListener('keydown', onKeyDown, true)
  candidates.add(candidate)
  return () => {
    candidates.delete(candidate)
    if (candidates.size === 0) window.removeEventListener('keydown', onKeyDown, true)
  }
}

const highlightsSupported = () => typeof CSS !== 'undefined' && 'highlights' in CSS && typeof Highlight !== 'undefined'
/** One document-wide pair named by the rules in tokens.css; each open search adds and removes only its own ranges, so panes never clobber each other. */
let sharedHighlights: { readonly match: Highlight; readonly current: Highlight } | undefined
const highlights = () => {
  if (sharedHighlights === undefined) {
    sharedHighlights = { match: new Highlight(), current: new Highlight() }
    CSS.highlights.set('transcript-search-match', sharedHighlights.match)
    CSS.highlights.set('transcript-search-current', sharedHighlights.current)
  }
  return sharedHighlights
}
const entrySelector = (id: string) => { const escaped = CSS.escape(id); return `[data-item-id="${escaped}"], [data-scroll-anchor-id="${escaped}"], [data-conversation-entry-id="${escaped}"]` }

/** Transcript search. Reads source rows, so the viewport's window never narrows it; history the host has not loaded is the only gap it reports. */
export function TranscriptFind({ scrollId, turns, rowIds, revealRow, history, active, reveal, memory, memoryKey }: {
  readonly scrollId: string
  readonly turns: readonly SearchableTurn[]
  readonly rowIds: readonly string[]
  readonly revealRow: (id: string, preservePosition?: boolean) => void
  readonly history: History
  readonly active: boolean
  readonly reveal: TranscriptRevealStore
  readonly memory: TranscriptFindMemory
  readonly memoryKey: string
}) {
  const remembered = memory.get(memoryKey)
  const [open, setOpen] = React.useState(remembered?.open ?? false)
  const [query, setQuery] = React.useState(remembered?.query ?? '')
  const [activeKey, setActiveKey] = React.useState(remembered?.activeKey)
  const [paging, setPaging] = React.useState(false)
  const [pagedOlder, setPagedOlder] = React.useState(false)
  const [immediate, setImmediate] = React.useState(false)
  const box = React.useRef<HTMLDivElement>(null)
  const input = React.useRef<HTMLInputElement>(null)
  const returnFocus = React.useRef<HTMLElement | null>(null)
  const origin = React.useRef(0)
  const pending = React.useRef<{ readonly match: SearchMatch; mounted: boolean } | undefined>(undefined)
  const lane = () => document.getElementById(scrollId)

  const entries = React.useMemo(() => open ? searchEntries(turns) : [], [open, turns])
  const deferredQuery = React.useDeferredValue(query)
  const needle = normalizeQuery(deferredQuery)
  const matches = React.useMemo(() => findMatches(entries, deferredQuery), [entries, deferredQuery])
  const rowIndex = React.useMemo(() => new Map(rowIds.map((id, index) => [id, index])), [rowIds])
  const pinned = activeKey === undefined ? -1 : matches.findIndex(match => searchMatchKey(match) === activeKey)
  const index = pinned !== -1 ? pinned : nearestMatch(matches, rowIndex, origin.current)
  const current = index === -1 ? undefined : matches[index]
  const currentKey = current === undefined ? undefined : searchMatchKey(current)
  const navigationKey = current === undefined ? undefined : `${needle}\u0000${currentKey}\u0000${current.deep}`
  // Restoring a tab restores its disclosures and paint without taking the reader's scroll.
  const navigated = React.useRef(remembered?.activeKey === currentKey ? navigationKey : undefined)
  const usable = open && active

  React.useEffect(() => { memory.set(memoryKey, { open, query, ...(currentKey === undefined ? {} : { activeKey: currentKey }) }) }, [memory, memoryKey, open, query, currentKey])

  const close = React.useCallback(() => {
    setOpen(false)
    setPaging(false)
    pending.current = undefined
    const target = returnFocus.current?.isConnected ? returnFocus.current : document.getElementById(scrollId)
    target?.focus({ preventScroll: true })
    returnFocus.current = null
  }, [scrollId])
  const state = React.useRef({ open, active, hasRows: rowIds.length > 0 })
  state.current = { open, active, hasRows: rowIds.length > 0 }
  React.useEffect(() => register({
    frame: () => document.getElementById(scrollId)?.parentElement ?? null,
    usable: () => {
      const element = document.getElementById(scrollId)
      return state.current.active && state.current.hasRows && element !== null && element.closest('[aria-hidden="true"], [hidden]') === null
    },
    isOpen: () => state.current.open,
    open: () => {
      const focused = document.activeElement
      returnFocus.current = focused instanceof HTMLElement && focused !== document.body ? focused : document.getElementById(scrollId)
      // A fresh query lands near where the reader already is.
      const element = document.getElementById(scrollId)
      if (element !== null) {
        const top = element.getBoundingClientRect().top
        const row = [...element.querySelectorAll<HTMLElement>('[data-testid="transcript-turn"]')].find(turn => turn.getBoundingClientRect().bottom > top)
        const first = row?.dataset.sourceIds?.split(' ')[0]
        if (first !== undefined) origin.current = rowIds.indexOf(first)
      }
      setActiveKey(undefined)
      setOpen(true)
    },
    close,
  }), [close, rowIds, scrollId])
  React.useLayoutEffect(() => {
    if (!open || returnFocus.current === null) return
    input.current?.focus({ preventScroll: true })
    input.current?.select()
  }, [open])

  // Typing waits a beat, so passing through a query does not open and close rows; stepping acts at once.
  React.useEffect(() => {
    pending.current = undefined
    if (!usable || current === undefined) {
      reveal.set({ deep: false })
      if (current === undefined) navigated.current = undefined
      return
    }
    const go = () => {
      // Pinned by identity from here on, so older pages prepending never move it.
      setActiveKey(searchMatchKey(current))
      reveal.set({ entryId: current.entryId, deep: current.deep })
      if (navigated.current === navigationKey) return
      navigated.current = navigationKey
      pending.current = { match: current, mounted: false }
      revealRowRef.current(current.entryId, true)
      paint.current()
    }
    const timer = immediate ? undefined : setTimeout(go, 120)
    if (immediate) go()
    return () => {
      clearTimeout(timer)
      pending.current = undefined
    }
    // A new query or disclosure depth is a new navigation even at the same occurrence.
    // Streaming updates to `revealRow` must not re-scroll the reader.
  }, [usable, navigationKey, reveal])
  React.useEffect(() => () => reveal.set({ deep: false }), [reveal])

  const clearPainted = React.useRef<() => void>(() => undefined)
  const revealRowRef = React.useRef(revealRow)
  revealRowRef.current = revealRow
  const paint = React.useRef<() => void>(() => undefined)
  paint.current = () => {
    const element = lane()
    if (element === null) return
    const all: Range[] = []
    let currentRange: Range | undefined
    const ids = new Set(matches.map(match => match.entryId))
    if (usable && needle !== '') for (const id of ids) {
      const row = element.querySelector(entrySelector(id))
      if (row === null) continue
      const ranges = rangesIn(row, needle)
      all.push(...ranges)
      if (id === current?.entryId) currentRange = ranges[current.occurrence]
    }
    clearPainted.current()
    if (highlightsSupported()) {
      const { match, current: currentHighlight } = highlights()
      for (const range of all) if (range !== currentRange) match.add(range)
      if (currentRange !== undefined) currentHighlight.add(currentRange)
      clearPainted.current = () => {
        for (const range of all) { match.delete(range); currentHighlight.delete(range) }
        clearPainted.current = () => undefined
      }
    }
    box.current?.setAttribute('data-highlighted', String(all.length))
    const target = pending.current
    if (target === undefined) return
    // The mutation observer follows the virtualizer until the row actually mounts.
    const row = element.querySelector<HTMLElement>(entrySelector(target.match.entryId))
    if (row === null) return
    // The row is mounted: navigation now owns the lane, as a followed message link does.
    if (!target.mounted) {
      target.mounted = true
      revealRowRef.current(target.match.entryId)
    }
    const bounds = element.getBoundingClientRect(), inset = box.current?.getBoundingClientRect().bottom ?? bounds.top
    const rect = (currentRange ?? row).getBoundingClientRect()
    if (rect.top < inset || rect.bottom > bounds.bottom) element.scrollTop += rect.top - bounds.top - bounds.height / 3
    if (currentRange !== undefined) pending.current = undefined
  }
  React.useEffect(() => {
    const element = lane()
    if (!usable || element === null) return
    let frame: number | undefined
    const schedule = () => { frame ??= requestAnimationFrame(() => { frame = undefined; paint.current() }) }
    const observer = typeof MutationObserver === 'undefined' ? undefined : new MutationObserver(schedule)
    observer?.observe(element, { childList: true, subtree: true, characterData: true })
    const cancelNavigation = () => { pending.current = undefined }
    const cancelKey = (event: KeyboardEvent) => {
      if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End'].includes(event.key)) cancelNavigation()
    }
    element.addEventListener('wheel', cancelNavigation, { passive: true })
    element.addEventListener('touchstart', cancelNavigation, { passive: true })
    element.addEventListener('pointerdown', cancelNavigation)
    element.addEventListener('keydown', cancelKey)
    schedule()
    return () => {
      observer?.disconnect()
      element.removeEventListener('wheel', cancelNavigation)
      element.removeEventListener('touchstart', cancelNavigation)
      element.removeEventListener('pointerdown', cancelNavigation)
      element.removeEventListener('keydown', cancelKey)
      if (frame !== undefined) cancelAnimationFrame(frame)
    }
  }, [usable, scrollId])
  React.useEffect(() => { if (usable) paint.current() }, [usable, needle, currentKey, matches])
  React.useEffect(() => { if (!usable) clearPainted.current() }, [usable])
  React.useEffect(() => () => clearPainted.current(), [])

  const older = history._tag === 'HasOlder' ? history : undefined
  const loadOlder = older?.onLoadEarlier
  const requestedAt = React.useRef(-1)
  React.useEffect(() => {
    if (!paging || !active) return
    if (older === undefined || loadOlder === undefined) { setPaging(false); return }
    if (older.older?._tag === 'Loading') return
    // A failure ends the run until Retry starts a fresh one; a page that added nothing ends it rather than asking forever.
    if (requestedAt.current === rowIds.length || (older.older?._tag === 'Failed' && requestedAt.current !== -1)) { setPaging(false); return }
    requestedAt.current = rowIds.length
    loadOlder()
  }, [paging, active, older, loadOlder, rowIds.length])
  const startPaging = () => { requestedAt.current = -1; setPagedOlder(true); setPaging(true) }

  const step = (direction: 1 | -1) => {
    const next = stepMatch(index, matches.length, direction)
    if (next === -1) return
    setImmediate(true)
    setActiveKey(searchMatchKey(matches[next]!))
  }
  if (!open) return null

  const failed = older?.older?._tag === 'Failed'
  const searched = `${entries.length} ${entries.length === 1 ? 'entry' : 'entries'}`
  const coverage = older === undefined
    ? pagedOlder ? `Full history searched · ${searched}` : undefined
    : failed ? `Couldn't load older history · ${searched} searched`
      : paging || older.older?._tag === 'Loading' ? `Loading older history · ${searched} searched`
        : loadOlder === undefined ? `Loaded entries only · ${searched} · older history isn't available here`
          : `Loaded entries only · ${searched}`
  const counted = needle === '' ? '' : matches.length === 0 ? 'No matches' : `${index + 1}/${matches.length}`
  const announcement = needle === '' ? '' : `${matches.length === 0 ? 'No matches' : `Match ${index + 1} of ${matches.length}`}. ${coverage ?? `All ${searched} searched`}.`
  return <div {...stylex.props(styles.anchor)}>
    <div ref={box} role="search" aria-label="Search transcript" data-testid="transcript-find" data-match-count={matches.length} data-coverage={older === undefined ? 'complete' : 'partial'} {...stylex.props(styles.box)}>
      <div {...stylex.props(styles.row)}>
        <input ref={input} type="text" value={query} aria-label="Search transcript" placeholder="Search transcript" spellCheck={false} autoComplete="off" onChange={event => { setImmediate(false); setActiveKey(undefined); setQuery(event.target.value) }} onKeyDown={event => {
          if (event.key !== 'Enter' || event.nativeEvent.isComposing) return
          event.preventDefault()
          step(event.shiftKey ? -1 : 1)
        }} {...stylex.props(styles.input)} />
        <span aria-hidden="true" data-testid="transcript-find-count" {...stylex.props(styles.count, needle !== '' && matches.length === 0 && styles.none)}>{counted}</span>
        <Button aria-label="Previous match" isDisabled={matches.length === 0} onPress={() => step(-1)} {...stylex.props(styles.icon)}><span {...stylex.props(styles.flip)}><Icon name="chevron-down" size={12} /></span></Button>
        <Button aria-label="Next match" isDisabled={matches.length === 0} onPress={() => step(1)} {...stylex.props(styles.icon)}><Icon name="chevron-down" size={12} /></Button>
        <Button aria-label="Close search" onPress={close} {...stylex.props(styles.icon)}><Icon name="x" size={12} /></Button>
      </div>
      {coverage !== undefined && <div data-testid="transcript-find-coverage" {...stylex.props(styles.row, styles.coverage)}>
        <span {...stylex.props(styles.coverageText)}>{coverage}</span>
        {older !== undefined && loadOlder !== undefined && (paging
          ? <Button onPress={() => setPaging(false)} {...stylex.props(styles.action)}>Stop</Button>
          : older.older?._tag !== 'Loading' && <Button onPress={startPaging} {...stylex.props(styles.action)}>{failed ? 'Retry' : 'Search full history'}</Button>)}
      </div>}
      <span role="status" aria-live="polite" data-testid="transcript-find-status" {...stylex.props(styles.visuallyHidden)}>{announcement}</span>
    </div>
  </div>
}

const styles = stylex.create({
  // Takes no height, so opening search never moves the lane under the reader.
  anchor: { position: 'relative', height: 0, flexShrink: 0, zIndex: 2 },
  box: { position: 'absolute', top: s.md, right: s.lg, width: `min(${g.tooltipMax}, calc(100% - 2 * ${s.lg}))`, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: s.xs, padding: s.xs, backgroundColor: surface.raised, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, boxShadow: elevation.popover, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  row: { display: 'flex', alignItems: 'center', gap: s.xs, minWidth: 0 },
  input: { flex: '1 1 0', minWidth: 0, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, outlineWidth: 0, '::placeholder': { color: ink.fgMuted } },
  count: { flexShrink: 0, color: ink.fgMuted, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  none: { color: ink.fgSoft },
  icon: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.controlSm, height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':disabled': { cursor: 'default', opacity: 0.5 }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  flip: { display: 'inline-flex', transform: 'rotate(180deg)' },
  coverage: { justifyContent: 'space-between', paddingInlineStart: s.sm, color: ink.fgMuted },
  coverageText: { minWidth: 0, overflowWrap: 'anywhere' },
  action: { flexShrink: 0, minHeight: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  visuallyHidden: { position: 'absolute', width: 1, height: 1, padding: 0, margin: -1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
})
