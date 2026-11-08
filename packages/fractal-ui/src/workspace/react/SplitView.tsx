import * as React from 'react'
import { mergeProps, useFocusRing, useKeyboard, useMove } from 'react-aria'
import { paneRects, splitRects, type Rect } from '../geometry.ts'
import { minimumSize, panesOf, sharesOf, splitsOf, type Pane, type Split } from '../layout.ts'
import { focusedPane } from '../model.ts'
import { useCurrentSpace, useDispatch } from './context.tsx'
import { dataAttributes, resolveClassName, resolveStyle, type StyleRenderProps } from './renderProps.ts'

export interface WorkspaceRenderValues {
  readonly isZoomed: boolean
  readonly isCollapsed: boolean
}

export interface WorkspaceProps extends StyleRenderProps<WorkspaceRenderValues> {
  /** Renders one pane, usually a `Pane` with its slots. */
  readonly renderPane: (pane: Pane) => React.ReactNode
  /** Renders a divider; a plain `Divider` with the given props by default. */
  readonly renderDivider?: ((props: DividerProps) => React.ReactNode) | undefined
  /** Only the focused pane shows, as on a phone. */
  readonly collapsed?: boolean | undefined
}

const percent = (value: number) => `${value * 100}%`

const placement = (rect: Rect): React.CSSProperties =>
  ({ position: 'absolute', left: percent(rect.x), top: percent(rect.y), width: percent(rect.width), height: percent(rect.height) })

/**
 * The current space's layout. Every pane renders as a flat sibling keyed by its id and placed from the
 * split sizes, so splitting, closing a neighbour, zooming or collapsing never remounts a pane's
 * content: an agent's composer keeps its draft and its terminal stays attached. Zoomed or collapsed,
 * the chosen pane fills the workspace and the others stay mounted but `hidden`.
 */
export const Workspace = ({ renderPane, renderDivider, collapsed = false, className, style }: WorkspaceProps) => {
  const space = useCurrentSpace()
  const container = React.useRef<HTMLDivElement>(null)
  const values: WorkspaceRenderValues = { isZoomed: space.local.zoomed !== null, isCollapsed: collapsed }
  const single = collapsed ? focusedPane(space)?.id : space.local.zoomed ?? undefined
  const panes = panesOf(space.layout)
  const rects = paneRects(space.layout)
  const splits = single === undefined ? splitsOf(space.layout) : []
  const splitAreas = splitRects(space.layout)
  return <div
    ref={container}
    data-workspace=""
    {...dataAttributes({ zoomed: values.isZoomed, collapsed })}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { display: 'block', flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip' })}
  >
    {panes.map(pane => {
      const shown = single === undefined || single === pane.id
      return <div
        key={pane.id}
        data-pane-slot={pane.id}
        hidden={!shown}
        style={{ ...placement(single === pane.id ? { x: 0, y: 0, width: 1, height: 1 } : rects.get(pane.id)!), display: shown ? 'flex' : 'none', minWidth: 0, minHeight: 0, overflow: 'clip' }}
      >
        {renderPane(pane)}
      </div>
    })}
    {splits.flatMap(split => split.children.slice(1).map((_, index) => {
      const area = splitAreas.get(split.id)!
      const props: DividerProps = {
        split,
        index,
        area,
        extent: () => {
          const rect = container.current?.getBoundingClientRect()
          return rect === undefined ? 0 : split.axis === 'row' ? rect.width * area.width : rect.height * area.height
        },
      }
      return <React.Fragment key={`${split.id}:${index}`}>{(renderDivider ?? defaultDivider)(props)}</React.Fragment>
    }))}
  </div>
}

const defaultDivider = (props: DividerProps) => <Divider {...props} />

export interface DividerRenderValues {
  readonly axis: Split['axis']
  readonly isDragging: boolean
  readonly isFocusVisible: boolean
}

export interface DividerProps extends StyleRenderProps<DividerRenderValues> {
  readonly split: Split
  /** Between child `index` and `index + 1`. */
  readonly index: number
  /** The split's area within the workspace, as fractions. */
  readonly area: Rect
  /** The split's length along its axis, in pixels, which a drag measures against. */
  readonly extent: () => number
  readonly 'aria-label'?: string | undefined
  /** Fraction an arrow press moves; five times that with Shift. */
  readonly step?: number | undefined
}

/** Moves the border between two children by `delta` of the whole split, both keeping their minimum. */
export const shiftShares = (shares: readonly number[], index: number, delta: number): readonly number[] => {
  const left = shares[index], right = shares[index + 1]
  if (left === undefined || right === undefined) return shares
  const clamped = Math.max(minimumSize - left, Math.min(right - minimumSize, delta))
  return shares.map((share, at) => at === index ? left + clamped : at === index + 1 ? right - clamped : share)
}

/**
 * A resizable border, a focusable `separator` centred on the boundary it moves (give it a width or
 * height): drag it, use the arrow keys along its axis (Shift for larger steps), Home and End for the
 * extremes, and double-click or Enter to share equally again.
 */
export const Divider = ({ split, index, area, extent, className, style, step = 0.02, 'aria-label': label }: DividerProps) => {
  const dispatch = useDispatch()
  const [dragging, setDragging] = React.useState(false)
  const drag = React.useRef<{ readonly size: number; readonly shares: readonly number[]; moved: number } | undefined>(undefined)
  const row = split.axis === 'row'
  const shares = sharesOf(split)
  const resize = (next: readonly number[]) => dispatch({ _tag: 'Resize', split: split.id, sizes: next })
  const { moveProps } = useMove({
    onMoveStart: event => {
      if (event.pointerType === 'keyboard') return
      drag.current = { size: extent(), shares, moved: 0 }
      setDragging(true)
    },
    onMove: event => {
      if (event.pointerType === 'keyboard') {
        const delta = row ? event.deltaX : event.deltaY
        if (delta !== 0) resize(shiftShares(shares, index, Math.sign(delta) * (event.shiftKey ? step * 5 : step)))
        return
      }
      const current = drag.current
      if (current === undefined || current.size <= 0) return
      current.moved += row ? event.deltaX : event.deltaY
      resize(shiftShares(current.shares, index, current.moved / current.size))
    },
    onMoveEnd: () => {
      drag.current = undefined
      setDragging(false)
    },
  })
  const { keyboardProps } = useKeyboard({
    onKeyDown: event => {
      if (event.key === 'Home') resize(shiftShares(shares, index, -1))
      else if (event.key === 'End') resize(shiftShares(shares, index, 1))
      else if (event.key === 'Enter') dispatch({ _tag: 'ResetSizes', split: split.id })
      else event.continuePropagation()
    },
  })
  const { focusProps, isFocusVisible } = useFocusRing()
  const before = shares.slice(0, index + 1).reduce((sum, share) => sum + share, 0)
  const values: DividerRenderValues = { axis: split.axis, isDragging: dragging, isFocusVisible }
  const position: React.CSSProperties = row
    ? { position: 'absolute', left: percent(area.x + area.width * before), top: percent(area.y), height: percent(area.height), transform: 'translateX(-50%)', cursor: 'col-resize' }
    : { position: 'absolute', top: percent(area.y + area.height * before), left: percent(area.x), width: percent(area.width), transform: 'translateY(-50%)', cursor: 'row-resize' }
  return <div
    {...mergeProps(moveProps, keyboardProps, focusProps)}
    role="separator"
    tabIndex={0}
    aria-label={label ?? 'Resize split'}
    aria-orientation={row ? 'vertical' : 'horizontal'}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(before * 100)}
    data-divider=""
    data-axis={split.axis}
    {...dataAttributes({ dragging, focusVisible: isFocusVisible })}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { ...position, touchAction: 'none', zIndex: 1 })}
    onDoubleClick={() => dispatch({ _tag: 'ResetSizes', split: split.id })}
  />
}
