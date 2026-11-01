// The sidebar's one-row tab bar (Agents, Alerts, Missions, Fleet) over the selected tab's content.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

/** Shared status glyphs: working ●, idle ○, ended ×, fault !, pending ~. */
export type SidebarTabGlyph = '●' | '○' | '×' | '!' | '~'

/** One sidebar tab. Counts are the tab's items carrying that attention; absent or 0 shows nothing. */
export interface SidebarTab {
  readonly id: string
  readonly label: string
  /** Items asking an open question (?). The strongest marker: it wins over needs-you and status. */
  readonly questionCount?: number
  /** Items that need the person (◆). Wins over status. */
  readonly needsYouCount?: number
  /** Shown only without a question or needs-you, e.g. Fleet's ! for an unreachable machine with work; `label` is spoken after the tab label. */
  readonly status?: { readonly glyph: SidebarTabGlyph; readonly label: string }
}

export interface SidebarTabsProps {
  /** Tabs in bar order. */
  readonly tabs: readonly SidebarTab[]
  readonly selectedId: string
  readonly onSelectionChange: (id: string) => void
  /** The selected tab's content; it fills the remaining height, so a scroller inside stays bounded. */
  readonly children: React.ReactNode
}

interface Marker {
  readonly kind: 'question' | 'needs-you' | 'status'
  readonly glyph: string
  /** Shown after the glyph when above 1; status has none. */
  readonly count: number
  readonly words: string
}

/** The tab's strongest marker: an open question beats needs-you, which beats the status glyph. */
function markerOf(tab: SidebarTab): Marker | null {
  const questions = tab.questionCount ?? 0, needsYou = tab.needsYouCount ?? 0
  if (questions > 0) return { kind: 'question', glyph: '?', count: questions, words: `${questions} ${questions === 1 ? 'open question' : 'open questions'}` }
  if (needsYou > 0) return { kind: 'needs-you', glyph: '◆', count: needsYou, words: `${needsYou} ${needsYou === 1 ? 'needs you' : 'need you'}` }
  if (tab.status !== undefined) return { kind: 'status', glyph: tab.status.glyph, count: 0, words: tab.status.label }
  return null
}

/** Accessible tab name: the label, then the marker in words ("Alerts, 3 need you"). */
export function sidebarTabName(tab: SidebarTab): string {
  const marker = markerOf(tab)
  return marker === null ? tab.label : `${tab.label}, ${marker.words}`
}

/** Sub-pixel slack: rects are fractional, and a row that fits by rounding must not flip counts in and out. */
const fitSlack = 0.5

function TabFace({ tab, showCount }: { readonly tab: SidebarTab; readonly showCount: boolean }) {
  const marker = markerOf(tab)
  // One text run, so the gap between marker and label is exactly one space: "◆3 Alerts", "? Missions", "! Fleet", "Agents".
  return <span {...stylex.props(styles.face)}>
    {marker === null ? null : <><span aria-hidden="true" data-sidebar-tab-marker={marker.kind} {...stylex.props(styles.marker, marker.kind === 'question' ? styles.question : marker.kind === 'needs-you' ? styles.needsYou : marker.glyph === '!' ? styles.fault : styles.status)}>{marker.glyph}{showCount && marker.count > 1 ? <span data-sidebar-tab-count>{marker.count}</span> : null}</span>{' '}</>}
    <span data-sidebar-tab-label>{tab.label}</span>
  </span>
}

/**
 * Controlled sidebar tabs (React Aria): ←/→ move between tabs, Home/End to the ends. Every tab spans the full row and labels
 * never truncate; when the row cannot hold every count, all counts drop together and the markers stay.
 */
export function SidebarTabs({ tabs, selectedId, onSelectionChange, children }: SidebarTabsProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const [countsFit, setCountsFit] = React.useState(true)
  const bar = React.useRef<HTMLDivElement>(null)
  const replica = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    const host = bar.current, row = replica.current
    if (host === null || row === null) return
    const fit = () => setCountsFit(row.getBoundingClientRect().width <= host.getBoundingClientRect().width + fitSlack)
    fit()
    const observer = new ResizeObserver(fit)
    observer.observe(host)
    observer.observe(row)
    return () => observer.disconnect()
  }, [])
  return <div ref={bar} data-sidebar-tabs data-sidebar-tab-counts={countsFit ? 'shown' : 'dropped'} {...stylex.props(styles.root)}>
    {/* Hidden replica of the row with every count, at intrinsic width; outside Tabs, whose collection pass renders its content twice. */}
    <div ref={replica} aria-hidden inert data-sidebar-tabs-measure {...stylex.props(styles.bar, narrow && styles.touch, styles.measure)}>
      {tabs.map(tab => <span key={tab.id} {...stylex.props(styles.tab, narrow && styles.touchTab)}><TabFace tab={tab} showCount /></span>)}
    </div>
    <Tabs selectedKey={selectedId} onSelectionChange={key => onSelectionChange(String(key))} {...stylex.props(styles.root)}>
      <TabList aria-label="Sidebar" {...stylex.props(styles.bar, narrow && styles.touch)}>
        {tabs.map(tab => <Tab key={tab.id} id={tab.id} data-sidebar-tab={tab.id} aria-label={sidebarTabName(tab)} {...stylex.props(styles.tab, narrow && styles.touchTab)}><TabFace tab={tab} showCount={countsFit} /></Tab>)}
      </TabList>
      {/* One panel per tab with a stable id, so every tab's aria-controls resolves; only the selected one holds content. */}
      {tabs.map(tab => <TabPanel key={tab.id} id={tab.id} data-sidebar-tab-panel={tab.id} {...stylex.props(styles.panel)}>{tab.id === selectedId ? children : null}</TabPanel>)}
    </Tabs>
  </div>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } as const
const styles = stylex.create({
  root: { position: 'relative', display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  bar: { display: 'grid', gridAutoFlow: 'column', gridAutoColumns: 'minmax(0, 1fr)', alignItems: 'center', gap: 0, flexShrink: 0, boxSizing: 'border-box', height: g.controlLg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  // Same box as the bar but sized by its content and taking no layout space, so measuring never changes what is measured.
  measure: { position: 'absolute', top: 0, insetInlineStart: 0, width: 'max-content', visibility: 'hidden', pointerEvents: 'none' },
  // Equal tracks keep tab positions stable when counts change; labels never shrink.
  tab: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', height: g.controlSm, borderRadius: r.control, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':is([data-focus-visible])': focusRing },
  touch: { height: g.touchTarget },
  touchTab: { height: g.touchTarget, minWidth: g.touchTarget, boxSizing: 'border-box' },
  face: { whiteSpace: 'nowrap' },
  marker: { fontWeight: t.weightSemibold, fontVariantNumeric: 'tabular-nums' },
  question: { color: accent.primary },
  needsYou: { color: tone.attention },
  status: { color: ink.fgMuted },
  fault: { color: tone.dangerFg },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none', ':is([data-focus-visible])': focusRing },
})
