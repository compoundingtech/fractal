// Parent and subagent threads as top tabs over one conversation; threads the strip cannot show are one press away in the thread drawer.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tab, TabList, TabPanel, Tabs, VisuallyHidden } from 'react-aria-components'
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
  /** Parent thread first, then subagent threads, in strip order. */
  readonly threads: readonly AgentThread[]
  readonly selectedId: string
  readonly onSelectionChange: (id: string) => void
  /**
   * The selected thread's conversation. The host renders it, including whether a composer appears:
   * a read-only thread gets this component's note, never a composer.
   */
  readonly children: React.ReactNode
  readonly ref?: React.Ref<AgentThreadTabsHandle>
}

/** Tabs the strip shows before the rest move to the drawer: fewer in the narrow (phone) layout, where tabs are touch targets. */
const visibleTabs = { wide: 6, narrow: 3 } as const

/** The strip keeps thread order and always shows the selected thread, taking the last slot when it would otherwise be hidden. */
function stripThreads(threads: readonly AgentThread[], selected: AgentThread, max: number): readonly AgentThread[] {
  if (threads.length <= max) return threads
  const leading = threads.slice(0, max)
  if (leading.includes(selected)) return leading
  return [...threads.slice(0, max - 1), selected]
}

/**
 * Controlled thread switcher: React Aria tabs (arrow keys, Home/End) for the threads that fit, and a `+N more` drawer
 * listing every thread. Choosing in the drawer closes it and focuses the chosen tab; Escape or Close returns focus to `+N more`.
 * The component fills its container (`minHeight: 0`), so the host's conversation pane can scroll inside it.
 */
export function AgentThreadTabs({ threads, selectedId, onSelectionChange, children, ref }: AgentThreadTabsProps) {
  const narrow = React.useContext(NarrowLayoutContext)
  const drawerId = React.useId()
  const [drawerRequested, setDrawerRequested] = React.useState(false)
  const selectedTab = React.useRef<HTMLElement | null>(null)
  const drawerToggle = React.useRef<HTMLButtonElement>(null)
  /** Thread id whose tab takes focus when it mounts or becomes selected after a drawer choice. */
  const pendingFocus = React.useRef<string | null>(null)
  const attachSelectedTab = React.useCallback((node: HTMLElement | null) => {
    selectedTab.current = node
    if (node === null || pendingFocus.current === null || node.dataset.threadId !== pendingFocus.current) return
    pendingFocus.current = null
    node.focus()
  }, [])
  React.useImperativeHandle(ref, () => ({ focusSelected: () => selectedTab.current?.focus() }), [])

  if (threads.length === 0) return <div {...stylex.props(styles.root)}><div {...stylex.props(styles.body)}>{children}</div></div>
  const selected = threads.find(thread => thread.id === selectedId) ?? threads[0]!
  const shown = stripThreads(threads, selected, narrow ? visibleTabs.narrow : visibleTabs.wide)
  const hidden = threads.length - shown.length
  // The drawer exists only while some thread is off the strip.
  const drawerOpen = drawerRequested && hidden > 0
  const closeDrawer = () => {
    setDrawerRequested(false)
    drawerToggle.current?.focus()
  }
  const chooseFromDrawer = (id: string) => {
    setDrawerRequested(false)
    if (id === selected.id) {
      selectedTab.current?.focus()
      return
    }
    pendingFocus.current = id
    onSelectionChange(id)
  }

  return <Tabs selectedKey={selected.id} onSelectionChange={key => onSelectionChange(String(key))} data-agent-thread-tabs data-narrow={narrow || undefined} {...stylex.props(styles.root)}>
    <div data-agent-thread-strip {...stylex.props(styles.strip, narrow && styles.stripNarrow)}>
      <TabList aria-label="Agent threads" {...stylex.props(styles.tabs)}>
        {shown.map(thread => <Tab key={thread.id} id={thread.id} data-thread-id={thread.id} ref={thread.id === selected.id ? attachSelectedTab : undefined} {...stylex.props(styles.tab, narrow && styles.touch)}>
          <span {...stylex.props(styles.label)}>{thread.label}</span>
          {thread.readOnly ? <VisuallyHidden elementType="span">, read only</VisuallyHidden> : null}
        </Tab>)}
      </TabList>
      {hidden > 0 ? <Button ref={drawerToggle} data-thread-drawer-toggle aria-expanded={drawerOpen} aria-controls={drawerOpen ? drawerId : undefined} onPress={() => drawerOpen ? closeDrawer() : setDrawerRequested(true)} {...stylex.props(styles.more, narrow && styles.touch, drawerOpen && styles.moreOpen)}>
        <Icon name="drawer" />{`+${hidden} more`}
      </Button> : null}
    </div>
    <div {...stylex.props(styles.body)}>
      {/* One panel under a stable React key whose id follows the selection (React Aria points only the selected tab's aria-controls at it), so the host's conversation and anything it retains hidden stay mounted across switches. */}
      <TabPanel key="conversation" id={selected.id} {...stylex.props(styles.panel)}>
        {selected.readOnly ? <div role="note" data-thread-read-only {...stylex.props(styles.note)}>
          <span {...stylex.props(styles.tag)}>Read only</span>
          <span {...stylex.props(styles.noteText)}>{selected.label} can be viewed here, not replied to.</span>
        </div> : null}
        {children}
      </TabPanel>
      {drawerOpen ? <aside id={drawerId} aria-label="All agent threads" data-thread-drawer onKeyDown={event => {
        if (event.key !== 'Escape') return
        event.stopPropagation()
        closeDrawer()
      }} {...stylex.props(styles.drawer, narrow && styles.drawerNarrow)}>
        <header {...stylex.props(styles.drawerHeader)}>
          <strong {...stylex.props(styles.drawerTitle)}>Threads</strong>
          <span {...stylex.props(styles.count)}>{threads.length}</span>
          <Button aria-label="Close agent threads" onPress={closeDrawer} {...stylex.props(styles.iconButton, narrow && styles.touchSquare)}><Icon name="x" /></Button>
        </header>
        <ul {...stylex.props(styles.drawerList)}>{threads.map(thread => <li key={thread.id}>
          <Button data-thread-item={thread.id} autoFocus={thread.id === selected.id} aria-current={thread.id === selected.id ? 'true' : undefined} onPress={() => chooseFromDrawer(thread.id)} {...stylex.props(styles.drawerItem, narrow && styles.touch, thread.id === selected.id && styles.current)}>
            <span {...stylex.props(styles.label)}>{thread.label}</span>
            {thread.readOnly ? <span {...stylex.props(styles.tag)}>{' '}Read only</span> : null}
          </Button>
        </li>)}</ul>
      </aside> : null}
    </div>
  </Tabs>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } as const
const styles = stylex.create({
  root: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  strip: { display: 'flex', alignItems: 'center', gap: s.xs, height: g.controlLg, flexShrink: 0, boxSizing: 'border-box', paddingInline: s.sm, minWidth: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  stripNarrow: { height: g.touchTarget },
  tabs: { display: 'flex', alignItems: 'center', gap: s.xs2, minWidth: 0, flex: '0 1 auto', overflow: 'hidden' },
  tab: { display: 'inline-flex', alignItems: 'center', height: g.controlSm, minWidth: 0, maxWidth: g.crumbMax, paddingInline: s.sm, borderRadius: r.control, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer', outline: 'none', ':hover': { color: ink.fg }, ':is([data-selected])': { backgroundColor: surface.rowActive, color: ink.fg }, ':is([data-focus-visible])': focusRing },
  touch: { height: g.touchTarget, minHeight: g.touchTarget },
  touchSquare: { width: g.touchTarget, height: g.touchTarget },
  label: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  more: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, height: g.controlSm, paddingInline: s.sm, borderWidth: 0, borderRadius: r.control, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap', cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  moreOpen: { color: ink.fg, backgroundColor: surface.rowActive, ':hover': { backgroundColor: surface.rowActive } },
  body: { position: 'relative', display: 'flex', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  panel: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0, outline: 'none', ':is([data-focus-visible])': focusRing },
  note: { display: 'flex', alignItems: 'center', gap: s.md, flexShrink: 0, paddingBlock: s.md, paddingInline: s.lg, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  noteText: { flex: '1 1 0', minWidth: 0, color: ink.fgSoft },
  // The 11/16 tag: 1px border, r6, no fill, muted ink.
  tag: { display: 'inline-flex', alignItems: 'center', flexShrink: 0, marginInlineStart: 'auto', paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap' },
  drawer: { display: 'flex', flexDirection: 'column', width: g.drawerDefault, flexShrink: 0, minHeight: 0, overflowY: 'auto', backgroundColor: surface.raised, color: ink.fg, borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border },
  // On a phone the drawer covers the conversation instead of squeezing it.
  drawerNarrow: { position: 'absolute', insetBlock: 0, insetInlineEnd: 0, width: '100%', borderLeftWidth: 0 },
  drawerHeader: { display: 'flex', alignItems: 'center', gap: s.md, paddingBlock: s.md, paddingInline: s.lg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  drawerTitle: { fontWeight: t.weightMedium },
  count: { flex: '1 1 0', color: ink.fgMuted, fontVariantNumeric: 'tabular-nums' },
  drawerList: { display: 'flex', flexDirection: 'column', gap: s.xs2, margin: s.zero, paddingInline: s.md, paddingBottom: s.md, listStyle: 'none' },
  drawerItem: { display: 'flex', alignItems: 'center', gap: s.sm, width: '100%', minHeight: g.controlLg, paddingInline: s.md, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  current: { backgroundColor: surface.rowActive, color: ink.fg, ':hover': { backgroundColor: surface.rowActive } },
  iconButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, flexShrink: 0, padding: s.zero, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
})
