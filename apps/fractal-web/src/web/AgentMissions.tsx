import * as React from 'react'
import { useAtomValue } from '@effect/atom-react'
import type { DetailsSection, SidebarMissionContext } from '@fractal/ui/assistant-ui'
import { PanelSection, type PanelItem } from './ResourcePanelRows.tsx'
import { useAgent, useConnection, useDataSource, useNowSnapshot } from '../data/react.tsx'
import { MissionEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/MissionCard.tsx'
import { missionChildOf, missionRowData } from '../missions/rowData.ts'
import { useMissionRows } from '../missions/useMissionRows.ts'
import { sidebarMissionFeedsAtom, sidebarRow } from './sidebarRow.ts'
import { missionFace } from '../workspace/faces.ts'

const agentMissionIds = (context: SidebarMissionContext | undefined): readonly string[] =>
  context?._tag !== 'Mission' ? [] : [...new Set([context.id, ...(context.alsoCurrent ?? []).map(each => each.id), ...context.queued.map(each => each.id)])]

/** The Resources tab's missions use the same face and urgency ordering as every resource kind. */
export const AgentMissions = ({ agentRef, onOpen, section }: { readonly agentRef: string; readonly onOpen: (ref: string, split: boolean) => void; readonly section?: DetailsSection | undefined }) => {
  const feed = useAgent(agentRef)
  const source = useDataSource()
  const feeds = useAtomValue(React.useMemo(() => sidebarMissionFeedsAtom(source), [source]))
  const connection = useConnection()
  const now = useNowSnapshot()
  const { rows } = useMissionRows()
  const agent = feed._tag === 'Observed' ? feed.value : undefined
  const frozen = feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')
  const ids = agent === undefined ? [] : agentMissionIds(sidebarRow({ agent, stale: frozen, ...feeds }).missionContext)
  const items: readonly PanelItem[] = ids.flatMap(id => rows.filter(row => row.mission.id === id)).map(row => {
    const data = missionRowData(row)
    const needsYou = row.presentation.steps.find(step => step.word === 'needs_you')
    const failed = row.presentation.steps.find(step => step.word === 'failed')
    const act = needsYou !== undefined ? { tone: 'act' as const, label: needsYou.title }
      : data.tone === 'failed' ? { tone: 'failed' as const, label: failed?.note ?? failed?.title ?? data.stateLabel } : undefined
    return {
      key: row.mission.id,
      observed: { _tag: 'Known', value: missionFace({ title: data.title, state: row.presentation.word === 'working' ? 'active' : row.presentation.word, updatedAt: data.updatedAt }, act), observedAt: data.updatedAt, freshness: frozen ? 'stale' : 'live' },
      ref: row.mission.id,
      hoverCard: <MissionEntityCard mission={missionChildOf(row)} now={now} />,
    }
  })
  return <PanelSection title="Missions" items={items} onOpen={onOpen} section={section} />
}
