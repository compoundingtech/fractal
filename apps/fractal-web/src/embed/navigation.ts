import type { WorkspaceNavigation } from '../web/LiveAgentWorkspace.tsx'

/** Navigation stays inside this one workspace, never the browser/parent history. */
export const createEmbedNavigation = (initial: string): WorkspaceNavigation => {
  let location = initial
  const listeners = new Set<() => void>()
  return {
    getLocation: () => location,
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    navigate: (next) => {
      if (next === location) return
      location = next
      listeners.forEach(listener => listener())
    },
  }
}
