import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { paneRects, panesOf, type Pane } from '@fractal/ui/workspace'
import { Divider, Workspace, dropPlacement, useDragState, type DividerProps } from '@fractal/ui/workspace/react'
import { colorVars as c, interactionVars as interaction } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { currentSpace, type Command, type WorkspaceState } from './model.ts'

/**
 * Where each pane's content is, for surfaces that live elsewhere. Every tab's surface stays in one host
 * outside the panes, so a split, a move between panes or a zoom never moves it in the DOM: a draft, a
 * scroll position and an attached terminal survive. Each pane renders an empty slot; this measures the
 * slots against the host and writes their rects as CSS variables on it, which surfaces position from.
 * Measuring runs in a ResizeObserver callback and in layout effects, so a surface lands before paint.
 */
export class PaneSlots {
  private host: HTMLElement | null = null
  private readonly slots = new Map<string, HTMLElement>()
  private readonly slotRefs = new Map<string, (element: HTMLElement | null) => void>()
  private readonly headerRefs = new Map<string, (element: HTMLElement | null) => void>()
  private headers: ReadonlyMap<string, HTMLElement> = new Map()
  private readonly listeners = new Set<() => void>()
  private readonly observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(() => this.measure())

  /** The surface host: the containing block surfaces are placed in. */
  readonly setHost = (element: HTMLElement | null) => {
    if (this.host !== null) this.observer?.unobserve(this.host)
    this.host = element
    if (element !== null) this.observer?.observe(element)
    this.measure()
  }

  /** A stable ref for a pane's content slot. */
  slot(pane: string): (element: HTMLElement | null) => void {
    let ref = this.slotRefs.get(pane)
    if (ref === undefined) {
      ref = element => {
        const previous = this.slots.get(pane)
        if (previous !== undefined) this.observer?.unobserve(previous)
        if (element === null) this.slots.delete(pane)
        else {
          this.slots.set(pane, element)
          this.observer?.observe(element)
        }
        this.measure()
      }
      this.slotRefs.set(pane, ref)
    }
    return ref
  }

  /** A stable ref for a pane's header action slot, where its shown conversation portals its status. */
  header(pane: string): (element: HTMLElement | null) => void {
    let ref = this.headerRefs.get(pane)
    if (ref === undefined) {
      ref = element => {
        const next = new Map(this.headers)
        if (element === null) next.delete(pane)
        else next.set(pane, element)
        this.headers = next
        for (const listener of this.listeners) listener()
      }
      this.headerRefs.set(pane, ref)
    }
    return ref
  }

  readonly subscribeHeaders = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  readonly getHeaders = (): ReadonlyMap<string, HTMLElement> => this.headers

  /** Writes each shown slot's rect on the host. A hidden pane (zoomed out, or off a phone) keeps its last rect. */
  readonly measure = () => {
    const host = this.host
    if (host === null) return
    const origin = host.getBoundingClientRect()
    for (const [pane, slot] of this.slots) {
      const rect = slot.getBoundingClientRect()
      if (rect.width === 0 && rect.height === 0) continue
      const vars = paneVars(pane)
      host.style.setProperty(vars.x, `${rect.left - origin.left}px`)
      host.style.setProperty(vars.y, `${rect.top - origin.top}px`)
      host.style.setProperty(vars.width, `${rect.width}px`)
      host.style.setProperty(vars.height, `${rect.height}px`)
    }
  }

  dispose() {
    this.observer?.disconnect()
  }
}

const paneVars = (pane: string) => {
  const name = pane.replace(/[^a-zA-Z0-9_-]/g, '_')
  return { x: `--wf-pane-${name}-x`, y: `--wf-pane-${name}-y`, width: `--wf-pane-${name}-w`, height: `--wf-pane-${name}-h` }
}

/** A surface's box: its pane's content slot. Before the first measure it fills the host, as one pane does. */
export const surfacePlacement = (pane: string): React.CSSProperties => {
  const vars = paneVars(pane)
  return { left: `var(${vars.x}, 0px)`, top: `var(${vars.y}, 0px)`, width: `var(${vars.width}, 100%)`, height: `var(${vars.height}, 100%)` }
}

export const PaneSlotsContext = React.createContext<PaneSlots | undefined>(undefined)

/**
 * A pane's empty content slot: where its shown surface appears, and where a dragged entity lands, its
 * centre opening it here and an edge splitting that side, previewed by a translucent overlay above
 * the surface. An empty pane shows `children` here.
 */
export const PaneSlot = ({ pane, children }: { readonly pane: string; readonly children?: React.ReactNode }) => {
  const slots = React.useContext(PaneSlotsContext)
  const zone = useDragState(state => state.over?._tag === 'Content' && state.over.pane === pane ? state.over.zone : undefined)
  return <div ref={slots?.slot(pane)} data-pane-slot-content={pane} data-drop-content={pane} {...stylex.props(styles.slot)}>
    {children}
    {zone === undefined ? null : <div data-drop-overlay={zone._tag === 'Center' ? 'center' : zone.side} style={dropPlacement(zone)} {...stylex.props(styles.overlay)} />}
  </div>
}

/** Pane `p` is shown at the top left: the zoomed one, the phone's one, or the first by geometry. */
export const leadPane = (state: WorkspaceState, collapsed: boolean): string => {
  const space = currentSpace(state)
  if (space.local.zoomed !== null) return space.local.zoomed
  if (collapsed) return space.local.focus
  for (const [pane, rect] of paneRects(space.layout)) if (rect.x < 1e-9 && rect.y < 1e-9) return pane
  return panesOf(space.layout)[0]?.id ?? space.local.focus
}

const SplitDivider = (props: DividerProps) => <Divider
  {...props}
  aria-label={props.split.axis === 'row' ? 'Resize split, left and right' : 'Resize split, above and below'}
  className={({ axis, isDragging, isFocusVisible }) => stylex.props(styles.divider, axis === 'row' ? styles.dividerRow : styles.dividerColumn, (isDragging || isFocusVisible) && (axis === 'row' ? styles.dividerRowActive : styles.dividerColumnActive)).className}
/>

/**
 * The current space's splits from the shared primitives, inside their `WorkspaceProvider`: panes as
 * flat siblings placed from the split sizes, dividers between them, and the one surface host over them.
 */
export const WorkspaceSplits = ({ slots, collapsed, onHost, renderPane }: {
  readonly slots: PaneSlots
  /** The surface host, once mounted: a portal container for the surfaces. */
  readonly onHost: (host: HTMLDivElement | null) => void
  /** Only the focused pane shows, as on a phone. */
  readonly collapsed: boolean
  readonly renderPane: (pane: Pane) => React.ReactNode
}) => {
  React.useLayoutEffect(() => { slots.measure() })
  const host = React.useCallback((element: HTMLDivElement | null) => {
    slots.setHost(element)
    onHost(element)
  }, [slots, onHost])
  return <PaneSlotsContext.Provider value={slots}>
    <div data-testid="workspace-splits" {...stylex.props(styles.area)}>
      <Workspace collapsed={collapsed} renderPane={renderPane} renderDivider={SplitDivider} />
      <div ref={host} data-testid="workspace-body-host" {...stylex.props(styles.host)} />
    </div>
  </PaneSlotsContext.Provider>
}

/** Commands the person makes in the workspace. */
export type Dispatch = (command: Command) => void

const styles = stylex.create({
  area: { position: 'relative', display: 'flex', flexGrow: 1, flexBasis: 0, minWidth: 0, minHeight: 0 },
  // Above the panes, but it takes no pointer itself: only its surfaces do, inside their slots.
  host: { position: 'absolute', inset: 0, zIndex: 0, pointerEvents: 'none' },
  // Above the surface host, which is above the panes.
  overlay: { zIndex: 2, backgroundColor: `color-mix(in srgb, ${c.primary} 16%, transparent)`, borderWidth: 2, borderStyle: 'solid', borderColor: `color-mix(in srgb, ${c.primary} 60%, transparent)`, borderRadius: 6, boxSizing: 'border-box' },
  slot: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0, overflow: 'hidden' },
  // A hairline centred in a wider grab area; above the surface host.
  divider: { backgroundColor: 'transparent', outline: 'none' },
  dividerRow: { width: 7, backgroundImage: `linear-gradient(to right, transparent 3px, ${c.border} 3px, ${c.border} 4px, transparent 4px)` },
  dividerColumn: { height: 7, backgroundImage: `linear-gradient(to bottom, transparent 3px, ${c.border} 3px, ${c.border} 4px, transparent 4px)` },
  dividerRowActive: { backgroundImage: `linear-gradient(to right, transparent 3px, ${interaction.focusRing} 3px, ${interaction.focusRing} 4px, transparent 4px)` },
  dividerColumnActive: { backgroundImage: `linear-gradient(to bottom, transparent 3px, ${interaction.focusRing} 3px, ${interaction.focusRing} 4px, transparent 4px)` },
})
