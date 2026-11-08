import type { Command, WorkspaceState } from '@fractal/ui/workspace'
import { paneIdentity } from '../workspace/subject.ts'
import { CommandPalette } from './CommandPalette.tsx'
import { useLiveHostCommands, useLivePalette, type HostActions } from './live.ts'

/**
 * The palette with its live inputs, as the workspace mounts it: loaded and observing only once a person
 * first opens it, so the shell's first paint carries neither its code nor its reads.
 */
export default function LivePalette({ isOpen, onOpenChange, state, dispatch, actions }: {
  readonly isOpen: boolean
  readonly onOpenChange: (open: boolean) => void
  readonly state: WorkspaceState
  /** The workspace's routed dispatch, so an opening reaches the URL. */
  readonly dispatch: (command: Command) => void
  /** Keep it referentially stable: each change re-asks the palette's sources. */
  readonly actions: HostActions
}) {
  const live = useLivePalette()
  const commands = useLiveHostCommands(actions)
  return <CommandPalette isOpen={isOpen} onOpenChange={onOpenChange} state={state} dispatch={dispatch} identity={paneIdentity} world={live.world} commands={commands} search={live.search} recentStore={live.recentStore} />
}
