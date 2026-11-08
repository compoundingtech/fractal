import * as Workspace from '@fractal/ui/workspace'
import { createWorkspaceStore as createSharedStore, localStoragePersistence, memoryPersistence, type SpaceSync } from '@fractal/ui/workspace/store'
import { currentSpace, emptyWorkspace, legacyDecoders, workspaceConfig, workspaceSchema, type Command, type TabView, type WorkspaceState } from './model.ts'
import { isAgentTerminalPane, paneIdentity } from './subject.ts'

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
 * The shared store with Fractal's binding: this browser's document (or memory, for embeds and stories), any
 * external space stores such as the person's glasses, and the terminal face a terminal link opens with.
 */
export const createWorkspaceStore = ({ storageKey, initial, sync, debounceMs, newId }: {
  /** `undefined` keeps the workspace in memory. */
  readonly storageKey: string | undefined
  /** A stored document's text, for the in-memory store. */
  readonly initial?: string | undefined
  readonly sync?: readonly SpaceSync[] | undefined
  readonly debounceMs?: number | undefined
  readonly newId?: (() => string) | undefined
}): WorkspaceStore => {
  const config = workspaceConfig(newId)
  const shared = createSharedStore<TabView>(
    storageKey === undefined ? memoryPersistence(initial) : localStoragePersistence(storageKey),
    { config, schema: workspaceSchema, legacy: legacyDecoders(config), sync, initial: () => emptyWorkspace(config), ...(debounceMs === undefined ? {} : { debounceMs }) },
  )
  return {
    ...shared,
    dispatch: command => {
      if (command._tag !== 'Open' || !isAgentTerminalPane(command.ref)) return shared.dispatch(command)
      // A terminal link opens the agent's one tab on its terminal face: the same as `model.ts::transition`.
      const key = paneIdentity(command.ref)
      shared.dispatch({ ...command, ref: key })
      const found = Workspace.locateTab(currentSpace(shared.getState()).layout, tab => paneIdentity(tab.ref) === key)
      if (found !== undefined) shared.dispatch({ _tag: 'SetView', tab: found.tab.id, view: { terminal: true } })
    },
  }
}
