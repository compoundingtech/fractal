import { useAtomValue } from '@effect/atom-react'
import type * as Fractal from '@fractal/client/schema'
import * as React from 'react'

import { useDataSource } from '../data/react.tsx'
import { sortMissions } from './missionWords.ts'
import { presentMission, type MissionPresentation } from './presentation.ts'

export type MissionRow = { readonly mission: Fractal.Mission; readonly presentation: MissionPresentation }

export const startOfDay = (now: number): number => { const day = new Date(now); day.setHours(0, 0, 0, 0); return day.getTime() }

/** Every observed mission, presented and in the contract's order: the one projection the sidebar list and the screen share. */
export const useMissionRows = () => {
  const source = useDataSource()
  const missions = useAtomValue(source.missions)
  const agents = useAtomValue(source.agents)
  const attention = useAtomValue(source.attention)
  const rows = React.useMemo((): readonly MissionRow[] => {
    if (missions._tag !== 'Observed') return []
    const agentRows = agents._tag === 'Observed' ? agents.value : []
    const openAttention = attention._tag === 'Observed' ? attention.value.filter((item) => item.state === 'open') : []
    return sortMissions(missions.value.map((mission) => ({ mission, presentation: presentMission({ mission, agents: agentRows, attention: openAttention }) })))
  }, [missions, agents, attention])
  const observedIds = React.useMemo(() => new Set(missions._tag === 'Observed' ? missions.value.map((mission) => mission.id) : []), [missions])
  return { missions, rows, observedIds }
}
