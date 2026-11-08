import * as React from 'react'
import { createPortal } from 'react-dom'
import * as stylex from '@stylexjs/stylex'
import type { Key } from 'react-aria-components'
import { useDragState, useDropTarget, useOptionalWorkspaceContext } from '@fractal/ui/workspace/react'
import { colorVars as c } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { SidebarMoveTarget, SidebarNode } from '../folders/sidebarContract.ts'
import { paneKey, subjectOfRef } from '../workspace/subject.ts'
import { attributeValue } from '../workspace/WorkspaceSplits.tsx'

/** Where a row would land, in the shape React Aria's drop targets used. */
export interface FolderDropPlace {
  readonly type: 'item' | 'root'
  readonly key?: Key
  readonly dropPosition?: 'before' | 'after' | 'on'
}

/** What a sidebar row's drag carries for the folders: the row it moves. */
interface SidebarRowData { readonly sidebarRow: string }

const isRowData = (data: unknown): data is SidebarRowData =>
  typeof data === 'object' && data !== null && typeof (data as { sidebarRow?: unknown }).sidebarRow === 'string'

const targetId = 'sidebar-folders'

/** Rows that move: folders and top-level agents, never a group, an agent's seat, or a mission line. */
export const movable = (node: SidebarNode | undefined): node is SidebarNode =>
  node !== undefined && node._tag !== 'Group' && !(node._tag === 'Agent' && (node.parent !== undefined || node.missionId !== undefined))

/** Which part of a row the pointer is in: a folder's middle takes the row into it, its edges place it beside. */
export const placeIn = (node: SidebarNode, rect: { readonly top: number; readonly height: number }, y: number): FolderDropPlace['dropPosition'] => {
  const at = rect.height <= 0 ? 0.5 : (y - rect.top) / rect.height
  if (node._tag === 'Agent') return at < 0.5 ? 'before' : 'after'
  return at < 0.25 ? 'before' : at > 0.75 ? 'after' : 'on'
}

/** Pixels from the scroller's top or bottom edge where a drag scrolls it, and the most it scrolls a frame. */
const autoscrollBand = 32
const autoscrollStep = 12

/**
 * Sidebar rows on the workspace's pointer drag: one drag for every destination. A top-level agent
 * lands in a pane as its tab would; any movable row lands among the folders here, before, after or
 * into a row, through the same move check the keyboard's drag uses. The pointer carries the live chip
 * all the way; near the tree's top or bottom edge the tree scrolls. React Aria's keyboard drag (the
 * row's Move button) stays the non-pointer way to file.
 */
export const useFolderPointerDrag = ({ root, nodes, mapTarget, apply }: {
  readonly root: React.RefObject<HTMLElement | null>
  readonly nodes: ReadonlyMap<string, SidebarNode>
  readonly mapTarget: (place: FolderDropPlace) => SidebarMoveTarget | undefined
  /** Moves the row there, or says why it cannot, as a drop does. */
  readonly apply: (id: string, target: SidebarMoveTarget) => void
}) => {
  const workspace = useOptionalWorkspaceContext()
  const keyOf = (element: Element): string | undefined => {
    // A pinned ancestor's copy stands in for its real row.
    const pinned = element.closest('[data-pinned-key]')?.getAttribute('data-pinned-key')
    return pinned ?? element.closest('[data-sidebar-tree-key]')?.getAttribute('data-sidebar-tree-key') ?? undefined
  }
  useDropTarget({
    id: targetId,
    resolve: (element, point, payload) => {
      const host = root.current
      if (host === null || !host.contains(element)) return undefined
      // Tabs and other entities have no place among the folders.
      if (!isRowData(payload.data)) return null
      const key = keyOf(element)
      const node = key === undefined ? undefined : nodes.get(key)
      const row = key === undefined ? null : element.closest('[data-pinned-key], [data-sidebar-tree-key]')
      const place: FolderDropPlace = node === undefined || row === null
        ? { type: 'root' }
        : { type: 'item', key: node.id, dropPosition: placeIn(node, row.getBoundingClientRect(), point.y) }
      if (place.type === 'item' && place.key === payload.data.sidebarRow) return null
      const target = mapTarget(place)
      return target === undefined ? null : { place, target }
    },
    drop: (payload, data) => {
      if (!isRowData(payload.data)) return
      apply(payload.data.sidebarRow, (data as { readonly target: SidebarMoveTarget }).target)
    },
  })
  // The press that ended a drag is not also the row's action. The controller settles in the release's
  // capture phase, before React Aria's press ends on the row, so its change is noted synchronously.
  const settledAt = React.useRef(-Infinity)
  const drag = workspace?.drag
  React.useEffect(() => {
    if (drag === undefined) return
    let carrying = false
    return drag.subscribe(() => {
      const now = drag.state.payload !== undefined && isRowData(drag.state.payload.data)
      if (carrying && !now) settledAt.current = performance.now()
      carrying = now
    })
  }, [drag])
  const justDragged = React.useCallback(() => performance.now() - settledAt.current < 100, [])
  const onPointerDown = (event: React.PointerEvent) => {
    // A touch press scrolls the drawer; the keyboard's drag is the way to file there.
    if (workspace === undefined || event.button !== 0 || event.pointerType === 'touch' || !(event.target instanceof Element)) return
    const key = keyOf(event.target)
    const node = key === undefined ? undefined : nodes.get(key)
    if (!movable(node)) return
    const ref = node._tag === 'Agent' ? paneKey(subjectOfRef(node.subject)) : `folder:${node.id}`
    workspace.drag.press(event, () => ({
      entity: node._tag === 'Agent' ? workspace.describe(ref) : { kind: 'folder', id: node.id, title: node.label },
      ref,
      opens: node._tag === 'Agent',
      data: { sidebarRow: node.id } satisfies SidebarRowData,
    }))
  }
  return { onPointerDown, justDragged }
}

/** The line or box where a dragged row would land, and the tree's edge autoscroll while dragging over it. */
export const FolderDropIndicator = ({ root }: { readonly root: React.RefObject<HTMLElement | null> }) =>
  useOptionalWorkspaceContext() === undefined ? null : <ActiveFolderDropIndicator root={root} />

const ActiveFolderDropIndicator = ({ root }: { readonly root: React.RefObject<HTMLElement | null> }) => {
  const over = useDragState(state => state.over?._tag === 'Target' && state.over.target === targetId ? (state.over.data as { readonly place: FolderDropPlace }).place : undefined)
  const pointer = useDragState(state => state.payload !== undefined && isRowData(state.payload.data) ? state.pointer : undefined)
  const [, redraw] = React.useReducer((count: number) => count + 1, 0)
  // Near the scroller's top or bottom edge, scroll it each frame the pointer rests there.
  React.useEffect(() => {
    const host = root.current
    const scroller = host?.querySelector<HTMLElement>('[role="treegrid"]')
    if (pointer === undefined || scroller == null) return
    let frame = 0
    const step = () => {
      const rect = scroller.getBoundingClientRect()
      if (pointer.x >= rect.left && pointer.x <= rect.right) {
        const delta = pointer.y < rect.top + autoscrollBand && pointer.y >= rect.top - autoscrollBand
          ? -autoscrollStep * (1 - Math.max(0, pointer.y - rect.top) / autoscrollBand)
          : pointer.y > rect.bottom - autoscrollBand && pointer.y <= rect.bottom + autoscrollBand
            ? autoscrollStep * (1 - Math.max(0, rect.bottom - pointer.y) / autoscrollBand)
            : 0
        if (delta !== 0) {
          scroller.scrollTop += delta
          redraw()
        }
      }
      frame = window.requestAnimationFrame(step)
    }
    frame = window.requestAnimationFrame(step)
    return () => window.cancelAnimationFrame(frame)
  }, [pointer, root])
  if (over === undefined || typeof document === 'undefined') return null
  const host = root.current
  const row = over.type === 'item' && host !== null ? host.querySelector(`[data-sidebar-tree-key="${attributeValue(String(over.key))}"]`) : null
  const box = (row ?? host?.querySelector('[role="treegrid"]'))?.getBoundingClientRect()
  if (box === undefined) return null
  const position = over.type === 'root' ? 'on' : over.dropPosition ?? 'on'
  const style: React.CSSProperties = position === 'on'
    ? { left: box.left, top: box.top, width: box.width, height: box.height }
    : { left: box.left, top: (position === 'before' ? box.top : box.bottom) - 1, width: box.width, height: 2 }
  return createPortal(<div aria-hidden="true" data-folder-drop={position} data-drop-key={over.key === undefined ? undefined : String(over.key)} style={style} {...stylex.props(styles.indicator, position === 'on' ? styles.box : styles.line)} />, document.body)
}

const styles = stylex.create({
  indicator: { position: 'fixed', pointerEvents: 'none', zIndex: 2147483646, boxSizing: 'border-box', borderRadius: 4 },
  box: { borderWidth: 2, borderStyle: 'solid', borderColor: c.primary, backgroundColor: `color-mix(in srgb, ${c.primary} 12%, transparent)` },
  line: { backgroundColor: c.primary },
})
