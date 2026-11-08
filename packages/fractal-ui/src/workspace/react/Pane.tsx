import * as React from 'react'
import { mergeProps, useFocusRing, useKeyboard, useMove, useTab, useTabList, useTabPanel } from 'react-aria'
import { Item, useTabListState, type TabListState } from 'react-stately'
import { bandOf, clampSize, resolveAside } from '../aside.ts'
import type { DropZone } from '../drop.ts'
import type { EntityDescriptor } from '../entity.ts'
import type { Pane as PaneData, Tab as TabData } from '../layout.ts'
import type { Band } from '../model.ts'
import { useCurrentSpace, useDispatch, useWorkspaceContext } from './context.tsx'
import { useDragState, useEntityDrag } from './dnd.tsx'
import { dataAttributes, resolveChildren, resolveClassName, resolveStyle, type RenderProps, type StyleRenderProps } from './renderProps.ts'

interface PaneContextValue {
  readonly pane: PaneData
  readonly isFocused: boolean
  /** The pane's measured width in pixels; 0 before the first measure. */
  readonly width: number
  readonly tabs: TabListState<TabData>
  readonly aside: AsideControls | undefined
  readonly setAside: (aside: AsideControls | undefined) => void
}

const PaneContext = React.createContext<PaneContextValue | undefined>(undefined)

const usePaneContext = (): PaneContextValue => {
  const value = React.useContext(PaneContext)
  if (value === undefined) throw new Error('Pane slots need a Pane')
  return value
}

/** The enclosing pane, its focus, its width and its shown tab. */
export const usePane = (): { readonly pane: PaneData; readonly isFocused: boolean; readonly width: number; readonly activeTab: TabData | undefined } => {
  const { pane, isFocused, width } = usePaneContext()
  return { pane, isFocused, width, activeTab: pane.tabs.find(tab => tab.id === pane.active) }
}

export interface PaneRenderValues {
  readonly pane: PaneData
  readonly activeTab: TabData | undefined
  readonly isFocused: boolean
  readonly isZoomed: boolean
  readonly isEmpty: boolean
  readonly isDropTarget: boolean
}

export interface PaneProps extends RenderProps<PaneRenderValues> {
  readonly pane: PaneData
}

const useWidth = (element: React.RefObject<HTMLElement | null>): number => {
  const [width, setWidth] = React.useState(0)
  React.useLayoutEffect(() => {
    const node = element.current
    if (node === null) return
    setWidth(node.getBoundingClientRect().width)
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      const entry = entries[entries.length - 1]
      if (entry !== undefined) setWidth(entry.contentRect.width)
    })
    observer.observe(node)
    return () => observer.disconnect()
  }, [element])
  return width
}

/**
 * One pane: a positioned, clipped box that takes the focus when pressed or focused within. Compose its
 * slots inside: `TabStrip`, then a `PaneBody` holding `PaneContent` and an optional `PaneAside`.
 */
export const Pane = ({ pane, className, style, children }: PaneProps) => {
  const space = useCurrentSpace()
  const dispatch = useDispatch()
  const element = React.useRef<HTMLDivElement>(null)
  const width = useWidth(element)
  const [aside, setAside] = React.useState<AsideControls | undefined>(undefined)
  const isFocused = space.local.focus === pane.id
  const isDropTarget = useDragState(state => state.over?.pane === pane.id)
  const tabs = useTabListState<TabData>({
    items: pane.tabs,
    children: tab => <Item key={tab.id} textValue={tab.title ?? tab.ref}>{tab.title ?? tab.ref}</Item>,
    // Only an empty pane shows nothing, and it has no tab to select.
    selectedKey: pane.active ?? '',
    onSelectionChange: key => {
      if (typeof key === 'string' && key !== pane.active) dispatch({ _tag: 'Activate', tab: key })
    },
  })
  const values: PaneRenderValues = {
    pane,
    activeTab: pane.tabs.find(tab => tab.id === pane.active),
    isFocused,
    isZoomed: space.local.zoomed === pane.id,
    isEmpty: pane.tabs.length === 0,
    isDropTarget,
  }
  const focus = () => { if (!isFocused) dispatch({ _tag: 'FocusPane', pane: pane.id }) }
  const context = React.useMemo(() => ({ pane, isFocused, width, tabs, aside, setAside }), [pane, isFocused, width, tabs, aside])
  return <PaneContext.Provider value={context}>
    <div
      ref={element}
      data-pane={pane.id}
      {...dataAttributes({ focused: isFocused, zoomed: values.isZoomed, empty: values.isEmpty, dropTarget: isDropTarget })}
      className={resolveClassName(className, values)}
      style={resolveStyle(style, values, { display: 'flex', flexDirection: 'column', flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip' })}
      onPointerDownCapture={focus}
      onFocusCapture={focus}
    >
      {resolveChildren(children, values)}
    </div>
  </PaneContext.Provider>
}

/** The row under the strip that holds the content and the aside. */
export const PaneBody = ({ className, style, children }: RenderProps<{ readonly pane: PaneData }>) => {
  const { pane } = usePaneContext()
  const values = { pane }
  return <div
    data-pane-body=""
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { display: 'flex', flexDirection: 'row', flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip' })}
  >
    {resolveChildren(children, values)}
  </div>
}

export interface PaneContentRenderValues {
  readonly pane: PaneData
  readonly activeTab: TabData | undefined
}

export interface DropOverlayValues {
  readonly zone: DropZone
  /** Structural placement for the zone: the half of the pane on that side, or all of it. */
  readonly placement: React.CSSProperties
}

export interface PaneContentProps extends RenderProps<PaneContentRenderValues> {
  /** The preview of where a dragged entity would land in this pane. */
  readonly renderDropOverlay?: ((values: DropOverlayValues) => React.ReactNode) | undefined
}

export const dropPlacement = (zone: DropZone): React.CSSProperties => {
  const base: React.CSSProperties = { position: 'absolute', pointerEvents: 'none' }
  if (zone._tag === 'Center') return { ...base, inset: 0 }
  switch (zone.side) {
    case 'left': return { ...base, top: 0, bottom: 0, left: 0, width: '50%' }
    case 'right': return { ...base, top: 0, bottom: 0, right: 0, width: '50%' }
    case 'top': return { ...base, left: 0, right: 0, top: 0, height: '50%' }
    case 'bottom': return { ...base, left: 0, right: 0, bottom: 0, height: '50%' }
  }
}

/**
 * The pane's one scroll container (the shown tab's `tabpanel`) and its drop target: the centre opens a
 * dragged entity here, an edge splits that side. Absolutely positioned content stays inside it, and
 * scrolling never chains past it.
 */
export const PaneContent = ({ className, style, children, renderDropOverlay }: PaneContentProps) => {
  const { pane, tabs } = usePaneContext()
  const scroller = React.useRef<HTMLDivElement>(null)
  const { tabPanelProps } = useTabPanel({}, tabs, scroller)
  const zone = useDragState(state => state.over?._tag === 'Content' && state.over.pane === pane.id ? state.over.zone : undefined)
  const values: PaneContentRenderValues = { pane, activeTab: pane.tabs.find(tab => tab.id === pane.active) }
  return <div
    data-pane-content=""
    data-drop-content={pane.id}
    {...dataAttributes({ dropZone: zone === undefined ? undefined : zone._tag === 'Center' ? 'center' : zone.side })}
    style={{ display: 'flex', flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip' }}
  >
    <div
      ref={scroller}
      {...(pane.tabs.length === 0 ? {} : tabPanelProps)}
      data-pane-scroll=""
      className={resolveClassName(className, values)}
      style={resolveStyle(style, values, { flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'auto', overscrollBehavior: 'contain' })}
    >
      {resolveChildren(children, values)}
    </div>
    {zone === undefined || renderDropOverlay === undefined ? null : renderDropOverlay({ zone, placement: dropPlacement(zone) })}
  </div>
}

// ---------------------------------------------------------------------------------------------------
// The tab strip

export interface TabStripRenderValues {
  readonly pane: PaneData
  readonly isDropTarget: boolean
  /** The slot a dragged entity would land in: 0 before the first tab. */
  readonly dropSlot: number | undefined
}

export interface TabStripProps extends StyleRenderProps<TabStripRenderValues> {
  readonly 'aria-label'?: string | undefined
  /** Renders each tab, usually a `Tab`. */
  readonly children: (tab: TabData, index: number) => React.ReactNode
  /** Content before and after the tabs that is not a tab: buttons, a menu. */
  readonly leading?: React.ReactNode
  readonly trailing?: React.ReactNode
}

const TabStripContext = React.createContext<{ readonly dropSlot: number | undefined } | undefined>(undefined)

/**
 * A pane's tabs as a React Aria tab list (selection follows focus; arrows, Home and End move), plus
 * Delete or Backspace to close and Alt+Shift+arrows to reorder. Its slots take drops between tabs.
 */
export const TabStrip = ({ className, style, children, leading, trailing, 'aria-label': label }: TabStripProps) => {
  const { pane, tabs } = usePaneContext()
  const list = React.useRef<HTMLDivElement>(null)
  const { tabListProps } = useTabList<TabData>({ 'aria-label': label ?? 'Tabs', orientation: 'horizontal', keyboardActivation: 'automatic' }, tabs, list)
  const dropSlot = useDragState(state => state.over?._tag === 'Strip' && state.over.pane === pane.id ? state.over.slot : undefined)
  const values: TabStripRenderValues = { pane, isDropTarget: dropSlot !== undefined, dropSlot }
  return <TabStripContext.Provider value={{ dropSlot }}>
    <div
      data-tab-strip=""
      data-drop-strip={pane.id}
      {...dataAttributes({ dropTarget: values.isDropTarget })}
      className={resolveClassName(className, values)}
      style={resolveStyle(style, values, { display: 'flex', flexDirection: 'row', alignItems: 'stretch', minWidth: 0, flex: '0 0 auto' })}
    >
      {leading}
      <div {...tabListProps} ref={list} style={{ display: 'flex', flexDirection: 'row', minWidth: 0, flex: '1 1 auto' }}>
        {pane.tabs.map((tab, index) => <React.Fragment key={tab.id}>{children(tab, index)}</React.Fragment>)}
      </div>
      {trailing}
    </div>
  </TabStripContext.Provider>
}

export interface TabRenderValues {
  readonly tab: TabData
  readonly entity: EntityDescriptor
  readonly index: number
  readonly isActive: boolean
  /** Active in the focused pane. */
  readonly isCurrent: boolean
  readonly isDragging: boolean
  readonly isFocusVisible: boolean
  readonly isDropBefore: boolean
  readonly isDropAfter: boolean
  readonly close: () => void
}

export interface TabProps extends RenderProps<TabRenderValues> {
  readonly tab: TabData
}

/** One tab: selectable, closable, and draggable within and between panes and onto pane edges. */
export const Tab = ({ tab, className, style, children }: TabProps) => {
  const { describe } = useWorkspaceContext()
  const dispatch = useDispatch()
  const space = useCurrentSpace()
  const { pane, isFocused, tabs } = usePaneContext()
  const strip = React.useContext(TabStripContext)
  const element = React.useRef<HTMLDivElement>(null)
  const entity = React.useMemo(() => describe(tab.ref), [describe, tab.ref])
  const { tabProps, isSelected } = useTab({ key: tab.id }, tabs, element)
  const drag = useEntityDrag({ ref: tab.ref, entity, tab: { space: space.id, id: tab.id } })
  const isDragging = useDragState(state => state.payload?.tab?.id === tab.id && state.payload.tab.space === space.id)
  const { focusProps, isFocusVisible } = useFocusRing()
  const index = pane.tabs.findIndex(row => row.id === tab.id)
  const close = React.useCallback(() => dispatch({ _tag: 'Close', tab: tab.id }), [dispatch, tab.id])
  const { keyboardProps } = useKeyboard({
    onKeyDown: event => {
      if (event.altKey && event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
        event.preventDefault()
        dispatch({ _tag: 'Move', tab: tab.id, to: { _tag: 'Strip', pane: pane.id, index: index + (event.key === 'ArrowLeft' ? -1 : 1) } })
      } else if (!event.altKey && !event.metaKey && !event.ctrlKey && (event.key === 'Delete' || event.key === 'Backspace')) {
        event.preventDefault()
        close()
      } else event.continuePropagation()
    },
  })
  const values: TabRenderValues = {
    tab,
    entity,
    index,
    isActive: isSelected,
    isCurrent: isSelected && isFocused,
    isDragging,
    isFocusVisible,
    isDropBefore: strip?.dropSlot === index,
    isDropAfter: strip?.dropSlot === index + 1 && index === pane.tabs.length - 1,
    close,
  }
  return <div
    {...mergeProps(tabProps, keyboardProps, focusProps, drag)}
    ref={element}
    data-workspace-tab=""
    data-tab-id={tab.id}
    data-kind={entity.kind}
    {...dataAttributes({ active: isSelected, current: values.isCurrent, dragging: isDragging, focusVisible: isFocusVisible, dropBefore: values.isDropBefore, dropAfter: values.isDropAfter })}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values)}
    onAuxClick={event => {
      if (event.button !== 1) return
      event.preventDefault()
      close()
    }}
  >
    {resolveChildren(children, values)}
  </div>
}

// ---------------------------------------------------------------------------------------------------
// The aside

export interface AsideControls {
  readonly isOpen: boolean
  readonly size: number
  readonly band: Band
  /** An explicit open or close, which holds until the pane leaves its width band. */
  readonly setOpen: (open: boolean) => void
  readonly toggle: () => void
}

/** The enclosing pane's aside, when it has one: for a toggle in its strip or header. */
export const usePaneAside = (): AsideControls | undefined => usePaneContext().aside

export interface PaneAsideRenderValues extends AsideControls {
  readonly side: 'start' | 'end'
  readonly activeTab: TabData | undefined
}

export interface PaneAsideProps extends RenderProps<PaneAsideRenderValues> {
  readonly side?: 'start' | 'end' | undefined
  /**
   * The pane width (px) from which the aside shows by default: what the content needs plus the aside.
   * Per kind, as a function of the shown tab.
   */
  readonly threshold: number | ((tab: TabData | undefined) => number)
  /** How far into the other band a pane must move before an explicit toggle stops holding. */
  readonly hysteresis?: number | undefined
  readonly defaultSize?: number | undefined
  readonly minSize?: number | undefined
  readonly maxSize?: number | undefined
  readonly 'aria-label'?: string | undefined
  readonly resizerClassName?: StyleRenderProps<{ readonly isDragging: boolean; readonly isFocusVisible: boolean }>['className']
  readonly resizerStyle?: StyleRenderProps<{ readonly isDragging: boolean; readonly isFocusVisible: boolean }>['style']
}

/**
 * A resizable, collapsible side panel inside the pane, on its start or end. By default it shows when
 * the pane is at least `threshold` wide; a person's explicit toggle wins until the pane crosses into
 * the other band. Its open state and width belong to the pane.
 */
export const PaneAside = ({ side = 'end', threshold, hysteresis, defaultSize = 320, minSize = 200, maxSize = 640, className, style, children, resizerClassName, resizerStyle, 'aria-label': label }: PaneAsideProps) => {
  const dispatch = useDispatch()
  const space = useCurrentSpace()
  const { pane, width, setAside } = usePaneContext()
  const stored = space.local.panes[pane.id]?.aside
  const activeTab = pane.tabs.find(tab => tab.id === pane.active)
  const limit = typeof threshold === 'function' ? threshold(activeTab) : threshold
  const measured = width > 0
  const resolved = resolveAside({ width, threshold: limit, hysteresis, explicit: stored?.explicit })
  const available = measured ? Math.max(minSize, Math.min(maxSize, width - minSize)) : maxSize
  const size = clampSize(stored?.size ?? defaultSize, { min: minSize, max: available })
  // An explicit toggle that no longer holds is forgotten, so it cannot come back with the band.
  const stale = measured && stored?.explicit !== undefined && resolved.explicit === undefined
  React.useEffect(() => {
    if (stale) dispatch({ _tag: 'SetAside', pane: pane.id, aside: { explicit: null } })
  }, [stale, dispatch, pane.id])
  const isOpen = measured && resolved.open
  const controls = React.useMemo((): AsideControls => {
    const setOpen = (open: boolean) => dispatch({ _tag: 'SetAside', pane: pane.id, aside: { explicit: { open, band: bandOf(width, limit) } } })
    return { isOpen, size, band: resolved.band, setOpen, toggle: () => setOpen(!isOpen) }
  }, [dispatch, pane.id, width, limit, isOpen, size, resolved.band])
  React.useEffect(() => { setAside(controls) }, [controls, setAside])
  React.useEffect(() => () => setAside(undefined), [setAside])
  if (!isOpen) return null
  const values: PaneAsideRenderValues = { ...controls, side, activeTab }
  return <div
    role="complementary"
    aria-label={label}
    data-pane-aside={side}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { display: 'flex', flexDirection: side === 'start' ? 'row-reverse' : 'row', flex: `0 0 ${size}px`, minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip', order: side === 'start' ? -1 : 1 })}
  >
    <AsideResizer pane={pane.id} side={side} size={size} minSize={minSize} maxSize={available} className={resizerClassName} style={resizerStyle} />
    <div data-pane-aside-body="" style={{ flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'auto', overscrollBehavior: 'contain' }}>
      {resolveChildren(children, values)}
    </div>
  </div>
}

const AsideResizer = ({ pane, side, size, minSize, maxSize, className, style }: {
  readonly pane: string
  readonly side: 'start' | 'end'
  readonly size: number
  readonly minSize: number
  readonly maxSize: number
} & StyleRenderProps<{ readonly isDragging: boolean; readonly isFocusVisible: boolean }>) => {
  const dispatch = useDispatch()
  const [dragging, setDragging] = React.useState(false)
  const start = React.useRef<{ readonly size: number; moved: number } | undefined>(undefined)
  const set = (next: number) => dispatch({ _tag: 'SetAside', pane, aside: { size: Math.round(Math.max(minSize, Math.min(maxSize, next))) } })
  // Dragging toward the content grows the aside.
  const direction = side === 'end' ? -1 : 1
  const { moveProps } = useMove({
    onMoveStart: event => {
      if (event.pointerType === 'keyboard') return
      start.current = { size, moved: 0 }
      setDragging(true)
    },
    onMove: event => {
      if (event.pointerType === 'keyboard') {
        if (event.deltaX !== 0) set(size + direction * Math.sign(event.deltaX) * (event.shiftKey ? 64 : 16))
        return
      }
      const from = start.current
      if (from === undefined) return
      from.moved += event.deltaX
      set(from.size + direction * from.moved)
    },
    onMoveEnd: () => {
      start.current = undefined
      setDragging(false)
    },
  })
  const { keyboardProps } = useKeyboard({
    onKeyDown: event => {
      if (event.key === 'Home') set(minSize)
      else if (event.key === 'End') set(maxSize)
      else event.continuePropagation()
    },
  })
  const { focusProps, isFocusVisible } = useFocusRing()
  const values = { isDragging: dragging, isFocusVisible }
  return <div
    {...mergeProps(moveProps, keyboardProps, focusProps)}
    role="separator"
    tabIndex={0}
    aria-label="Resize side panel"
    aria-orientation="vertical"
    aria-valuemin={minSize}
    aria-valuemax={maxSize}
    aria-valuenow={size}
    data-aside-resizer=""
    {...dataAttributes({ dragging, focusVisible: isFocusVisible })}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { flex: '0 0 auto', touchAction: 'none', cursor: 'col-resize' })}
  />
}
