import type { SidebarAgentRowData, SidebarAgentStatus } from '@fractal/ui/assistant-ui'
import { statuses } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import type { Agent } from '../data/source.ts'
import type { AgentSubagent } from '../data/source.ts'

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
  children: [],
})

/** Presentation of one actual roster observation. A stale roster retains its last-observed status. */
export const sidebarRow = ({ agent, stale, needsMe }: {
  readonly agent: Agent; readonly stale: boolean; readonly needsMe?: boolean
}): SidebarAgentRowData => {
  const status: SidebarAgentStatus = agent.state === 'retired' ? 'retired'
    : agent.state === 'suspended' ? 'suspended'
    : agent.state === 'stopped' || agent.state === 'ended' ? 'ended'
    : agent.state === 'desired' || agent.state === 'starting' ? 'pending'
    : !agent.connected ? 'offline' : agent.activity === 'errored' ? 'stale' : agent.activity
  const validTimestamp = (value: number | undefined) => value !== undefined && Number.isFinite(value) &&
    value >= 0 && value <= 8640000000000000 ? value : undefined
  const lastActivityAt = validTimestamp(agent.lastActivityAt._tag === 'Known' ? agent.lastActivityAt.value : undefined)
  const statusSince = validTimestamp(agent.statusSince)
  // The canonical state drives both the G3 glyph and its sentence-case word.
  // Lifecycle takes precedence over harness activity: a stopped idle harness is Ended, not Idle.
  const statusLabel = statuses[status].label
  return {
    ref: agent.ref, id: agent.ref, title: agent.name, host: agent.host, status,
    statusLabel: stale ? 'Last verified · ' + statusLabel : statusLabel,
    freshness: stale ? 'stale' : 'live', description: agent.description, harness: agent.harness,
    terminal: agent.terminal || undefined, mission: agent.mission, lastActivityAt, statusSince,
    needsMe,
    childrenKnown: agent.subagents !== undefined,
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
