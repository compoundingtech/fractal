import { EntityCard, EntityCardHeader, EntityStatusMark } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { alertFrom, KIND_WORDS } from '../conversation/alerts/alertChoices.ts'
import { isWaitingForAnswer } from '../data/attention.ts'
import { useAttention, useFleet, useMissions } from '../data/react.tsx'

/** An alert's one card, wherever it is asked for by id (its tab, a palette row): what it asks, who asks, for which mission. */
export const LiveAlertCard = ({ alertId }: { readonly alertId: string }) => {
  const attention = useAttention()
  const fleet = useFleet()
  const missions = useMissions()
  const item = attention._tag === 'Observed' ? attention.value.find(each => each.id === alertId) : undefined
  if (item === undefined) return null
  const agents = fleet._tag === 'Observed' ? fleet.value.agents : []
  const from = alertFrom(item, ref => agents.find(agent => agent.ref === ref)?.name)
  const mission = item.mission_id === undefined || missions._tag !== 'Observed' ? undefined : missions.value.find(each => each.mission.id === item.mission_id)?.mission.title
  const waiting = isWaitingForAnswer(item)
  return <EntityCard entity={{ kind: 'alert', id: item.id }} testId="alert-hover-card">
    <EntityCardHeader title={item.title} kind={KIND_WORDS[item.attention_kind] ?? 'Alert'}
      glyph={waiting ? <EntityStatusMark status={{ tone: 'needs-you', label: 'Waiting for you' }} /> : undefined}
      summary={[waiting ? 'Waiting for you' : undefined, from, mission]} />
  </EntityCard>
}
