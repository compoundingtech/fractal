import * as React from 'react'
import { mergeProps, useFocusRing, useKeyboard, useMove } from 'react-aria'
import { isPane, minimumSize, sharesOf, type Layout, type Pane, type Split } from '../layout.ts'
import { focusedPane, shownPanes } from '../model.ts'
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

/** A flex child that may shrink below its content and keeps absolutely positioned content inside it. */
const fill: React.CSSProperties = { display: 'flex', flex: '1 1 0px', minWidth: 0, minHeight: 0, position: 'relative', overflow: 'clip' }

/** The current space's layout, or only its zoomed (or, collapsed, its focused) pane. */
export const Workspace = ({ renderPane, renderDivider, collapsed = false, className, style }: WorkspaceProps) => {
  const space = useCurrentSpace()
  const values: WorkspaceRenderValues = { isZoomed: space.local.zoomed !== null, isCollapsed: collapsed }
  const single = collapsed ? focusedPane(space) : space.local.zoomed !== null ? shownPanes(space)[0] : undefined
  return <div
    data-workspace=""
    {...dataAttributes({ zoomed: values.isZoomed, collapsed })}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, fill)}
  >
    {single !== undefined ? renderPane(single) : <LayoutNode node={space.layout} renderPane={renderPane} renderDivider={renderDivider} />}
  </div>
}

const LayoutNode = ({ node, renderPane, renderDivider }: {
  readonly node: Layout
  readonly renderPane: (pane: Pane) => React.ReactNode
  readonly renderDivider: ((props: DividerProps) => React.ReactNode) | undefined
}) => isPane(node) ? <>{renderPane(node)}</> : <SplitView split={node} renderPane={renderPane} renderDivider={renderDivider} />

export interface SplitViewProps extends StyleRenderProps<{ readonly axis: Split['axis'] }> {
  readonly split: Split
  readonly renderPane: (pane: Pane) => React.ReactNode
  readonly renderDivider?: ((props: DividerProps) => React.ReactNode) | undefined
}

/** One split: its children sized by their shares, with a divider between each two. */
export const SplitView = ({ split, renderPane, renderDivider, className, style }: SplitViewProps) => {
  const container = React.useRef<HTMLDivElement>(null)
  const shares = sharesOf(split)
  const values = { axis: split.axis }
  return <div
    ref={container}
    data-split={split.id}
    data-axis={split.axis}
    className={resolveClassName(className, values)}
    style={resolveStyle(style, values, { ...fill, flexDirection: split.axis === 'row' ? 'row' : 'column' })}
  >
    {split.children.map((child, index) => <React.Fragment key={child.id}>
      {index === 0 ? null : (renderDivider ?? defaultDivider)({ split, index: index - 1, container })}
      <div data-split-child="" style={{ ...fill, flex: `${shares[index]} 1 0px` }}>
        <LayoutNode node={child} renderPane={renderPane} renderDivider={renderDivider} />
      </div>
    </React.Fragment>)}
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
  /** The split's element, whose size a drag measures against. */
  readonly container: React.RefObject<HTMLElement | null>
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
 * A resizable border, a focusable `separator`: drag it, use the arrow keys along its axis (Shift for
 * larger steps), Home and End for the extremes, and double-click or Enter to share equally again.
 */
export const Divider = ({ split, index, container, className, style, step = 0.02, 'aria-label': label }: DividerProps) => {
  const dispatch = useDispatch()
  const [dragging, setDragging] = React.useState(false)
  const drag = React.useRef<{ readonly size: number; readonly shares: readonly number[]; moved: number } | undefined>(undefined)
  const row = split.axis === 'row'
  const shares = sharesOf(split)
  const resize = (next: readonly number[]) => dispatch({ _tag: 'Resize', split: split.id, sizes: next })
  const { moveProps } = useMove({
    onMoveStart: event => {
      if (event.pointerType === 'keyboard') return
      const rect = container.current?.getBoundingClientRect()
      drag.current = { size: rect === undefined ? 0 : row ? rect.width : rect.height, shares, moved: 0 }
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
    style={resolveStyle(style, values, { flex: '0 0 auto', touchAction: 'none', cursor: row ? 'col-resize' : 'row-resize' })}
    onDoubleClick={() => dispatch({ _tag: 'ResetSizes', split: split.id })}
  />
}
