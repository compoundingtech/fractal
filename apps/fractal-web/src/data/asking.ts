import type { Attention } from '@fractal/client/schema'
import type { DecodedAgent as AgentRow } from '@fractal/sdk/read'
import * as Option from 'effect/Option'

/** Open Fractal attention about an agent: it raised, requested, or owns or shows the card in its conversation, or the card is on one of its current missions. */
export const attentionAboutAgent = ({
  card,
  ref,
  missionIds,
}: {
  readonly card: Attention
  readonly ref: string
  readonly missionIds: ReadonlySet<string>
}): boolean =>
  card.state === 'open' &&
  (card.source_id === ref ||
    card.requester_id === ref ||
    card.conversation_id === ref ||
    card.conversation_ids?.some((id) => id === ref) === true ||
    (card.mission_id !== undefined && missionIds.has(card.mission_id)))

/** Missions of the agent's current work, as Fractal names them on the agent row. */
export const currentMissionIds = (row: AgentRow | undefined): ReadonlySet<string> =>
  new Set(row?.current_work?.map((work) => work.mission_id) ?? [])

/** Fractal reports an ask-tool question: the seat's structured ask kind is `question`. Fractal's harness writer sets
 * `ask` only while `blocked_on` is `human` (fractal-drivers harness_state), so `ask` alone decides. */
export const asksQuestion = (row: AgentRow): boolean => Option.getOrUndefined(row.ask) === 'question'

/** Open attention that is the agent's own ask: it raised or requested the card, or the card is shown in its conversation.
 * A card on a mission the agent merely works on is the mission's question, not this agent's. */
export const ownAttention = ({ card, ref }: { readonly card: Attention; readonly ref: string }): boolean =>
  attentionAboutAgent({ card: { ...card, mission_id: undefined }, ref, missionIds: new Set() })

/** "Waiting for answer" reads only Fractal: an ask-tool question OR the agent's own open attention, never Fractal's status
 * heuristics and never a card it only shares through a mission. */
export const askingAgentRefs = ({
  rows,
  attention,
}: {
  readonly rows: readonly AgentRow[]
  readonly attention: readonly Attention[]
}): ReadonlySet<string> => {
  const open = attention.filter((card) => card.state === 'open')
  const asking = new Set<string>()
  for (const row of rows) {
    if (asksQuestion(row)) asking.add(row.id)
    else if (open.some((card) => ownAttention({ card, ref: row.id }))) asking.add(row.id)
  }
  return asking
}
