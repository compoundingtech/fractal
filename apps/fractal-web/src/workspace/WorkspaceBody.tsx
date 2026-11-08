import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { RegistryContext } from '@effect/atom-react'
import type { WorkLogCall } from '@fractal/ui/assistant-ui'
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

export interface WorkspaceBodyProps {
  readonly tabs: readonly Tab[]
  readonly active: string | null
  readonly tiers: ReadonlyMap<string, Tier>
  readonly views: Readonly<Record<string, TabView>>
  readonly names: ReadonlyMap<string, string>
  readonly onOpenTool: (call: WorkLogCall) => void
  readonly onOpen: (ref: string) => void
  readonly onReplace: (id: string, ref: string) => void
  readonly onClose: (id: string) => void
  readonly onBackToAlerts: () => void
  readonly onView: (id: string, view: TabView) => void
  /** Set by an explicit face toggle: the next commit moves focus to what it revealed. */
  readonly focusShownFace: React.RefObject<boolean>
  readonly ux?: UxTelemetry
  readonly empty: React.ReactNode
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
const TabSurface = React.memo(function TabSurface({ tab, shown, covered, name, view, onOpenTool, onOpen, onReplace, onClose, onBackToAlerts, onView, ux }: {
  readonly tab: Tab
  readonly shown: boolean
  readonly covered: boolean
  readonly name: string | undefined
  readonly view: TabView | undefined
  readonly onOpenTool: (call: WorkLogCall) => void
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
  const body = <Surface subject={subject} name={name} visible={shown} terminal={showsTerminal(tab, view)} onOpenTool={onOpenTool} onOpen={onOpen} onReplace={replace} onClose={close} onBackToAlerts={onBackToAlerts} ux={ux} />
  return <div
    ref={memory.host}
    aria-hidden={!shown}
    data-tab-surface={tab.ref}
    data-tab-kind={kind.id}
    onScrollCapture={memory.onScrollCapture}
    {...stylex.props(styles.surface, shown ? styles.shown : covered ? null : styles.hidden, surface.hidden === 'activity' && !shown && styles.inert)}
  >
    {surface.hidden === 'activity' ? <React.Activity mode={shown ? 'visible' : 'hidden'}>{body}</React.Activity> : body}
  </div>
})

/**
 * Hosts every retained surface in id order: strip order never reaches the DOM, because moving a
 * mounted subtree discards its rendering state. A hidden surface is revealed, not re-rendered, on a
 * switch; the outgoing pane stays covered through the switch frame, since locking it costs O(its rows).
 */
export function WorkspaceBody({ tabs, active, tiers, views, names, onOpenTool, onOpen, onReplace, onClose, onBackToAlerts, onView, focusShownFace, ux, empty }: WorkspaceBodyProps) {
  // Work-log folds outlive the surface and reload; reading positions stay with this surface.
  const registry = React.useContext(RegistryContext)
  const [viewportStore] = React.useState(() => new ViewportStore({ folds: persistedFolds(registry) }))
  React.useEffect(() => {
    viewportStore.open()
    return () => viewportStore.dispose()
  }, [viewportStore])
  const activeRow = tabs.find(tab => tab.id === active)
  // Reload restores tabs, not subtrees: a hidden tab first mounts when it is first shown.
  const [shownOnce, setShownOnce] = React.useState<ReadonlySet<string>>(() => new Set(active === null ? [] : [active]))
  if (active !== null && !shownOnce.has(active)) setShownOnce(new Set([...shownOnce, active]))
  const [switched, setSwitched] = React.useState<{ readonly shown: string | null; readonly covered: string | null }>({ shown: active, covered: null })
  if (switched.shown !== active) setSwitched({ shown: active, covered: switched.shown })
  React.useEffect(() => {
    if (switched.covered === null) return
    let timer: number | undefined
    // rAF runs before the switch frame paints; the timeout runs after it.
    const frame = window.requestAnimationFrame(() => { timer = window.setTimeout(() => setSwitched(value => ({ shown: value.shown, covered: null }))) })
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
    area.current?.querySelector<HTMLElement>(terminalFace ? '[data-testid="terminal-pane"][data-terminal-shown="true"]' : '[data-wf-shown-pane] textarea:not(:disabled)')?.focus({ preventScroll: true })
  }, [terminalFace, focusShownFace, narrow])
  const mounted = tabs
    .filter(tab => {
      const tier = tiers.get(tab.id)
      return tier === 'active' || tab.id === active || (tier === 'mounted' && shownOnce.has(tab.id)) || tab.id === switched.covered
    })
    .toSorted((left, right) => left.id < right.id ? -1 : left.id > right.id ? 1 : 0)
  return <ViewportStoreContext.Provider value={viewportStore}>
    <div ref={area} data-testid="workspace-body" {...stylex.props(styles.area)}>
      {mounted.map(tab => {
        const agent = subjectAgent(parsePane(tab.ref))
        return <TabSurface
          // A replaced tab keeps its place but is another surface.
          key={`${tab.id}:${tab.ref}`}
          tab={tab}
          shown={tab.id === active}
          covered={tab.id === switched.covered}
          name={agent === undefined ? undefined : names.get(agent)}
          view={views[tab.id]}
          onOpenTool={onOpenTool}
          onOpen={onOpen}
          onReplace={onReplace}
          onClose={onClose}
          onBackToAlerts={onBackToAlerts}
          onView={onView}
          {...(ux === undefined ? {} : { ux })}
        />
      })}
      {tabs.length === 0 ? empty : null}
    </div>
  </ViewportStoreContext.Provider>
}

const styles = stylex.create({
  area: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, flexBasis: 0, minWidth: 0, minHeight: 0 },
  // Identical geometry in both states: revealing a pane must not invalidate its layout. Only
  // non-inherited properties toggle, so a reveal does not restyle the retained subtree.
  surface: { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, backgroundColor: c.canvas },
  shown: { zIndex: 1 },
  hidden: { contentVisibility: 'hidden' },
  inert: { pointerEvents: 'none' },
})
