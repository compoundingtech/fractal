import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { RegistryContext } from '@effect/atom-react'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { cancelMeasure } from '../web/perfMarks.ts'
import { persistedFolds } from '../state/cells/view.ts'
import type { UxTelemetry } from '../telemetry/ux.ts'
import { workspaceKinds } from './kinds.ts'
import { showsTerminal, type Tab, type TabView } from './model.ts'
import { parsePane, subjectAgent } from './subject.ts'
import type { Tier } from './resources.ts'
import { ThreadHeaderSlotContext } from '../shell/threadHeaderSlot.tsx'
import { attributeValue, surfacePlacement } from './WorkspaceSplits.tsx'

export interface WorkspaceBodyProps {
  readonly tabs: readonly Tab[]
  /** The focused pane's shown tab: the URL's subject, and where an explicit face toggle moves focus. */
  readonly active: string | null
  /** Each pane's shown tab. */
  readonly shown: ReadonlySet<string>
  /** Each tab's pane, whose content slot its surface sits over. */
  readonly paneOf: ReadonlyMap<string, string>
  /** Each pane's header action slot, where its shown conversation portals its status. */
  readonly headerSlots: ReadonlyMap<string, HTMLElement>
  /** A press or focus inside a surface focuses its pane. */
  readonly onFocusPane: (pane: string) => void
  readonly tiers: ReadonlyMap<string, Tier>
  readonly views: Readonly<Record<string, TabView>>
  readonly names: ReadonlyMap<string, string>
  readonly onOpen: (ref: string) => void
  readonly onReplace: (id: string, ref: string) => void
  readonly onClose: (id: string) => void
  readonly onBackToAlerts: () => void
  readonly onView: (id: string, view: TabView) => void
  /** Set by an explicit face toggle: the next commit moves focus to what it revealed. */
  readonly focusShownFace: React.RefObject<boolean>
  readonly ux?: UxTelemetry
}

const scrollerWithin = (root: HTMLElement): HTMLElement | undefined => {
  for (const node of root.querySelectorAll<HTMLElement>('*')) {
    if (node.scrollHeight <= node.clientHeight) continue
    const overflow = getComputedStyle(node).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return node
  }
  return undefined
}

/** Carries a kind's reading position through hiding and eviction. Conversations use the kit's ViewportStore instead. */
const useScrollMemory = ({ enabled, shown, scrollTop, save }: {
  readonly enabled: boolean
  readonly shown: boolean
  readonly scrollTop: number | undefined
  readonly save: (scrollTop: number) => void
}) => {
  const host = React.useRef<HTMLDivElement>(null)
  const latest = React.useRef(scrollTop)
  React.useLayoutEffect(() => {
    if (!enabled || !shown || latest.current === undefined || latest.current === 0) return
    const restore = () => {
      const scroller = host.current === null ? undefined : scrollerWithin(host.current)
      if (scroller === undefined) return false
      scroller.scrollTop = latest.current ?? 0
      return true
    }
    if (restore()) return
    // A cold surface fills in after its data and code arrive.
    let attempts = 0
    const timer = window.setInterval(() => {
      attempts += 1
      if (restore() || attempts > 20) window.clearInterval(timer)
    }, 50)
    return () => window.clearInterval(timer)
  }, [enabled, shown])
  React.useEffect(() => {
    if (enabled && !shown && latest.current !== undefined) save(latest.current)
  }, [enabled, shown, save])
  const onScrollCapture = enabled && shown
    ? (event: React.UIEvent<HTMLDivElement>) => {
      if (event.target instanceof HTMLElement) latest.current = event.target.scrollTop
    }
    : undefined
  return { host, onScrollCapture, latest }
}

/** One tab's retained surface. Its props are stable so a roster tick or another tab's switch never re-renders it. */
const TabSurface = React.memo(function TabSurface({ tab, pane, headerSlot, shown, covered, name, view, onOpen, onReplace, onClose, onBackToAlerts, onView, ux }: {
  readonly tab: Tab
  readonly pane: string
  readonly headerSlot: HTMLElement | null
  readonly shown: boolean
  readonly covered: boolean
  readonly name: string | undefined
  readonly view: TabView | undefined
  readonly onOpen: (ref: string) => void
  readonly onReplace: (id: string, ref: string) => void
  readonly onClose: (id: string) => void
  readonly onBackToAlerts: () => void
  readonly onView: (id: string, view: TabView) => void
  readonly ux?: UxTelemetry
}) {
  const subject = React.useMemo(() => parsePane(tab.ref), [tab.ref])
  const kind = workspaceKinds.kindOf(subject)
  const surface = kind.view
  const save = React.useCallback((scrollTop: number) => onView(tab.id, { scrollTop }), [onView, tab.id])
  const replace = React.useCallback((ref: string) => onReplace(tab.id, ref), [onReplace, tab.id])
  const close = React.useCallback(() => onClose(tab.id), [onClose, tab.id])
  const memory = useScrollMemory({ enabled: surface.scrollMemory, shown, scrollTop: view?.scrollTop, save })
  React.useEffect(() => {
    // Only the kit's composer knows its text; read it as the tab hides, while the surface is still here.
    if (shown) return
    const hasDraft = (memory.host.current?.querySelector('textarea')?.value ?? '') !== ''
    if (hasDraft !== (view?.dirty ?? false)) onView(tab.id, { dirty: hasDraft })
  }, [shown, tab.id, onView, memory.host, view?.dirty])
  const Surface = surface.Surface
  const body = <Surface subject={subject} name={name} visible={shown} terminal={showsTerminal(tab, view)} onOpen={onOpen} onReplace={replace} onClose={close} onBackToAlerts={onBackToAlerts} ux={ux} />
  return <div
    ref={memory.host}
    aria-hidden={!shown}
    data-tab-surface={tab.ref}
    data-tab-surface-id={tab.id}
    data-surface-pane={pane}
    data-tab-kind={kind.id}
    onScrollCapture={memory.onScrollCapture}
    style={surfacePlacement(pane)}
    {...stylex.props(styles.surface, shown ? styles.shown : covered ? null : styles.hidden, !shown && styles.inert)}
  >
    <ThreadHeaderSlotContext.Provider value={headerSlot as HTMLDivElement | null}>
      {surface.hidden === 'activity' ? <React.Activity mode={shown ? 'visible' : 'hidden'}>{body}</React.Activity> : body}
    </ThreadHeaderSlotContext.Provider>
  </div>
})

const sameSet = (left: ReadonlySet<string>, right: ReadonlySet<string>) => left.size === right.size && [...left].every(id => right.has(id))

/**
 * Hosts every retained surface in id order: neither strip order nor the split layout reaches the DOM,
 * because moving a mounted subtree discards its rendering state. Each surface sits over its pane's
 * content slot. A hidden surface is revealed, not re-rendered, on a switch; the outgoing one stays
 * covered through the switch frame, since locking it costs O(its rows).
 */
export function WorkspaceBody({ tabs, active, shown, paneOf, headerSlots, onFocusPane, tiers, views, names, onOpen, onReplace, onClose, onBackToAlerts, onView, focusShownFace, ux }: WorkspaceBodyProps) {
  // Work-log folds outlive the surface and reload; reading positions stay with this surface.
  const registry = React.useContext(RegistryContext)
  const [viewportStore] = React.useState(() => new ViewportStore({ folds: persistedFolds(registry) }))
  React.useEffect(() => {
    viewportStore.open()
    return () => viewportStore.dispose()
  }, [viewportStore])
  const activeRow = tabs.find(tab => tab.id === active)
  // Reload restores tabs, not subtrees: a hidden tab first mounts when it is first shown.
  const [shownOnce, setShownOnce] = React.useState<ReadonlySet<string>>(() => shown)
  if ([...shown].some(id => !shownOnce.has(id))) setShownOnce(new Set([...shownOnce, ...shown]))
  const [switched, setSwitched] = React.useState<{ readonly shown: ReadonlySet<string>; readonly covered: ReadonlySet<string> }>({ shown, covered: new Set() })
  if (!sameSet(switched.shown, shown)) setSwitched({ shown, covered: new Set([...switched.shown].filter(id => !shown.has(id))) })
  React.useEffect(() => {
    if (switched.covered.size === 0) return
    let timer: number | undefined
    // rAF runs before the switch frame paints; the timeout runs after it.
    const frame = window.requestAnimationFrame(() => { timer = window.setTimeout(() => setSwitched(value => ({ shown: value.shown, covered: new Set() }))) })
    return () => { window.cancelAnimationFrame(frame); window.clearTimeout(timer) }
  }, [switched])
  const reading = React.useMemo(() => new Set(tabs.flatMap(tab => {
    const agent = subjectAgent(parsePane(tab.ref))
    return agent === undefined ? [] : [agent]
  })), [tabs])
  // Viewports save in mutation cleanup; prune after those saves so a closed key cannot re-enter the memory.
  React.useLayoutEffect(() => { viewportStore.retain(reading) })
  const activeAgent = activeRow === undefined ? undefined : subjectAgent(parsePane(activeRow.ref))
  const terminalFace = activeRow !== undefined && activeAgent !== undefined && showsTerminal(activeRow, views[activeRow.id])
  const showingConversation = terminalFace ? undefined : activeAgent
  const lastConversation = React.useRef<string | undefined>(undefined)
  React.useLayoutEffect(() => {
    // Abandon transcript timings at the eager boundary, even if the module or data has not loaded.
    const left = lastConversation.current
    if (left !== undefined && left !== showingConversation) {
      cancelMeasure('switch_to_tail', left)
      cancelMeasure('open_transcript', left)
    }
    lastConversation.current = showingConversation
  }, [showingConversation])
  const area = React.useRef<HTMLDivElement>(null)
  const narrow = React.useContext(NarrowLayoutContext)
  // An explicit switch moves focus to the shown face. Terminal focus stays on the pane: focusing the grid would arm typing without a gesture.
  // A phone keeps focus on its toggle, so the switch does not raise the keyboard over the view it just revealed.
  React.useLayoutEffect(() => {
    if (!focusShownFace.current) return
    focusShownFace.current = false
    if (narrow) return
    const surface = active === null ? null : area.current?.querySelector<HTMLElement>(`[data-tab-surface-id="${attributeValue(active)}"]`)
    surface?.querySelector<HTMLElement>(terminalFace ? '[data-testid="terminal-pane"][data-terminal-shown="true"]' : '[data-wf-shown-pane] textarea:not(:disabled)')?.focus({ preventScroll: true })
  }, [terminalFace, focusShownFace, narrow, active])
  // The surfaces sit outside their panes, so a press or focus in one focuses its pane here.
  const focusPaneOf = (event: React.SyntheticEvent) => {
    const id = event.target instanceof Element ? event.target.closest('[data-tab-surface-id]')?.getAttribute('data-tab-surface-id') : undefined
    const pane = id === null || id === undefined ? undefined : paneOf.get(id)
    if (pane !== undefined) onFocusPane(pane)
  }
  const mounted = tabs
    .filter(tab => {
      const tier = tiers.get(tab.id)
      return tier === 'active' || shown.has(tab.id) || (tier === 'mounted' && shownOnce.has(tab.id)) || switched.covered.has(tab.id)
    })
    .toSorted((left, right) => left.id < right.id ? -1 : left.id > right.id ? 1 : 0)
  return <ViewportStoreContext.Provider value={viewportStore}>
    <div ref={area} data-testid="workspace-body" onPointerDownCapture={focusPaneOf} onFocusCapture={focusPaneOf} {...stylex.props(styles.area)}>
      {mounted.map(tab => {
        const agent = subjectAgent(parsePane(tab.ref))
        return <TabSurface
          // A replaced tab keeps its place but is another surface.
          key={`${tab.id}:${tab.ref}`}
          tab={tab}
          pane={paneOf.get(tab.id) ?? ''}
          headerSlot={headerSlots.get(paneOf.get(tab.id) ?? '') ?? null}
          shown={shown.has(tab.id)}
          covered={switched.covered.has(tab.id)}
          name={agent === undefined ? undefined : names.get(agent)}
          view={views[tab.id]}
          onOpen={onOpen}
          onReplace={onReplace}
          onClose={onClose}
          onBackToAlerts={onBackToAlerts}
          onView={onView}
          {...(ux === undefined ? {} : { ux })}
        />
      })}
    </div>
  </ViewportStoreContext.Provider>
}

const styles = stylex.create({
  area: { position: 'absolute', inset: 0 },
  // Identical geometry in both states: revealing a pane must not invalidate its layout. Only
  // non-inherited properties toggle, so a reveal does not restyle the retained subtree.
  // Placed over its pane's slot by `surfacePlacement`; the host itself takes no pointer.
  surface: { position: 'absolute', display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, backgroundColor: c.canvas, pointerEvents: 'auto' },
  shown: { zIndex: 1 },
  hidden: { contentVisibility: 'hidden' },
  inert: { pointerEvents: 'none' },
})
