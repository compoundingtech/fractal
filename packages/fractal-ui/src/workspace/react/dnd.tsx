import * as React from 'react'
import { createPortal } from 'react-dom'
import { dropFor, isNoopDrop, resolveDropZone, stripDropIndex, stripInsertIndex, type DropZone } from '../drop.ts'
import type { EntityDescriptor, EntityDragPayload } from '../entity.ts'
import { findPane } from '../layout.ts'
import { currentSpace, type Drop, type WorkspaceState } from '../model.ts'
import { useWorkspaceContext } from './context.tsx'

/**
 * Pointer-event drag and drop: no HTML5 drag, so the chip under the pointer is live React and every
 * preview is ordinary state. A drag starts once the pointer moved a few pixels with the button held,
 * hit-tests drop targets by their data attributes, and ends on release (drop) or Escape (cancel).
 *
 * Drop targets mark themselves: `data-drop-strip="<pane>"` on a tab strip (tabs inside carry
 * `data-workspace-tab`) and `data-drop-content="<pane>"` on a pane's content.
 */

/** Where a drag would land right now. */
export type DragOver =
  | { readonly _tag: 'Strip'; readonly pane: string; readonly slot: number }
  | { readonly _tag: 'Content'; readonly pane: string; readonly zone: DropZone }

export interface DragState {
  readonly payload: EntityDragPayload | undefined
  readonly pointer: { readonly x: number; readonly y: number } | undefined
  readonly over: DragOver | undefined
}

const idle: DragState = { payload: undefined, pointer: undefined, over: undefined }

/** Pixels the pointer must travel before a press becomes a drag. */
export const dragThreshold = 5

export class DragController {
  state: DragState = idle
  private readonly listeners = new Set<() => void>()
  private teardown: (() => void) | undefined

  constructor(private readonly host: {
    readonly getState: () => WorkspaceState<unknown>
    readonly drop: (payload: EntityDragPayload, drop: Drop) => void
    /** The edge band of a pane's content, as a fraction (or pixels above 1). */
    readonly edge?: number | undefined
  }) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  private set(next: DragState) {
    this.state = next
    for (const listener of this.listeners) listener()
  }

  /** Arms a drag from a pointer press; it starts once the pointer moves past the threshold. */
  press(event: { readonly clientX: number; readonly clientY: number; readonly pointerId: number; readonly currentTarget: EventTarget | null }, payload: () => EntityDragPayload) {
    this.cancel()
    const source = event.currentTarget instanceof Node ? event.currentTarget : undefined
    const origin = { x: event.clientX, y: event.clientY }
    let started = false
    let restoreSelect: string | undefined
    const move = (moved: PointerEvent) => {
      if (moved.pointerId !== event.pointerId) return
      if (!started) {
        if (Math.hypot(moved.clientX - origin.x, moved.clientY - origin.y) < dragThreshold) return
        started = true
        // A drag is not a text selection: none starts, and any the press began goes.
        restoreSelect = document.documentElement.style.userSelect
        document.documentElement.style.userSelect = 'none'
        document.getSelection()?.removeAllRanges()
        this.set({ payload: payload(), pointer: undefined, over: undefined })
      }
      moved.preventDefault()
      this.moveTo(moved.clientX, moved.clientY)
    }
    const up = (released: PointerEvent) => {
      if (released.pointerId !== event.pointerId) return
      const { payload: carried, over } = this.state
      finish()
      if (!started || carried === undefined) return
      // The press that ended a drag is not also a click on what it started on.
      window.addEventListener('click', swallow, { capture: true })
      setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0)
      const drop = over === undefined ? undefined : this.dropOf(over, carried)
      if (drop !== undefined) this.host.drop(carried, drop)
    }
    const cancel = (pointer: PointerEvent) => { if (pointer.pointerId === event.pointerId) finish() }
    const key = (pressed: KeyboardEvent) => {
      if (pressed.key !== 'Escape' || !started) return
      pressed.preventDefault()
      pressed.stopPropagation()
      finish()
    }
    const swallow = (click: MouseEvent) => {
      window.removeEventListener('click', swallow, { capture: true })
      if (source === undefined || !(click.target instanceof Node) || !source.contains(click.target)) return
      click.preventDefault()
      click.stopPropagation()
    }
    const finish = () => {
      window.removeEventListener('pointermove', move, true)
      window.removeEventListener('pointerup', up, true)
      window.removeEventListener('pointercancel', cancel, true)
      window.removeEventListener('keydown', key, true)
      if (restoreSelect !== undefined) document.documentElement.style.userSelect = restoreSelect
      restoreSelect = undefined
      this.teardown = undefined
      if (this.state !== idle) this.set(idle)
    }
    window.addEventListener('pointermove', move, true)
    window.addEventListener('pointerup', up, true)
    window.addEventListener('pointercancel', cancel, true)
    window.addEventListener('keydown', key, true)
    this.teardown = finish
  }

  cancel() {
    this.teardown?.()
  }

  /** Hit-tests the drop targets under a point and records where the drag would land. */
  moveTo(x: number, y: number) {
    const payload = this.state.payload
    if (payload === undefined) return
    this.set({ payload, pointer: { x, y }, over: this.overAt(x, y, payload) })
  }

  private overAt(x: number, y: number, payload: EntityDragPayload): DragOver | undefined {
    const stack = typeof document.elementsFromPoint === 'function' ? document.elementsFromPoint(x, y) : []
    for (const element of stack) {
      const strip = element.closest('[data-drop-strip]')
      if (strip !== null) {
        const pane = strip.getAttribute('data-drop-strip')!
        const tabs = Array.from(strip.querySelectorAll('[data-workspace-tab]')).map(node => node.getBoundingClientRect())
        return { _tag: 'Strip', pane, slot: stripInsertIndex(tabs, x) }
      }
      const content = element.closest('[data-drop-content]')
      if (content !== null) {
        const pane = content.getAttribute('data-drop-content')!
        const zone = resolveDropZone(content.getBoundingClientRect(), { x, y }, { edge: this.host.edge })
        const over: DragOver = { _tag: 'Content', pane, zone }
        return this.dropOf(over, payload) === undefined ? undefined : over
      }
    }
    return undefined
  }

  /** The model's drop for a target, or `undefined` where it would change nothing. */
  dropOf(over: DragOver, payload: EntityDragPayload): Drop | undefined {
    const space = currentSpace(this.host.getState())
    const own = payload.tab !== undefined && payload.tab.space === space.id ? payload.tab.id : undefined
    if (over._tag === 'Strip') {
      const pane = findPane(space.layout, over.pane)
      const from = own === undefined || pane === undefined ? undefined : pane.tabs.findIndex(tab => tab.id === own)
      const drop: Drop = { _tag: 'Strip', pane: over.pane, index: stripDropIndex(over.slot, from) }
      return isNoopDrop(space.layout, own, drop) ? undefined : drop
    }
    const drop = dropFor(over.pane, over.zone)
    return isNoopDrop(space.layout, own, drop) ? undefined : drop
  }
}

export const useDragState = <T,>(selector: (state: DragState) => T): T => {
  const { drag } = useWorkspaceContext()
  const select = () => selector(drag.state)
  return React.useSyncExternalStore(drag.subscribe, select, select)
}

export interface EntityDragProps {
  readonly onPointerDown: (event: React.PointerEvent) => void
}

/**
 * Makes anything that shows an entity draggable onto the workspace: a list row, a resource, a mention,
 * a tab. Spread (or `mergeProps`) the result onto the element. Touch presses scroll as usual unless
 * `touch` is set, since phones show one pane and have nowhere to drop.
 */
export const useEntityDrag = ({ ref, entity, tab, touch = false, isDisabled = false }: {
  readonly ref: string
  /** The entity's description; the workspace's `describe(ref)` when absent. */
  readonly entity?: EntityDescriptor | undefined
  /** Set when the dragged thing is one of the workspace's own tabs, so a drop moves it. */
  readonly tab?: { readonly space: string; readonly id: string } | undefined
  readonly touch?: boolean | undefined
  readonly isDisabled?: boolean | undefined
}): EntityDragProps => {
  const { drag, describe } = useWorkspaceContext()
  const tabSpace = tab?.space, tabId = tab?.id
  return React.useMemo(() => ({
    onPointerDown: (event: React.PointerEvent) => {
      if (isDisabled || event.button !== 0 || (event.pointerType === 'touch' && !touch)) return
      drag.press(event, () => {
        const described = entity ?? describe(ref)
        return tabSpace === undefined || tabId === undefined
          ? { entity: described, ref }
          : { entity: described, ref, tab: { space: tabSpace, id: tabId } }
      })
    },
  }), [drag, describe, ref, entity, tabSpace, tabId, touch, isDisabled])
}

/** The live chip that follows the pointer while dragging. */
export const DragLayer = ({ controller, renderChip }: {
  readonly controller: DragController
  readonly renderChip: ((entity: EntityDescriptor) => React.ReactNode) | undefined
}) => {
  const state = React.useSyncExternalStore(controller.subscribe, () => controller.state, () => controller.state)
  if (renderChip === undefined || state.payload === undefined || state.pointer === undefined || typeof document === 'undefined') return null
  return createPortal(
    <div
      data-drag-chip=""
      {...(state.over === undefined ? {} : { 'data-drop-target': '' })}
      aria-hidden="true"
      style={{ position: 'fixed', left: 0, top: 0, transform: `translate(${state.pointer.x + 12}px, ${state.pointer.y + 12}px)`, pointerEvents: 'none', zIndex: 2147483647 }}
    >
      {renderChip(state.payload.entity)}
    </div>,
    document.body,
  )
}
