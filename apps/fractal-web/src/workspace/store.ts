import * as Workspace from '@fractal/ui/workspace'
import { createWorkspaceStore as createSharedStore, memoryPersistence, type SpaceSync } from '@fractal/ui/workspace/store'
import { currentSpace, emptyWorkspace, terminalRefOf, towardAgent, workspaceConfig, workspaceSchema, type Command, type TabView, type WorkspaceState } from './model.ts'
import { paneIdentity } from './subject.ts'

export interface WorkspaceStore {
  /** Synchronous, so an activation commits in the event that caused it. */
  readonly dispatch: (command: Command) => void
  readonly getState: () => WorkspaceState
  readonly subscribe: (listener: () => void) => () => void
  /** Writes pending state now, as the page goes away. */
  readonly flush: () => void
  readonly dispose: () => void
}

/**
 * The shared store with Fractal's binding. The workspace lives in memory: with glasses granted, st's glasses
 * are its spaces (`glassSync.ts`) and the layout comes back from them; without, it lasts as long as the page.
 * A terminal link opens the agent's tab on its terminal face.
 */
export const createWorkspaceStore = ({ initial, sync, debounceMs, newId }: {
  /** What the workspace starts from, such as the person's glasses; an empty one otherwise. */
  readonly initial?: WorkspaceState | undefined
  readonly sync?: readonly SpaceSync[] | undefined
  readonly debounceMs?: number | undefined
  readonly newId?: (() => string) | undefined
} = {}): WorkspaceStore => {
  const config = workspaceConfig(newId)
  const shared = createSharedStore<TabView>(memoryPersistence(), {
    config, schema: workspaceSchema, sync,
    initial: () => initial ?? emptyWorkspace(config),
    ...(debounceMs === undefined ? {} : { debounceMs }),
  })
  return {
    ...shared,
    dispatch: command => {
      const terminal = terminalRefOf(command)
      if (terminal === undefined) return shared.dispatch(command)
      // A terminal ref opens, replaces with or splits to the agent's one tab, on its terminal face: as `model.ts::transition`.
      const key = paneIdentity(terminal)
      shared.dispatch(towardAgent(command, terminal))
      const found = Workspace.locateTab(currentSpace(shared.getState()).layout, tab => paneIdentity(tab.ref) === key)
      if (found !== undefined) shared.dispatch({ _tag: 'SetView', tab: found.tab.id, view: { terminal: true } })
    },
  }
}
