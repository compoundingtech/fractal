import * as React from 'react'

import { type AgentNames, sameRosterNames, selectRosterNames } from './agentName.ts'
import { useFleetSelection } from './react.tsx'

/** Roster display names by agent id, for surfaces that name agents they hold no row for; re-renders only on a rename. */
export const useAgentNames = (): AgentNames => {
  const names = useFleetSelection({ select: selectRosterNames, equal: sameRosterNames })
  return React.useCallback((id: string) => names.get(id), [names])
}
