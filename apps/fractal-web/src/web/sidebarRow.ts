import type { SidebarAgentRowData, SidebarAgentStatus } from '@fractal/ui/assistant-ui'
import { statuses } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import type { Agent, AgentSubagent } from '../data/source.ts'
import { contextOccupancy } from '../data/contextOccupancy.ts'

/** Harness IDs are only unique within their parent seat; never use them as graph subjects. */
export const subagentRowId = ({ parent, id }: { readonly parent: string; readonly id: string }): string =>
  `wf/subagent/${encodeURIComponent(parent)}/${encodeURIComponent(id)}`

export const sidebarSubagentRow = ({ parent, child, stale }: {
  readonly parent: Agent; readonly child: AgentSubagent; readonly stale: boolean
}): SidebarAgentRowData => ({
  id: subagentRowId({ parent: parent.ref, id: child.id }),
  ref: subagentRowId({ parent: parent.ref, id: child.id }),
  parentRef: parent.ref, title: child.description ?? child.id,
  harness: child.driver, subagent: child, host: parent.host,
  // The active-child lease proves membership, not activity, usage or a last-turn time.
  status: 'unobserved', statusLabel: 'Not observed', freshness: stale ? 'stale' : 'live',
  usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' },
  subagentObservation: { _tag: 'NotReported', reason: { _tag: 'Harness' } }, children: [],
})

/**
 * Presentation of one actual roster observation. A stale roster retains its last-observed status. Mission runs are not
 * joined here: the row's related-runs read draws the server's projection beneath it.
 */
export const sidebarRow = ({ agent, stale, needsMe, now = Date.now() }: {
  readonly agent: Agent; readonly stale: boolean; readonly needsMe?: boolean
  /** Only fences out future-dated occupancy; rows stay off the ticking clock, which reaches time leaves. */
  readonly now?: number
}): SidebarAgentRowData => {
  const status: SidebarAgentStatus = agent.activity === 'errored' || agent.state === 'failed' ? 'failed'
    : agent.state === 'retired' ? 'retired'
    : agent.state === 'suspended' ? 'suspended'
    : agent.state === 'stopped' || agent.state === 'ended' ? 'ended'
    : agent.state === 'desired' || agent.state === 'starting' ? 'pending'
    : agent.activity
  const validTimestamp = (value: number | undefined) => value !== undefined && Number.isFinite(value) &&
    value >= 0 && value <= 8640000000000000 ? value : undefined
  const lastActivityAt = validTimestamp(agent.lastActivityAt._tag === 'Known' ? agent.lastActivityAt.value : undefined)
  const statusSince = validTimestamp(agent.statusSince)
  const lastObserved = stale || !agent.connected
  // Lifecycle takes precedence over harness activity unless Fractal reports a fault.
  const statusLabel = statuses[status].label
  // Preserve the real projection's driver capability; visual harness metadata can omit non-observing/unknown drivers.
  const observesSubagents = agent.driver?._tag === 'Observing'
  const awaitingFirstReport = observesSubagents && (agent.state === 'desired' || agent.state === 'starting')
  // Stale observations retain the same last-verified facts as the row status, including children and their count.
  const subagentsKnown = agent.subagents !== undefined
  const context = contextOccupancy({ usage: agent.usage, now, observedAfter: agent.contextObservedAfter })
  // The model rides the same incarnation-fenced context report; a placeholder is not a model. Effort is never reported here.
  const model = context._tag === 'Known' && agent.usage._tag === 'Known' && typeof agent.usage.value.aggregation !== 'object'
    ? agent.usage.value.context?.model : undefined
  return {
    ref: agent.ref, id: agent.ref, title: agent.name, host: agent.host, status,
    statusLabel: lastObserved ? 'Last verified · ' + statusLabel : statusLabel,
    freshness: lastObserved ? 'stale' : 'live', description: agent.description, harness: agent.harness,
    terminal: agent.terminal || undefined, mission: agent.mission, lastActivityAt, statusSince, context,
    model: model === undefined || model.trim().length === 0 || model === '.unknown' ? undefined : model,
    needsMe,
    asking: agent.ask._tag === 'Known' ? agent.ask.value === 'question' : undefined,
    subagentCount: subagentsKnown ? agent.subagents!.length : undefined,
    subagentObservation: subagentsKnown ? { _tag: 'Reported' } : awaitingFirstReport ? { _tag: 'Unavailable' } :
      { _tag: 'NotReported', reason: { _tag: observesSubagents ? 'DaemonVersion' : 'Harness' } },
    children: agent.subagents?.map(child => sidebarSubagentRow({ parent: agent, child, stale })) ?? [],
    // Cost is optional and may use another currency; never present unknown spend as $0.
    usage: agent.usage._tag === 'Known' ? {
      _tag: 'Known', scope: 'reported-agent', tokens: agent.usage.value.total_tokens,
      ...(agent.usage.value.currency === 'USD' && agent.usage.value.cost !== undefined ? { usd: agent.usage.value.cost } : {}),
    } : { _tag: 'Unknown' },
    duration: { _tag: 'Unknown' },
    lastTurn: lastActivityAt === undefined ? { _tag: 'Unknown' } : { _tag: 'Known', kind: 'activity', at: lastActivityAt },
  }
}
