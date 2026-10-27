// Parent and subagent threads as top tabs over one conversation; threads the strip cannot fit are one press away in the thread drawer.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tab, TabList, Tabs } from 'react-aria-components'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from './Icons'
import { NarrowLayoutContext } from './NarrowLayout'

/** One thread: the parent agent's own conversation or a subagent's. A read-only thread can be viewed but not replied to. */
export interface AgentThread {
  readonly id: string
  readonly label: string
  readonly readOnly?: boolean
}

export interface AgentThreadTabsHandle {
  /** Moves focus to the selected thread's tab. */
  focusSelected(): void
}

export interface AgentThreadTabsProps {
  /** Parent thread first (the host labels it "Main"), then subagent threads, in strip order. */
  readonly threads: readonly AgentThread[]
  readonly selectedId: string
  readonly onSelectionChange: (id: string) => void
  /**
   * The selected thread's conversation. The host renders it, including whether a composer appears:
   * a read-only subagent thread gets this component's notice where the composer would be, never a composer.
   */
  readonly children: React.ReactNode
  readonly ref?: React.Ref<AgentThreadTabsHandle>
}

/** Intrinsic widths read from the hidden strip replica; every value is in CSS pixels. */
interface StripMetrics {
  /** Content-box width of the strip. */
  readonly available: number
  /** Gap between tabs, and between the tab list and the overflow toggle. */
  readonly tabGap: number
  readonly stripGap: number
  /** Overflow toggle at its widest count. */
  readonly toggle: number
  readonly tabs: ReadonlyMap<string, number>
}

/** Sub-pixel slack: rects are fractional, and a tab that fits by rounding must not flip in and out. */
const fitSlack = 0.5

const px = (value: string) => Number.parseFloat(value) || 0

function readMetrics(layer: HTMLElement): StripMetrics | null {
  const row = layer.querySelector<HTMLElement>('[data-thread-measure-row]')
  const toggle = layer.querySelector<HTMLElement>('[data-thread-measure-toggle]')
  if (row === null || toggle === null) return null
  const strip = getComputedStyle(layer)
  const tabs = new Map<string, number>()
  for (const node of row.querySelectorAll<HTMLElement>('[data-thread-measure]')) tabs.set(node.dataset.threadMeasure!, node.getBoundingClientRect().width)
  return {
    available: layer.getBoundingClientRect().width - px(strip.paddingLeft) - px(strip.paddingRight),
    tabGap: px(getComputedStyle(row).columnGap),
    stripGap: px(strip.columnGap),
    toggle: toggle.getBoundingClientRect().width,
    tabs,
  }
}

/**
 * Threads the strip shows, in thread order. All of them when they fit; otherwise the selected thread plus the longest
 * leading run of the others that fits beside it and the overflow toggle. The selected tab always shows, even alone.
 */
function fitThreads(threads: readonly AgentThread[], selected: AgentThread, metrics: StripMetrics | null): readonly AgentThread[] {
  if (metrics === null) return threads
  const width = (thread: AgentThread) => metrics.tabs.get(thread.id) ?? 0
  const all = threads.reduce((sum, thread, index) => sum + width(thread) + (index > 0 ? metrics.tabGap : 0), 0)
  if (all <= metrics.available + fitSlack) return threads
  let budget = metrics.available + fitSlack - metrics.toggle - metrics.stripGap - width(selected)
  const shown = new Set([selected.id])
  for (const thread of threads) {
    if (thread === selected) continue
    const cost = width(thread) + metrics.tabGap
    if (cost > budget) break
    budget -= cost
    shown.add(thread.id)
  }
  return threads.filter(thread => shown.has(thread.id))
}

/** Accessible name in visible order; an explicit label because flex items would make name-from-content read "Docs update , read only". */
const threadName = (thread: AgentThread) => thread.readOnly ? `${thread.label}, read only` : thread.label

/**
 * Controlled thread switcher, shown once a parent has at least one subagent: React Aria tabs (arrow keys, Home/End) for
 * the threads that fit the strip's measured width, and a `+N more` drawer grouping Main and its subagents. Choosing in the
 * drawer closes it and focuses the chosen tab; Escape or Close returns focus to `+N more`. With fewer than two threads
 * there is no strip and the children render as they are. The conversation element stays mounted across switches and
 * when the strip appears. The component fills its container (`minHeight: 0`), so the host's conversation can scroll inside it.
 */
export function AgentThreadTabs({ threads, selectedId, onSelectionChange, children, ref }: AgentThreadTabsProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const drawerId = React.useId()
  const subagentsLabelId = React.useId()
  const [drawerRequested, setDrawerRequested] = React.useState(false)
  const [metrics, setMetrics] = React.useState<StripMetrics | null>(null)
  const selectedTab = React.useRef<HTMLElement | null>(null)
  const panel = React.useRef<HTMLDivElement | null>(null)
  const measureLayer = React.useRef<HTMLDivElement | null>(null)
  const drawerToggle = React.useRef<HTMLButtonElement>(null)
  /** Thread id whose tab takes focus when it mounts or becomes selected after a drawer or notice choice. */
  const pendingFocus = React.useRef<string | null>(null)
  const hasStrip = threads.length >= 2
  /** Re-measure when a label, the thread set or the touch sizing changes; the observer covers width and font changes. */
  const measureKey = hasStrip ? `${narrow}\u0001${threads.map(thread => `${thread.id}\u0000${thread.label}`).join('\u0001')}` : null
  React.useLayoutEffect(() => {
    const layer = measureLayer.current
    if (measureKey === null || layer === null) return
    const update = () => {
      const next = readMetrics(layer)
      // Unchanged metrics keep the previous object, so an observer callback with nothing new re-renders nothing.
      if (next !== null) setMetrics(previous => previous !== null
        && previous.available === next.available && previous.tabGap === next.tabGap && previous.stripGap === next.stripGap && previous.toggle === next.toggle
        && previous.tabs.size === next.tabs.size && [...next.tabs].every(([id, width]) => previous.tabs.get(id) === width)
        ? previous : next)
    }
    update()
    if (typeof ResizeObserver === 'undefined') return
    // The replica is absolutely positioned: its width follows the strip, and what the strip shows never resizes it, so this cannot loop.
    const observer = new ResizeObserver(update)
    observer.observe(layer)
    const row = layer.querySelector('[data-thread-measure-row]')
    if (row !== null) observer.observe(row)
    return () => observer.disconnect()
  }, [measureKey])
  /**
   * The conversation panel is one element for every thread, so the host's conversation never remounts on a switch.
   * React Aria's TabPanel would either remount per key or keep its first generated id (its id hook caches the initial value),
   * leaving the selected tab's `aria-controls` dangling. Instead the panel takes the id the selected tab already controls,
   * and is labelled by that tab, at the commit that attaches either element.
   */
  const linkPanel = () => {
    const tab = selectedTab.current
    const node = panel.current
    if (node === null) return
    const controls = tab?.getAttribute('aria-controls') ?? null
    if (controls === null) node.removeAttribute('id')
    else node.id = controls
    if (tab === null) node.removeAttribute('aria-labelledby')
    else node.setAttribute('aria-labelledby', tab.id)
  }
  const attachSelectedTab = (node: HTMLElement | null) => {
    if (node === null) {
      selectedTab.current = null
      linkPanel()
      return
    }
    // React Aria's collection pass may hand the ref a non-DOM node; only a mounted tab counts.
    if (!(node instanceof HTMLElement)) return
    selectedTab.current = node
    linkPanel()
    if (pendingFocus.current === null || node.dataset.threadId !== pendingFocus.current) return
    pendingFocus.current = null
    node.focus()
  }
  const attachPanel = (node: HTMLDivElement | null) => {
    panel.current = node
    linkPanel()
  }
  React.useImperativeHandle(ref, () => ({ focusSelected: () => selectedTab.current?.focus() }), [])
  const parent = threads[0]
  const subagents = threads.slice(1)
  const selected = threads.find(thread => thread.id === selectedId) ?? parent
  const shown = hasStrip && selected !== undefined ? fitThreads(threads, selected, metrics) : []
  const hidden = hasStrip ? threads.length - shown.length : 0
  // The drawer exists only while some thread is off the strip.
  const drawerOpen = drawerRequested && hidden > 0
  const readOnlyChild = hasStrip && selected !== undefined && selected !== parent && selected.readOnly === true
  // One "read only" on the Subagents group replaces a tag per row when it holds for every subagent.
  const subagentsReadOnly = subagents.length > 0 && subagents.every(thread => thread.readOnly === true)
  const closeDrawer = () => {
    setDrawerRequested(false)
    drawerToggle.current?.focus()
  }
  const choose = (id: string) => {
    if (id === selected?.id) {
      selectedTab.current?.focus()
      return
    }
    pendingFocus.current = id
    onSelectionChange(id)
  }
  const chooseFromDrawer = (id: string) => {
    setDrawerRequested(false)
    choose(id)
  }
  const drawerRow = (thread: AgentThread, role: 'parent' | 'subagent', groupConveysReadOnly: boolean) => <li key={thread.id}>
    <Button data-thread-item={thread.id} data-thread-role={role} aria-label={groupConveysReadOnly ? thread.label : threadName(thread)} autoFocus={thread.id === selected?.id} aria-current={thread.id === selected?.id ? 'true' : undefined} onPress={() => chooseFromDrawer(thread.id)} {...stylex.props(styles.drawerItem, narrow && styles.touch, thread.id === selected?.id && styles.current)}>
      <span {...stylex.props(styles.label)}>{thread.label}</span>
      {thread.readOnly && !groupConveysReadOnly ? <span {...stylex.props(styles.tag)}>{' '}Read only</span> : null}
    </Button>
  </li>

  return <div data-agent-thread-tabs data-narrow={narrow || undefined} data-thread-count={threads.length} data-thread-shown={hasStrip ? shown.length : undefined} data-thread-hidden={hasStrip ? hidden : undefined} data-thread-strip-measured={hasStrip && metrics !== null ? '' : undefined} {...stylex.props(styles.root)}>
    {hasStrip ? <>
      {/* Hidden replica of the strip: every tab and the widest toggle at intrinsic width, measured against the strip's width. Outside Tabs, whose collection pass renders its content twice. */}
      <div ref={measureLayer} aria-hidden inert data-thread-measure-layer {...stylex.props(styles.strip, narrow && styles.stripNarrow, styles.measure)}>
        <div data-thread-measure-row {...stylex.props(styles.tabs, styles.measureRow)}>
          {threads.map(thread => <span key={thread.id} data-thread-measure={thread.id} {...stylex.props(styles.tab, narrow && styles.touch)}><span {...stylex.props(styles.label)}>{thread.label}</span></span>)}
        </div>
        <span data-thread-measure-toggle {...stylex.props(styles.more, narrow && styles.touch)}><Icon name="drawer" />{`+${threads.length - 1} more`}</span>
      </div>
      {/* Only the strip lives inside Tabs: React Aria renders Tabs content twice (once hidden, to build the collection), which must never mount the host's conversation. */}
      <Tabs selectedKey={selected!.id} onSelectionChange={key => onSelectionChange(String(key))} data-agent-thread-strip {...stylex.props(styles.strip, narrow && styles.stripNarrow)}>
        <TabList aria-label="Subagents" {...stylex.props(styles.tabs)}>
          {shown.map(thread => <Tab key={thread.id} id={thread.id} data-thread-id={thread.id} data-thread-role={thread === parent ? 'parent' : 'subagent'} aria-label={threadName(thread)} ref={thread.id === selected!.id ? attachSelectedTab : undefined} {...stylex.props(styles.tab, narrow && styles.touch)}>
            <span {...stylex.props(styles.label)}>{thread.label}</span>
          </Tab>)}
        </TabList>
        {hidden > 0 ? <Button ref={drawerToggle} data-thread-drawer-toggle aria-expanded={drawerOpen} aria-controls={drawerOpen ? drawerId : undefined} onPress={() => drawerOpen ? closeDrawer() : setDrawerRequested(true)} {...stylex.props(styles.more, narrow && styles.touch, drawerOpen && styles.moreOpen)}>
          <Icon name="drawer" />{`+${hidden} more`}
        </Button> : null}
      </Tabs>
    </> : null}
    <div {...stylex.props(styles.body)}>
      {/* Same element with or without the strip, so the conversation survives the first subagent appearing. */}
      <div ref={attachPanel} role={hasStrip ? 'tabpanel' : undefined} data-thread-panel={selected?.id} inert={narrow && drawerOpen} {...stylex.props(styles.panel)}>
        {children}
        {readOnlyChild ? <div role="note" data-thread-read-only={selected!.id} {...stylex.props(styles.note)}>
          <span {...stylex.props(styles.noteText)}>{'Subagent thread · read only — reply in '}</span>
          <Button data-thread-reply-in-parent onPress={() => choose(parent!.id)} {...stylex.props(styles.noteAction, narrow && styles.touch)}>{parent!.label}</Button>
        </div> : null}
      </div>
      {drawerOpen ? <aside id={drawerId} aria-label="Subagent threads" data-thread-drawer onKeyDown={event => {
        if (event.key !== 'Escape') return
        event.stopPropagation()
        closeDrawer()
      }} {...stylex.props(styles.drawer, narrow && styles.drawerNarrow)}>
        <header {...stylex.props(styles.drawerHeader)}>
          <strong {...stylex.props(styles.drawerTitle)}>Subagent threads</strong>
          <Button aria-label="Close subagent threads" onPress={closeDrawer} {...stylex.props(styles.iconButton, narrow && styles.touchSquare)}><Icon name="x" /></Button>
        </header>
        <ul data-thread-group="parent" {...stylex.props(styles.drawerList)}>{drawerRow(parent!, 'parent', false)}</ul>
        <div id={subagentsLabelId} data-thread-group-label {...stylex.props(styles.groupLabel)}>
          <span>Subagents</span>
          <span {...stylex.props(styles.count)}>{subagents.length}</span>
          {subagentsReadOnly ? <span {...stylex.props(styles.tag)}>Read only</span> : null}
        </div>
        <ul aria-labelledby={subagentsLabelId} data-thread-group="subagents" {...stylex.props(styles.drawerList)}>{subagents.map(thread => drawerRow(thread, 'subagent', subagentsReadOnly))}</ul>
      </aside> : null}
    </div>
  </div>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } as const
const styles = stylex.create({
  root: { position: 'relative', display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  strip: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, boxSizing: 'border-box', paddingInline: s.sm, minWidth: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  stripNarrow: { height: g.touchTarget },
  // Spans the strip's width without taking layout space, so measuring never changes what is measured.
  measure: { position: 'absolute', insetInline: 0, top: 0, overflow: 'hidden', visibility: 'hidden', pointerEvents: 'none' },
  measureRow: { flex: 'none', width: 'max-content', overflow: 'visible' },
  tabs: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flex: '0 1 auto', overflow: 'hidden' },
  tab: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, minWidth: 0, maxWidth: g.crumbMax, paddingInline: s.sm, borderRadius: r.control, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':is([data-focus-visible])': focusRing },
  touch: { height: g.touchTarget, minHeight: g.touchTarget },
  touchSquare: { width: g.touchTarget, height: g.touchTarget },
  label: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  more: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.control, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  moreOpen: { color: ink.fg, backgroundColor: surface.rowActive, ':hover': { backgroundColor: surface.rowActive } },
  body: { position: 'relative', display: 'flex', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  // Sits where the composer would: after the conversation, pinned to the bottom of the panel.
  note: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.xs, flexShrink: 0, marginTop: 'auto', paddingBlock: s.md, paddingInline: s.lg, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  noteText: { minWidth: 0, color: ink.fgSoft },
  noteAction: { display: 'inline-flex', alignItems: 'center', flexShrink: 0, height: g.controlSm, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  // The 11/16 tag uses soft ink to retain 4.5:1 contrast on the selected drawer row.
  tag: { display: 'inline-flex', alignItems: 'center', flexShrink: 0, marginInlineStart: 'auto', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap' },
  drawer: { display: 'flex', flexDirection: 'column', width: g.drawerDefault, flexShrink: 0, minHeight: 0, overflowY: 'auto', backgroundColor: surface.raised, color: ink.fg, borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border },
  // On a phone the drawer covers the conversation instead of squeezing it.
  drawerNarrow: { position: 'absolute', insetBlock: 0, insetInlineEnd: 0, width: '100%', borderLeftWidth: 0 },
  drawerHeader: { display: 'flex', alignItems: 'center', gap: s.md, paddingBlock: s.md, paddingInline: s.lg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  drawerTitle: { flex: '1 1 0', minWidth: 0, fontWeight: t.weightMedium },
  groupLabel: { display: 'flex', alignItems: 'center', gap: s.sm, paddingBlock: s.sm, paddingInline: s.lg, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  count: { color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  drawerList: { display: 'flex', flexDirection: 'column', gap: s.xs2, margin: s.zero, paddingInline: s.md, paddingBottom: s.md, listStyle: 'none' },
  drawerItem: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.controlLg, paddingInline: s.md, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  current: { backgroundColor: surface.rowActive, color: ink.fg, ':hover': { backgroundColor: surface.rowActive } },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: s.zero, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
})
