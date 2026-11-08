import * as React from 'react'
import type { EntityDescriptor, EntityDragPayload } from '../entity.ts'
import { currentSpace, type Command, type Drop, type Space, type WorkspaceState } from '../model.ts'
import type { WorkspaceStore } from '../store.ts'
import { DragController, DragLayer } from './dnd.tsx'

export interface WorkspaceProviderProps<V> {
  readonly store: WorkspaceStore<V>
  /** One description per ref, for tabs, drag chips and anything else that shows an entity. */
  readonly describe: (ref: string) => EntityDescriptor
  /** What the pointer carries while dragging an entity: a live React node that follows it. */
  readonly renderDragChip?: ((entity: EntityDescriptor) => React.ReactNode) | undefined
  /** A drop the host takes over; return `true` when handled. */
  readonly onDropEntity?: ((payload: EntityDragPayload, drop: Drop) => boolean) | undefined
  /** The edge band of a pane's content that splits rather than opens: a fraction, or pixels above 1. */
  readonly dropEdge?: number | undefined
  readonly children?: React.ReactNode
}

export interface WorkspaceContextValue {
  readonly store: WorkspaceStore<unknown>
  readonly dispatch: (command: Command<unknown>) => void
  readonly describe: (ref: string) => EntityDescriptor
  readonly drag: DragController
}

const WorkspaceContext = React.createContext<WorkspaceContextValue | undefined>(undefined)

export const useWorkspaceContext = (): WorkspaceContextValue => {
  const value = React.useContext(WorkspaceContext)
  if (value === undefined) throw new Error('Workspace primitives need a WorkspaceProvider')
  return value
}

/** Provides the store, entity descriptions and the drag-and-drop controller to every primitive. */
export const WorkspaceProvider = <V,>({ store, describe, renderDragChip, onDropEntity, dropEdge, children }: WorkspaceProviderProps<V>) => {
  const latest = React.useRef(onDropEntity)
  latest.current = onDropEntity
  const drag = React.useMemo(() => new DragController({
    getState: () => store.getState() as WorkspaceState<unknown>,
    edge: dropEdge,
    drop: (payload, to) => {
      if (latest.current?.(payload, to) === true) return
      const space = currentSpace(store.getState())
      const dispatch = store.dispatch as (command: Command<unknown>) => void
      if (payload.tab !== undefined && payload.tab.space === space.id) dispatch({ _tag: 'Move', tab: payload.tab.id, to })
      else dispatch({ _tag: 'Open', ref: payload.ref, at: to })
    },
  }), [store, dropEdge])
  React.useEffect(() => () => drag.cancel(), [drag])
  const value = React.useMemo((): WorkspaceContextValue => ({
    store: store as WorkspaceStore<unknown>,
    dispatch: store.dispatch as (command: Command<unknown>) => void,
    describe,
    drag,
  }), [store, describe, drag])
  return <WorkspaceContext.Provider value={value}>
    {children}
    <DragLayer controller={drag} renderChip={renderDragChip} />
  </WorkspaceContext.Provider>
}

/** A slice of the workspace state, re-rendering only when it changes by `Object.is`. */
export const useWorkspace = <V, T>(selector: (state: WorkspaceState<V>) => T): T => {
  const { store } = useWorkspaceContext()
  const select = () => selector(store.getState() as WorkspaceState<V>)
  return React.useSyncExternalStore(store.subscribe, select, select)
}

export const useCurrentSpace = <V = unknown,>(): Space<V> => useWorkspace((state: WorkspaceState<V>) => currentSpace(state))

export const useDispatch = <V = unknown,>(): ((command: Command<V>) => void) =>
  useWorkspaceContext().dispatch as (command: Command<V>) => void
