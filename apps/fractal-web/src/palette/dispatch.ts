import { activeTab, currentSpace, focusedPane, locateTab, type Command, type WorkspaceState } from '@fractal/ui/workspace'
import type { OpenTarget } from '@fractal/ui/workspace/palette'

/** The focused pane of the current space. */
export const paneOf = (state: WorkspaceState): string | undefined => focusedPane(currentSpace(state))?.id

/**
 * The commands that open `ref` where the palette was asked to, at TUI parity (glass.rs `Open`):
 * - here: a tab already open anywhere in the space is revealed; otherwise the focused pane's shown tab
 *   gives way to it, or an empty pane takes it;
 * - tab: a new tab in the focused pane;
 * - right, below: a new split on that side of the focused pane.
 * `identity` is the workspace's own, so an agent's terminal and its conversation are one tab.
 */
export const openCommands = (state: WorkspaceState, ref: string, target: OpenTarget, identity: (ref: string) => string = ref => ref): readonly Command[] => {
  const space = currentSpace(state)
  switch (target) {
    case 'here': {
      const key = identity(ref)
      if (locateTab(space.layout, tab => identity(tab.ref) === key) !== undefined) return [{ _tag: 'Open', ref }]
      const shown = activeTab(space)
      return [shown === undefined ? { _tag: 'Open', ref } : { _tag: 'Replace', tab: shown.id, ref }]
    }
    case 'tab': {
      const pane = paneOf(state)
      return [pane === undefined ? { _tag: 'Open', ref } : { _tag: 'Open', ref, at: { _tag: 'Center', pane } }]
    }
    case 'right': return [{ _tag: 'Split', side: 'right', ref }]
    case 'below': return [{ _tag: 'Split', side: 'bottom', ref }]
  }
}

export type NamingIntent = 'create' | 'rename' | 'duplicate'

/** A name typed in naming mode: a new space, or the current one renamed or copied. */
export const namingCommand = (state: WorkspaceState, intent: NamingIntent, name: string): Command => {
  const space = currentSpace(state).id
  switch (intent) {
    case 'create': return { _tag: 'SpaceNew', name }
    case 'rename': return { _tag: 'SpaceRename', space, name }
    case 'duplicate': return { _tag: 'SpaceDuplicate', space, name }
  }
}
