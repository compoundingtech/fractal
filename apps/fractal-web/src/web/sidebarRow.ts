import type { SidebarAgentRowData, SidebarAgentStatus, SidebarMissionContext } from '@fractal/ui/assistant-ui'
import { statuses, type SidebarMissionInfo } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import type { Agent, DataSource, Feed } from '../data/source.ts'
import type { FeedSync } from '../data/feedSync.ts'
import * as Atom from 'effect/reactivity/Atom'
import type { AgentSubagent } from '../data/source.ts'
import type { Attention, Mission } from '@fractal/client/schema'
import * as Option from 'effect/Option'
import { isWaitingForAnswer } from '../data/attention.ts'

const missionSlug = (id: string): string => id.split('/').at(-1) ?? id

/** Preserve independent feed verdicts; only retained Observed events supply update times. */
export const sidebarMissionFeeds = ({ missions, attention, missionsSync, attentionSync }: {
  readonly missions: Feed<readonly Mission[]>
  readonly attention: Feed<readonly Attention[]>
  readonly missionsSync?: FeedSync<readonly Mission[]>
  readonly attentionSync?: FeedSync<readonly Attention[]>
}) => {
  const feeds = [missions, attention]
  const syncs = [missionsSync, attentionSync]
  const failed = feeds.some(feed => feed._tag === 'Unavailable' || (feed._tag === 'Observed' && feed.error !== undefined)) ||
    syncs.some(sync => sync?.sync.status._tag === 'Failed')
  const stale = feeds.some(feed => feed._tag !== 'Observed' || feed.freshness === 'stale') ||
    syncs.some(sync => sync !== undefined && sync.sync.status._tag !== 'Live')
  const times = syncs.flatMap(sync => sync?.last._tag === 'Some' ? [sync.last.value.observedAt] : [])
  const updatedAt = times.length === 0 ? undefined : Math.min(...times)
  const missionInfo: SidebarMissionInfo = failed ? { _tag: 'Failed', updatedAt } :
    stale ? { _tag: 'Stale', updatedAt } : { _tag: 'Live' }
  return {
    missions: missions._tag === 'Observed' ? missions.value : undefined,
    attention: attention._tag === 'Observed' ? attention.value : undefined,
    missionInfo,
    missionUnavailable: feeds.some(feed => feed._tag === 'Unavailable') ||
      syncs.some((sync, index) => sync?.sync.status._tag === 'Failed' && feeds[index]?._tag !== 'Observed'),
  }
}

/** Mounted only by sidebar rows and their accessible-name projection, never the workspace. */
export const sidebarMissionFeedsAtom = (source: DataSource) => Atom.make(get => sidebarMissionFeeds({
  missions: get(source.missions), attention: get(source.attention),
  missionsSync: source.sync === undefined ? undefined : get(source.sync.missions),
  attentionSync: source.sync === undefined ? undefined : get(source.sync.attention),
}))

/** Join retained feeds only: no per-seat read, queue-size estimate, or synthetic decision handle. */
export const sidebarMissionContext = ({ agent, missions = [], attention = [] }: {
  readonly agent: Agent
  readonly missions?: readonly Mission[]
  readonly attention?: readonly Attention[]
}): SidebarMissionContext => {
  const work = agent.currentWork?.[0]
  const id = work?.missionId ?? agent.mission
  if (id === undefined) return { _tag: 'None' }
  const mission = missions.find(mission => mission.id === id)
  const runs = mission?.run_details?.filter(run => run.id === work?.runId) ?? []
  // WorkLabel has no generation field. A matching step is strongest evidence; otherwise
  // require the summary's generation to agree with the mission's current run generation.
  const run = runs.find(run => run.current_steps.some(step => step.id === work?.id) ||
    Option.getOrUndefined(run.steps)?.some(step => step.id === work?.id)) ??
    runs.find(run => run.generation_id === mission?.run_generations[run.id])
  const question = attention.find(card => isWaitingForAnswer(card) &&
    ((work !== undefined && (card.step_run_id === work.id || card.blocked?.step_run_id === work.id || card.source_id === work.id)) ||
      card.conversation_id === agent.ref || card.conversation_ids?.some(id => id === agent.ref) ||
      card.requester_id === agent.ref || card.source_id === agent.ref) &&
    (card.detail.trim().length > 0 || card.title.trim().length > 0))
  const queued = new Map<string, { readonly id: string; readonly slug: string }>()
  for (const upcoming of agent.upcomingWork ?? []) {
    if (upcoming.missionId === id || queued.has(upcoming.missionId)) continue
    queued.set(upcoming.missionId, { id: upcoming.missionId, slug: missionSlug(upcoming.missionId) })
  }
  return {
    _tag: 'Mission', id, slug: missionSlug(id),
    progress: run === undefined ? { _tag: 'Unknown' } : { _tag: 'Known', ...run.progress },
    step: question !== undefined ? { _tag: 'Waiting', question: question.detail.trim() || question.title } :
      work === undefined ? { _tag: 'Unknown' } : { _tag: 'Step', name: work.title || work.path },
    queued: [...queued.values()],
  }
}

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
export const sidebarRow = ({ agent, stale, needsMe, missions, attention, missionInfo, missionUnavailable = false }: {
  readonly agent: Agent; readonly stale: boolean; readonly needsMe?: boolean
  readonly missions?: readonly Mission[]; readonly attention?: readonly Attention[]
  readonly missionInfo?: SidebarMissionInfo; readonly missionUnavailable?: boolean
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
    missionContext: missionUnavailable && (agent.currentWork?.[0]?.missionId ?? agent.mission) !== undefined
      ? { _tag: 'Unavailable' } : sidebarMissionContext({ agent, missions, attention }),
    missionInfo,
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
