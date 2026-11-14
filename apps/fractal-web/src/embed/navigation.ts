import type { WorkspaceNavigation } from '../web/LiveAgentWorkspace.tsx'

/** Navigation stays inside this one workspace, never the browser/parent history. */
export const createEmbedNavigation = (initial: string): WorkspaceNavigation => {
  let location = initial
  const listeners = new Set<() => void>()
  return {
    getLocation: () => location,
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    // This single-location store has no Back stack: push and replace both update the same slot.
    navigate: (next, _options) => {
      if (next === location) return
      location = next
      listeners.forEach(listener => listener())
    },
  }
}
