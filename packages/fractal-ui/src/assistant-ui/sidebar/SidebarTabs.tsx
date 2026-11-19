// The sidebar's one-row tab bar (Agents, Alerts, Missions, Fleet) over the selected tab's content.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { mergeProps, useFocusRing, useTab, useTabList } from 'react-aria'
import { Item, useTabListState, type TabListState } from 'react-stately'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon, type IconName } from '../composition/Icons'
import { NarrowLayoutContext } from '../composition/NarrowLayout'

/** Shared status glyphs: working ●, idle ○, ended ×, fault !, pending ~. */
export type SidebarTabGlyph = '●' | '○' | '×' | '!' | '~'

/** One sidebar tab. Counts are the tab's items carrying that attention; absent or 0 shows nothing. */
export interface SidebarTab {
  readonly id: string
  readonly label: string
  /** Leads the label. */
  readonly icon?: IconName
  /** The tab's items when none needs the person: a neutral pill, the first thing to drop when space is tight. */
  readonly count?: number
  /** Items asking an open question: the strongest marker, over needs-you and status. */
  readonly questionCount?: number
  /** Items that need the person. Wins over status. */
  readonly needsYouCount?: number
  /** Shown only without a question or needs-you, e.g. Fleet's fault for an unreachable machine with work; `label` is spoken after the tab label. */
  readonly status?: { readonly glyph: SidebarTabGlyph; readonly label: string }
}

export interface SidebarTabsProps {
  /** Tabs in bar order. */
  readonly tabs: readonly SidebarTab[]
  readonly selectedId: string
  readonly onSelectionChange: (id: string) => void
  /** The host guarantees a tab stop inside the selected panel, including loading and empty states. */
  readonly panelHasTabbableContent?: boolean
  /** The selected tab's content; it fills the remaining height, so a scroller inside stays bounded. */
  readonly children: React.ReactNode
}

interface Marker {
  readonly kind: 'question' | 'needs-you' | 'status' | 'count'
  /** Status has no count, so its icon says it. */
  readonly icon?: IconName
  /** Painted after the icon while counts fit; status has none. */
  readonly count: number
  readonly words: string
}

const statusIcons: Record<SidebarTabGlyph, IconName> = { '●': 'dot', '○': 'dot', '×': 'x', '!': 'alert', '~': 'clock' }

/** The tab's strongest marker: an open question beats needs-you, which beats status, which beats a neutral count. */
function markerOf(tab: SidebarTab): Marker | null {
  const questions = tab.questionCount ?? 0, needsYou = tab.needsYouCount ?? 0
  if (questions > 0) return { kind: 'question', count: questions, words: `${questions} ${questions === 1 ? 'open question' : 'open questions'}` }
  if (needsYou > 0) return { kind: 'needs-you', count: needsYou, words: `${needsYou} ${needsYou === 1 ? 'needs you' : 'need you'}` }
  if (tab.status !== undefined) return { kind: 'status', icon: statusIcons[tab.status.glyph], count: 0, words: tab.status.label }
  if ((tab.count ?? 0) > 0) return { kind: 'count', count: tab.count!, words: String(tab.count) }
  return null
}

/** Accessible tab name: the label, then the marker in words ("Alerts, 3 need you"). */
export function sidebarTabName(tab: SidebarTab): string {
  const marker = markerOf(tab)
  return marker === null ? tab.label : `${tab.label}, ${marker.words}`
}

/** Sub-pixel slack: rects are fractional, and a row that fits by rounding must not flip counts in and out. */
const fitSlack = 0.5

function TabFace({ tab, showCount, showIcon = true }: { readonly tab: SidebarTab; readonly showCount: boolean; readonly showIcon?: boolean }) {
  const marker = markerOf(tab)
  // Icon, label, then a pill. Needing the person is a filled pill and a neutral count an unfilled one, so colour is never the only cue.
  // When counts drop a filled pill stays as a dot and a neutral one goes. The count is painted, not DOM text, so the tab's only
  // visible text is its label, which its name ("Agents, 3 need you") contains.
  const pill = marker !== null && (showCount || marker.kind !== 'count')
  return <span {...stylex.props(styles.face)}>
    {tab.icon === undefined || !showIcon ? null : <span aria-hidden="true" data-sidebar-tab-icon={tab.icon} {...stylex.props(styles.icon)}><Icon name={tab.icon} size={12} /></span>}
    <span data-sidebar-tab-label>{tab.label}</span>
    {pill ? <span aria-hidden="true" data-sidebar-tab-marker={marker.kind} data-sidebar-tab-count={showCount && marker.count > 0 ? (marker.count > 99 ? '99+' : marker.count) : undefined} {...stylex.props(styles.pill, marker.icon !== undefined ? styles.iconOnly : !(showCount && marker.count > 0) && styles.dot, marker.kind === 'question' || marker.kind === 'needs-you' ? styles.attention : marker.kind === 'status' ? (tab.status?.glyph === '!' ? styles.fault : styles.neutral) : styles.neutral)}>{marker.icon === undefined ? null : <Icon name={marker.icon} size={12} />}</span> : null}
  </span>
}

/** The sidebar tab count alone, for other tab rows: 16px, a true circle around one digit, amber only when it needs the person. Painted; the tab's name carries the words. */
export function CountPill({ count, needsYou = false }: { readonly count: number; readonly needsYou?: boolean }) {
  return <span aria-hidden="true" data-sidebar-tab-marker={needsYou ? 'needs-you' : 'count'} data-sidebar-tab-count={count > 99 ? '99+' : count} {...stylex.props(styles.pill, needsYou ? styles.attention : styles.neutral)} />
}

/**
 * Controlled sidebar tabs (React Aria): ←/→ move between tabs, Home/End to the ends. Every tab spans the full row and labels
 * never truncate; when the row cannot hold every count, all counts drop together and attention stays as a dot.
 */
export function SidebarTabs({ tabs, selectedId, onSelectionChange, panelHasTabbableContent = false, children }: SidebarTabsProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const [countsFit, setCountsFit] = React.useState(true)
  const [iconsFit, setIconsFit] = React.useState(true)
  const bar = React.useRef<HTMLDivElement>(null)
  const replica = React.useRef<HTMLDivElement>(null)
  const countlessReplica = React.useRef<HTMLDivElement>(null)
  const prefix = React.useId()
  const list = React.useRef<HTMLDivElement>(null)
  const state = useTabListState({
    items: tabs,
    children: tab => <Item key={tab.id} title={sidebarTabName(tab)}>{tab.label}</Item>,
    selectedKey: selectedId,
    onSelectionChange: key => onSelectionChange(String(key)),
  })
  const { tabListProps } = useTabList({ 'aria-label': 'Sidebar' }, state, list)
  const { focusProps, isFocusVisible } = useFocusRing()
  React.useLayoutEffect(() => {
    const host = bar.current, row = replica.current, countless = countlessReplica.current
    if (host === null || row === null || countless === null) return
    const fit = () => {
      const width = host.getBoundingClientRect().width + fitSlack
      setCountsFit(row.getBoundingClientRect().width <= width)
      setIconsFit(countless.getBoundingClientRect().width <= width)
    }
    fit()
    const observer = new ResizeObserver(fit)
    observer.observe(host)
    observer.observe(row)
    observer.observe(countless)
    return () => observer.disconnect()
  }, [])
  return <div ref={bar} data-sidebar-tabs data-sidebar-tab-counts={countsFit ? 'shown' : 'dropped'} {...stylex.props(styles.root)}>
    {/* Hidden replica of the row with every count, at intrinsic width and outside the tablist. */}
    <div ref={replica} aria-hidden inert data-sidebar-tabs-measure {...stylex.props(styles.bar, narrow && styles.touch, styles.measure)}>
      {tabs.map(tab => <span key={tab.id} {...stylex.props(styles.tab, narrow && styles.touchTab)}><TabFace tab={tab} showCount /></span>)}
    </div>
    {/* The next step down: no counts, icons kept. If even this overflows the icons go too, because labels never truncate. */}
    <div ref={countlessReplica} aria-hidden inert data-sidebar-tabs-measure-countless {...stylex.props(styles.bar, narrow && styles.touch, styles.measure)}>
      {tabs.map(tab => <span key={tab.id} {...stylex.props(styles.tab, narrow && styles.touchTab)}><TabFace tab={tab} showCount={false} /></span>)}
    </div>
    <div {...stylex.props(styles.root)}>
      <div {...tabListProps} ref={list} {...stylex.props(styles.bar, narrow && styles.touch)}>
        {tabs.map(tab => <SidebarTabButton key={tab.id} tab={tab} state={state} prefix={prefix} narrow={narrow} showCount={countsFit} showIcon={iconsFit} />)}
      </div>
      {/* Static host knowledge avoids a visibility walk on every roster render and subtree mutation. */}
      {tabs.map(tab => tab.id === selectedId
        ? <div {...focusProps} key={tab.id} id={`${prefix}-panel-${tab.id}`} role="tabpanel" aria-labelledby={`${prefix}-tab-${tab.id}`} tabIndex={panelHasTabbableContent ? -1 : 0} data-sidebar-tab-panel={tab.id} data-focus-visible={isFocusVisible || undefined} {...stylex.props(styles.panel)}>{children}</div>
        : <div key={tab.id} id={`${prefix}-panel-${tab.id}`} role="tabpanel" aria-labelledby={`${prefix}-tab-${tab.id}`} data-sidebar-tab-panel={tab.id} hidden />)}
    </div>
  </div>
}

function SidebarTabButton({ tab, state, prefix, narrow, showCount, showIcon }: { readonly tab: SidebarTab; readonly state: TabListState<SidebarTab>; readonly prefix: string; readonly narrow: boolean; readonly showCount: boolean; readonly showIcon: boolean }) {
  const ref = React.useRef<HTMLDivElement>(null)
  const { tabProps, isSelected } = useTab({ key: tab.id }, state, ref)
  const { focusProps, isFocusVisible } = useFocusRing()
  return <div {...mergeProps(tabProps, focusProps)} ref={ref} id={`${prefix}-tab-${tab.id}`} aria-controls={`${prefix}-panel-${tab.id}`} aria-label={sidebarTabName(tab)} data-sidebar-tab={tab.id} data-selected={isSelected || undefined} data-focus-visible={isFocusVisible || undefined} {...stylex.props(styles.tab, narrow && styles.touchTab)}><TabFace tab={tab} showCount={showCount} showIcon={showIcon} /></div>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } as const
const styles = stylex.create({
  root: { position: 'relative', display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  bar: { display: 'grid', gridAutoFlow: 'column', gridAutoColumns: 'auto', alignItems: 'center', gap: 0, flexShrink: 0, boxSizing: 'border-box', height: g.controlLg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  // Same box as the bar but sized by its content and taking no layout space, so measuring never changes what is measured.
  measure: { position: 'absolute', top: 0, insetInlineStart: 0, width: 'max-content', visibility: 'hidden', pointerEvents: 'none' },
  // Tracks start from content and share the rest, so three tabs with counts fit a 256px sidebar; labels never shrink.
  tab: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', paddingInline: s.xs2, height: g.controlSm, borderRadius: r.control, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':is([data-focus-visible])': focusRing },
  touch: { height: g.touchTarget },
  touchTab: { height: g.touchTarget, minWidth: g.touchTarget, boxSizing: 'border-box' },
  face: { display: 'inline-flex', alignItems: 'center', gap: s.xs2, whiteSpace: 'nowrap' },
  icon: { display: 'inline-flex', flexShrink: 0 },
  // An absent count attribute paints as empty, so a dropped count leaves the attention icon alone.
  // A compact badge: never stretched by the row, a true circle around one digit, growing sideways for more. The line box is the
  // inner height, so a digit sits optically centred. An absent count attribute paints as empty.
  pill: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', alignSelf: 'center', flexShrink: 0, marginInlineStart: s.xs2, boxSizing: 'border-box', height: g.icon, minWidth: g.icon, paddingInline: s.xs2, borderWidth: g.hairline, borderStyle: 'solid', borderRadius: r.full, fontSize: t.denseSize, lineHeight: `calc(${g.icon} - 2 * ${g.hairline})`, fontWeight: t.weightMedium, fontVariantNumeric: 'tabular-nums', '::after': { content: 'attr(data-sidebar-tab-count)' } },
  // Counts dropped: the filled pill stays as a dot; status keeps its icon at badge size.
  dot: { height: s.md, minWidth: s.md, paddingInline: 0 },
  iconOnly: { paddingInline: 0 },
  neutral: { color: ink.fgMuted, borderColor: border.border, backgroundColor: surface.controlFill },
  attention: { color: surface.canvas, borderColor: tone.attention, backgroundColor: tone.attention },
  fault: { color: tone.dangerFg, borderColor: tone.dangerFg },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none', ':is([data-focus-visible])': focusRing },
})
