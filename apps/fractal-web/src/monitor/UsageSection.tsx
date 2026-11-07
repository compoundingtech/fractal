// The sidebar's bottom Usage section: one worst-quota line when collapsed, worst-first rows with meters when open.
import { useAtom, useAtomValue } from '@effect/atom-react'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { ResizableSplit } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Shell.tsx'
import { colorVars as c, geometryNumbers, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource, useNow } from '../data/react.tsx'
import { usageSection } from '../state/cells/lists.ts'
import { quotaStateAtom } from './state.ts'
import { usageSectionView, type UsageLevel, type UsageRowView, type UsageSectionView } from './usageSectionView.ts'

const DEFAULT_HEIGHT = geometryNumbers.drawerDefault
const MIN_HEIGHT = geometryNumbers.drawerMin
const MAX_HEIGHT = geometryNumbers.drawerMax

const stored = (key: string): string | null => {
  try { return globalThis.localStorage?.getItem(key) ?? null } catch { return null }
}
const remember = (key: string, value: string): void => {
  try { globalThis.localStorage?.setItem(key, value) } catch { /* the choice still applies for this session */ }
}
const storedHeight = (key: string): number => {
  const value = Number(stored(key))
  return Number.isFinite(value) && value >= MIN_HEIGHT && value <= MAX_HEIGHT ? value : DEFAULT_HEIGHT
}

/** The reading as text: the at-limit `!` means the state never rests on colour. */
const Percent = ({ row, muted }: { readonly row: UsageRowView; readonly muted: boolean }) =>
  <span {...stylex.props(styles.percent, muted || row.old !== undefined ? levels.unknown : levels[row.level])}>{row.percent}{row.level === 'limit' ? '!' : ''}</span>

const Meter = ({ row, muted }: { readonly row: UsageRowView; readonly muted: boolean }) =>
  <span aria-hidden="true" data-usage-meter={row.level} {...stylex.props(styles.meter)}>
    {row.fill === undefined ? null : <span style={{ inlineSize: `${row.fill}%` }} {...stylex.props(styles.fill, muted || row.old !== undefined ? fills.old : fills[row.level])} />}
  </span>

const Row = ({ row, muted, onOpenUsage }: { readonly row: UsageRowView; readonly muted: boolean; readonly onOpenUsage: () => void }) => {
  const narrow = React.useContext(NarrowLayoutContext)
  const wordsId = React.useId()
  // The name is what the row shows, so speech input can say it; the description carries the reading in words.
  return <li>
    <span id={wordsId} hidden>{row.words}</span>
    <Aria.Button aria-describedby={wordsId} onPress={onOpenUsage} data-usage-row={row.id} data-usage-level={row.level} {...stylex.props(styles.row, narrow && styles.touch)}>
      <span {...stylex.props(styles.rowName)}>{row.name}</span>
      <Percent row={row} muted={muted} />
      <span {...stylex.props(styles.rowWindow)}>{row.window}</span>
      <Meter row={row} muted={muted} />
      <span {...stylex.props(styles.rowReset)}>{row.old ?? row.reset}{row.more === undefined ? '' : ` +${row.more.count}`}{row.more?.atLimit ? '!' : ''}</span>
    </Aria.Button>
  </li>
}

const openSummary = (view: Extract<UsageSectionView, { _tag: 'ready' }>): string => {
  const counts = view.atLimit > 0 ? `${view.atLimit} at limit` : view.running > 0 ? `${view.running} running short` : `${view.rows.length} ${view.rows.length === 1 ? 'account' : 'accounts'}`
  return view.old === undefined ? counts : `${view.old} · ${counts}`
}

/** Collapsed, the single worst account; open, the rows name it, so the header counts instead. Otherwise the state in words. */
const Summary = ({ view, open }: { readonly view: UsageSectionView; readonly open: boolean }) => {
  switch (view._tag) {
    case 'loading': return <span {...stylex.props(styles.quiet)}>Reading quota…</span>
    case 'not-configured': return <span {...stylex.props(styles.quiet)}>No quota source</span>
    case 'failed': return <span {...stylex.props(styles.quiet)}>Could not read quota</span>
    case 'empty': return <span {...stylex.props(styles.quiet)}>No accounts reported</span>
    case 'ready': return open ? <span {...stylex.props(styles.quiet)}>{openSummary(view)}</span> : <>
      <span {...stylex.props(styles.quiet)}>{(view.old ?? view.worst.old) === undefined ? view.worst.name : `${view.old ?? view.worst.old} · ${view.worst.name}`}</span>
      <Percent row={view.worst} muted={view.old !== undefined} />
    </>
  }
}

const summaryWords = (view: UsageSectionView): string => {
  switch (view._tag) {
    case 'loading': return 'Usage, reading quota'
    case 'not-configured': return 'Usage, no quota source configured'
    case 'failed': return 'Usage, could not read quota'
    case 'empty': return 'Usage, no accounts reported'
    case 'ready': return [
      `Usage, worst: ${view.worst.words}`,
      ...(view.atLimit > 0 ? [`${view.atLimit} at limit`] : []),
      ...(view.old === undefined ? [] : [`reading ${view.old}`]),
    ].join(', ')
  }
}

const FAILED: Readonly<Record<Extract<UsageSectionView, { _tag: 'failed' }>['kind'], string>> = {
  unreachable: 'The quota source can’t be reached. Trying again.',
  timeout: 'The quota source didn’t answer in time. Trying again.',
  http: 'The quota source returned an error. Trying again.',
  unavailable: 'The quota source is unavailable. Trying again.',
  incompatible: 'This Fractal can’t read the quota source’s format.',
}

const Body = ({ view, onOpenUsage }: { readonly view: UsageSectionView; readonly onOpenUsage: () => void }) => {
  switch (view._tag) {
    case 'loading': return <p role="status" {...stylex.props(styles.note)}>Reading quota…</p>
    case 'not-configured': return <p {...stylex.props(styles.note)}>No quota source configured on this Fractal.</p>
    case 'failed': return <p role="status" {...stylex.props(styles.note)}>{FAILED[view.kind]}</p>
    case 'empty': return <p {...stylex.props(styles.note)}>The quota source reports no accounts.{view.old === undefined ? '' : ` Reading ${view.old}.`}</p>
    case 'ready': return <>
      {view.old === undefined ? null : <p role="status" {...stylex.props(styles.oldNote)}>Last reading, {view.old}</p>}
      <ul {...stylex.props(styles.list)}>{view.rows.map(row => <Row key={row.id} row={row} muted={view.old !== undefined} onOpenUsage={onOpenUsage} />)}</ul>
      {view.unreported === 0 ? null : <p {...stylex.props(styles.note)}>{view.unreported} {view.unreported === 1 ? 'account' : 'accounts'} not reporting</p>}
    </>
  }
}

const clampHeight = (value: number): number => Number.isFinite(value) && value >= MIN_HEIGHT && value <= MAX_HEIGHT ? value : DEFAULT_HEIGHT

/** Where the section keeps its open state and committed height. The app passes a shared cell. */
export interface UsageSectionMemory {
  readonly open: boolean
  readonly height: number
  readonly onChange: (next: { readonly open: boolean; readonly height: number }) => void
}

/** Uncontrolled memory under one localStorage key, for stories and isolated hosts. */
const useStoredMemory = (storageKey: string, controlled: boolean): UsageSectionMemory => {
  // A controlled section never touches storage, so embeds stay off the host page's storage.
  const [state, setState] = React.useState(() => controlled ? { open: false, height: DEFAULT_HEIGHT } : { open: stored(`${storageKey}.open`) === 'true', height: storedHeight(`${storageKey}.height`) })
  return { ...state, onChange: (next) => {
    setState(next)
    remember(`${storageKey}.open`, String(next.open))
    remember(`${storageKey}.height`, String(next.height))
  } }
}

export const UsageSection = ({ view, onOpenUsage, storageKey = 'wf.sidebar.usageSection', memory }: {
  readonly view: UsageSectionView
  readonly onOpenUsage: () => void
  readonly storageKey?: string
  /** Controlled open state and height; without it the section remembers under `storageKey`. */
  readonly memory?: UsageSectionMemory
}): React.ReactNode => {
  const narrow = React.useContext(NarrowLayoutContext)
  const fallback = useStoredMemory(storageKey, memory !== undefined)
  const remembered = memory ?? fallback
  const open = remembered.open
  // Dragging previews locally every frame; only a commit reaches the memory.
  const [dragHeight, setDragHeight] = React.useState<number | undefined>(undefined)
  const height = dragHeight ?? clampHeight(remembered.height)
  const toggle = (next: boolean) => { remembered.onChange({ open: next, height: clampHeight(remembered.height) }) }
  const commitHeight = (value: number) => { setDragHeight(undefined); remembered.onChange({ open, height: clampHeight(value) }) }
  const wordsId = React.useId()
  return (
    <Aria.Disclosure isExpanded={open} onExpandedChange={toggle} data-usage-section="" {...stylex.props(styles.section)}>
      {open && !narrow
        ? <ResizableSplit id="usage-section" label="Resize Usage section" orientation="horizontal" reverse collapsed={false} value={height} min={MIN_HEIGHT} max={MAX_HEIGHT}
          onChange={setDragHeight} onCommit={commitHeight} onToggle={() => toggle(false)} onReset={() => commitHeight(DEFAULT_HEIGHT)} />
        : null}
      <div {...stylex.props(styles.header)}>
        <span id={wordsId} hidden>{summaryWords(view)}</span>
        <Aria.Button slot="trigger" aria-describedby={wordsId} {...stylex.props(styles.trigger, narrow && styles.touch)}>
          <span aria-hidden="true" {...stylex.props(styles.chevron, open && styles.chevronOpen)}><Icon name="chevron-right" /></span>
          <span {...stylex.props(styles.title)}>Usage</span>
          <Summary view={view} open={open} />
        </Aria.Button>
        <ChromeControl icon="gauge" label="Open Usage" onPress={onOpenUsage} />
      </div>
      <Aria.DisclosurePanel style={narrow ? undefined : { maxBlockSize: height }} {...stylex.props(styles.panel, narrow && styles.panelNarrow)}>
        <Body view={view} onOpenUsage={onOpenUsage} />
      </Aria.DisclosurePanel>
    </Aria.Disclosure>
  )
}

/** The section over the source's quota poll, which runs only while this is mounted and the page is visible. */
export const LiveUsageSection = ({ onOpenUsage }: { readonly onOpenUsage: () => void }): React.ReactNode => {
  const state = useAtomValue(quotaStateAtom(useDataSource().usage))
  const now = useNow()
  const view = usageSectionView({ state, now })
  // Each poll passes through in-flight, which carries no result; only a first read shows as loading.
  // A slow read doesn't overturn a daemon's answer that it has no source, which only a real answer changes.
  const settled = React.useRef<UsageSectionView>(view)
  if (view._tag !== 'loading' && !(view._tag === 'failed' && settled.current._tag === 'not-configured')) settled.current = view
  const [remembered, setRemembered] = useAtom(usageSection)
  return <UsageSection view={settled.current} onOpenUsage={onOpenUsage} memory={{ ...remembered, onChange: setRemembered }} />
}

const levels: Readonly<Record<UsageLevel, stylex.StyleXStyles>> = stylex.create({
  ok: { color: c.fgSoft },
  warn: { color: c.attention },
  high: { color: c.dangerFg },
  limit: { color: c.dangerFg, fontWeight: t.weightSemibold },
  unknown: { color: c.fgMuted },
})

const fills: Readonly<Record<UsageLevel | 'old', stylex.StyleXStyles>> = stylex.create({
  ok: { backgroundColor: c.fgSoft },
  warn: { backgroundColor: c.attention },
  high: { backgroundColor: c.dangerFg },
  limit: { backgroundColor: c.dangerFg },
  unknown: { backgroundColor: c.fgMuted },
  old: { backgroundColor: c.fgMuted, opacity: 0.5 },
})

const styles = stylex.create({
  section: { position: 'relative', flexShrink: 0, display: 'flex', flexDirection: 'column', minHeight: 0, maxHeight: '60%', borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: c.borderStrong, backgroundColor: c.sidebar, color: c.fg, fontFamily: t.fontSans },
  header: { display: 'flex', alignItems: 'center', gap: s.xs, flexShrink: 0, paddingInlineEnd: s.xs },
  trigger: { flexGrow: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: s.sm, minHeight: g.controlLg, paddingInlineStart: s.md, paddingInlineEnd: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fg, fontFamily: 'inherit', fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}`, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  touch: { minHeight: g.touchTarget },
  chevron: { display: 'inline-flex', flexShrink: 0, color: c.sidebarFgMuted, transitionProperty: 'transform', transitionDuration: '150ms', '@media (prefers-reduced-motion: reduce)': { transitionDuration: '0s' } },
  chevronOpen: { transform: 'rotate(90deg)' },
  title: { flexShrink: 0, fontWeight: t.weightMedium },
  quiet: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: c.sidebarFgMuted },
  percent: { flexShrink: 0, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', textAlign: 'end' },
  panel: { minHeight: 0, overflowY: 'auto', scrollbarWidth: 'thin' },
  panelNarrow: { maxBlockSize: '50dvh' },
  list: { margin: 0, padding: 0, paddingBlockEnd: s.xs, listStyle: 'none' },
  // Two lines at 256px: name and percentage, then window, meter and reset. Grid keeps every meter on one column.
  row: { inlineSize: '100%', display: 'grid', gridTemplateColumns: '3ch minmax(0, 1fr) auto', columnGap: s.sm, rowGap: s.xs2, alignItems: 'center', paddingBlock: s.xs, paddingInline: s.md, borderWidth: 0, backgroundColor: 'transparent', color: c.fg, fontFamily: 'inherit', fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `${g.focusRing} solid ${c.primary}`, outlineOffset: `calc(-1 * ${g.focusRing})` } },
  rowName: { gridColumn: '1 / 3', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  rowWindow: { color: c.sidebarFgMuted, fontSize: t.denseSize, whiteSpace: 'nowrap' },
  rowReset: { color: c.sidebarFgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', textAlign: 'end' },
  meter: { display: 'block', blockSize: g.progressSm, minWidth: 0, borderRadius: r.full, overflow: 'hidden', backgroundColor: c.borderStrong },
  fill: { display: 'block', blockSize: '100%', borderRadius: r.full },
  note: { margin: 0, paddingBlock: s.xs, paddingInline: s.md, color: c.sidebarFgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  oldNote: { margin: 0, paddingBlockStart: s.xs, paddingInline: s.md, color: c.attention, fontSize: t.denseSize },
})
