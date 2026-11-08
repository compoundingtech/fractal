import { EntityCard, EntityCardHeader, EntityCardList, EntityCardListItem, EntityCardNote, EntityCardSection } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { useMissionRows } from '../missions/useMissionRows.ts'

const listLimit = 4

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`

/** The Missions view card. It orders rows by the contract's word list, so it loads with the on-demand missions surfaces. */
export const MissionsViewCard = () => {
  const { missions, rows } = useMissionRows()
  const active = rows.filter(row => row.presentation.settled_at === null && !row.presentation.system)
  return <EntityCard entity={{ kind: 'view', id: 'missions' }} testId="missions-hover-card">
    <EntityCardHeader title="Missions" summary={missions._tag !== 'Observed' ? ['Loading'] : [plural(active.length, 'active mission'), `${rows.length} in all`]} />
    {active.length > 0 && <EntityCardSection label="Active" icon="checklist">
      <EntityCardList>{active.slice(0, listLimit).map(row => <EntityCardListItem key={row.mission.id} title={row.mission.title} trailing={`${row.presentation.progress.done}/${row.presentation.progress.total}`} detail={row.presentation.word} />)}</EntityCardList>
      {active.length > listLimit && <EntityCardNote>+{active.length - listLimit} more</EntityCardNote>}
    </EntityCardSection>}
  </EntityCard>
}
