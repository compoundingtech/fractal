import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { spaceVars as s, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/**
 * Fits run in batches, one per animation frame and before that frame's paint: every style write,
 * then every read, then every write. A sidebar mounts all its rows in one commit, and a row that
 * measured itself at once would force a full layout per row.
 */
const batch = <T,>(run: (items: readonly T[]) => void) => {
  const pending = new Set<T>()
  let frame: number | undefined
  const flush = () => {
    frame = undefined
    const items = [...pending]
    pending.clear()
    run(items)
  }
  return {
    schedule: (item: T) => { pending.add(item); frame ??= requestAnimationFrame(flush) },
    cancel: (item: T) => { pending.delete(item) },
  }
}

const sidebarScope = (row: HTMLElement) =>
  row.closest<HTMLElement>('[data-testid="agent-sidebar"]') ?? row.closest<HTMLElement>('[data-frame-rows]') ?? row.closest<HTMLElement>('[data-explore-variant]') ?? row.parentElement

/** Reads the widest metric and trailing-signal content in a scope; the caller writes the tracks after every read. */
const scopeTracks = (scope: HTMLElement) => {
  let widest = 0
  let widestSignals = 0
  for (const peer of scope.querySelectorAll<HTMLElement>('[data-testid="taste-agent-row"]')) {
    const natural = Number.parseFloat(peer.dataset.rowTimeNatural ?? '')
    if (Number.isFinite(natural)) widest = Math.max(widest, natural)
    const trailing = peer.querySelector<HTMLElement>('[data-row-trailing-signals]')
    if (trailing !== null) {
      const gap = Number.parseFloat(getComputedStyle(trailing).columnGap) || 0
      let width = 0, count = 0
      for (const signal of trailing.children) {
        const naturalWidth = signal.getBoundingClientRect().width
        if (naturalWidth > 0) { width += naturalWidth + (count > 0 ? gap : 0); count++ }
      }
      widestSignals = Math.max(widestSignals, width)
    }
  }
  return { widest, widestSignals }
}

interface Line1Row {
  readonly row: HTMLElement
  readonly changes: MutationObserver
}

interface Line1Fit {
  readonly row: HTMLElement
  readonly time: HTMLElement
  readonly title: HTMLElement
  readonly drops: readonly HTMLElement[]
  floor: number
  natural: number
  dropped: number
}

const timeNatural = (time: HTMLElement) => {
  const content = time.firstElementChild as HTMLElement | null
  return content === null ? 0 : Math.max(content.scrollWidth, content.getBoundingClientRect().width)
}

const applyTracks = (fits: readonly Line1Fit[]) => {
  for (const fit of fits) fit.row.dataset.rowTimeNatural = String(fit.natural)
  const scopes = new Set<HTMLElement>()
  for (const fit of fits) { const scope = sidebarScope(fit.row); if (scope !== null) scopes.add(scope) }
  const tracks = [...scopes].map(scope => ({ scope, ...scopeTracks(scope) }))
  for (const { scope, widest, widestSignals } of tracks) {
    if (widest > 0) scope.style.setProperty('--sidebar-metric-track', `${Math.ceil(widest)}px`)
    if (widestSignals > 0) scope.style.setProperty('--sidebar-signal-track', `${Math.ceil(widestSignals)}px`)
    else scope.style.removeProperty('--sidebar-signal-track')
  }
}

const overflows = ({ row, time }: Line1Fit) => {
  const content = time.firstElementChild
  if (content === null) return false
  const bounds = content.getBoundingClientRect()
  if (bounds.width > time.clientWidth + 0.5) return true
  const count = row.querySelector<HTMLElement>('[data-row-column="children"] [data-line1-drop="subs"]')
  if (count === null || count.getBoundingClientRect().width === 0) return false
  const gap = Number.parseFloat(getComputedStyle(row).columnGap) || 0
  return count.getBoundingClientRect().right + gap > bounds.left + 0.5
}

const line1Drops = ['[data-row-column="time"] [data-line1-drop="tokens"]', '[data-row-column="children"] [data-line1-drop="subs"]', '[data-row-column="time"] [data-line1-drop="scope"]']

const line1 = batch<Line1Row>(rows => {
  const fits: Line1Fit[] = []
  for (const { row } of rows) {
    if (!row.isConnected) continue
    const title = row.querySelector<HTMLElement>('[data-row-column="title-text"]')
    const time = row.querySelector<HTMLElement>('[data-row-column="time"]')
    if (title === null || time === null) continue
    const drops = line1Drops.map(selector => row.querySelector<HTMLElement>(selector)).filter((drop): drop is HTMLElement => drop !== null)
    for (const drop of drops) drop.style.display = ''
    fits.push({ row, time, title, drops, floor: 0, natural: 0, dropped: 0 })
  }
  for (const fit of fits) { fit.natural = timeNatural(fit.time); fit.floor = fit.row.clientWidth * 0.6 }
  applyTracks(fits)
  // Drop token and subagent counts, then scope text, a round at a time across the whole batch.
  for (let round = 0; round < line1Drops.length; round++) {
    const crowded = fits.filter(fit => fit.dropped < fit.drops.length && (fit.title.clientWidth < fit.floor || overflows(fit)))
    if (crowded.length === 0) break
    for (const fit of crowded) fit.drops[fit.dropped++]!.style.display = 'none'
    for (const fit of crowded) fit.natural = timeNatural(fit.time)
    applyTracks(crowded)
  }
  // Restoring and dropping our own fields must not schedule another fit.
  for (const { changes } of rows) changes.takeRecords()
})

/** Line 1 keeps a truncated title at least sixty percent of the row. Metric and trailing-signal tracks share their widest intrinsic content width per cohort, without measuring the already allocated grid track. Drop token and subagent counts before scope text; preserve the chevron and the distinct unread and needs marks with at least six pixels after the time. */
export function attachSidebarLine1Fit(row: HTMLElement | null) {
  if (row === null) return
  const schedule = () => line1.schedule(entry)
  const resize = new ResizeObserver(schedule)
  const changes = new MutationObserver(schedule)
  const entry: Line1Row = { row, changes }
  resize.observe(row)
  for (const element of row.querySelectorAll('[data-row-column="title-text"], [data-row-column="time"]')) resize.observe(element)
  changes.observe(row, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ['style'] })
  schedule()
  document.fonts?.ready.then(schedule)
  requestAnimationFrame(() => requestAnimationFrame(schedule))
  return () => { resize.disconnect(); changes.disconnect(); line1.cancel(entry) }
}

interface SignalsFit {
  readonly element: HTMLElement
  fields: HTMLElement[]
  gap: number
  available: number
  readonly widths: Map<HTMLElement, number>
  readonly minimum: Map<HTMLElement, number>
}

const signals = batch<HTMLElement>(elements => {
  const fits: SignalsFit[] = elements.filter(element => element.isConnected).map(element => ({ element, fields: [...element.children] as HTMLElement[], gap: 0, available: 0, widths: new Map(), minimum: new Map() }))
  for (const { fields } of fits) for (const field of fields) {
    field.style.position = 'absolute'
    field.style.visibility = 'hidden'
    field.style.maxWidth = 'none'
    field.style.width = 'max-content'
    field.style.flexShrink = '0'
  }
  for (const fit of fits) {
    fit.gap = Number.parseFloat(getComputedStyle(fit.element).columnGap) || 0
    fit.available = fit.element.clientWidth
    for (const field of fit.fields) {
      fit.widths.set(field, field.getBoundingClientRect().width)
      // Only free-text current work may shorten, with twelve measured characters retained.
      const walker = document.createTreeWalker(field, NodeFilter.SHOW_TEXT)
      const range = document.createRange()
      let remaining = 12, width = 0
      for (let node = walker.nextNode(); node && remaining > 0; node = walker.nextNode()) {
        const count = Math.min(remaining, node.textContent?.length ?? 0)
        if (!count) continue
        range.setStart(node, 0); range.setEnd(node, count)
        width += range.getBoundingClientRect().width
        remaining -= count
      }
      fit.minimum.set(field, width + (field.textContent && field.textContent.length > 12 ? Number.parseFloat(getComputedStyle(field).fontSize) : 0))
    }
  }
  const retention = (field: HTMLElement) => {
    const value = field.firstElementChild?.getAttribute('data-row-retention')
    const rank = value === null || value === '' ? Number.NaN : Number(value)
    return Number.isFinite(rank) ? rank : 0
  }
  for (const { fields, gap, widths, minimum, available: width } of fits) {
    const order = fields.map((field, index) => ({ field, index })).sort((a, b) => retention(b.field) - retention(a.field) || b.index - a.index)
    let available = width, kept = 0, shortened = false
    for (const { field } of order) {
      const space = available - (kept ? gap : 0)
      const complete = widths.get(field)! <= space + 0.5
      const keep = !shortened && (complete || (field.querySelector('[data-row-field="current-work"]') !== null && minimum.get(field)! <= space))
      field.dataset.rowDropped = String(!keep)
      field.dataset.rowShortened = String(keep && !complete)
      field.setAttribute('aria-hidden', String(!keep))
      if (!keep) continue
      field.style.position = 'static'
      field.style.visibility = 'visible'
      field.style.maxWidth = `${Math.max(0, space)}px`
      available = space - widths.get(field)!
      kept++
      if (!complete) shortened = true
    }
  }
})

export function SidebarRowSignals({ children }: { readonly children: React.ReactNode }) {
  const attach = React.useCallback((element: HTMLSpanElement | null) => {
    if (!element) return
    const schedule = () => signals.schedule(element)
    const resize = new ResizeObserver(schedule)
    const changes = new MutationObserver(schedule)
    resize.observe(element)
    changes.observe(element, { childList: true, characterData: true, subtree: true })
    schedule()
    return () => { resize.disconnect(); changes.disconnect(); signals.cancel(element) }
  }, [])
  return <span ref={attach} data-row-column="subtitle" data-row-signals="measured" {...stylex.props(styles.signals)}>{React.Children.toArray(children).map((child, index) => <span key={index} data-row-priority={index} {...stylex.props(styles.field)}>{child}</span>)}</span>
}
const styles = stylex.create({
  // A painted sibling of the row button spanning the button's columns; pointer input passes through to the button beneath.
  signals: { position: 'relative', gridColumn: '2 / -1', pointerEvents: 'none', gridRow: '2', display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, overflow: 'hidden', color: ink.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  field: { display: 'block', flexShrink: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
})
