import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import { MissionListRow, missionRowAccessibleName } from '@fractal/ui/assistant-ui/shell'
import { colorVars as c, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useAgent, useConnection, useDataSource, useNowSnapshot } from '../data/react.tsx'
import { HoverCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx'
import { MissionEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/MissionCard.tsx'
import { missionChildOf, missionRowData } from '../missions/rowData.ts'
import { useMissionRows } from '../missions/useMissionRows.ts'
import { sidebarMissionFeedsAtom, sidebarRow } from './sidebarRow.ts'

/** This agent's missions, in its own order: what it works on now, then what is queued behind it. */
const agentMissionIds = (context: ReturnType<typeof sidebarRow>['missionContext']): readonly string[] =>
  context?._tag !== 'Mission' ? [] : [...new Set([context.id, ...(context.alsoCurrent ?? []).map(each => each.id), ...context.queued.map(each => each.id)])]

/**
 * The agent's missions in Details, drawn with the Missions tab's own row and model; the agents column is left out
 * because every row is this agent's. A row opens its mission; pointer rest shows the mission's canonical hover card.
 */
export const AgentMissions = ({ agentRef, onOpen }: { readonly agentRef: string; readonly onOpen: (ref: string) => void }) => {
  const feed = useAgent(agentRef)
  const source = useDataSource()
  const feeds = useAtomValue(React.useMemo(() => sidebarMissionFeedsAtom(source), [source]))
  const connection = useConnection()
  const now = useNowSnapshot()
  const { rows } = useMissionRows()
  const agent = feed._tag === 'Observed' ? feed.value : undefined
  const frozen = feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')
  const ids = React.useMemo(() => agent === undefined ? [] : agentMissionIds(sidebarRow({ agent, stale: frozen, ...feeds }).missionContext), [agent, frozen, feeds])
  const shown = React.useMemo(() => ids.flatMap(id => rows.filter(row => row.mission.id === id)), [ids, rows])
  if (shown.length === 0) return null
  return <section aria-label="Missions" data-agent-missions {...stylex.props(styles.section)}>
    <h2 {...stylex.props(styles.title)}>Missions</h2>
    <Aria.GridList aria-label="Missions" selectionMode="none" onAction={key => onOpen(String(key))} {...stylex.props(styles.list)}>
      {shown.map(row => {
        const data = missionRowData(row)
        return <Aria.GridListItem key={row.mission.id} id={row.mission.id} textValue={data.title} aria-label={missionRowAccessibleName(data, now)} {...stylex.props(styles.item)}>
          {/* The mission's canonical card; no agents list, since every row here is this agent's. */}
          <HoverCard placement="left top" content={() => <MissionEntityCard mission={missionChildOf(row)} now={now} />}>
            <div data-mission-hover-anchor><MissionListRow mission={data} now={now} showAgents={false} frozen={frozen} /></div>
          </HoverCard>
        </Aria.GridListItem>
      })}
    </Aria.GridList>
  </section>
}

const styles = stylex.create({
  section: { paddingInline: s.sm, paddingBlock: s.xs, flexShrink: 0 },
  title: { margin: 0, paddingBlock: s.xs, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted },
  list: { outline: 'none' },
  // As in the Missions tab: the row draws its own hover and focus.
  item: { display: 'block', outlineStyle: 'none' },
})
