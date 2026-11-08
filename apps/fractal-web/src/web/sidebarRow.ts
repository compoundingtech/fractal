import { markdownPlainText } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdown-source.ts'
import { markdownSource } from '@fractal/client/markdown'
import type { SidebarAgentRowData, SidebarAgentStatus, SidebarMissionContext } from '@fractal/ui/assistant-ui'
import { statuses, type SidebarMissionInfo, type SidebarOtherMission } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import type { Agent, DataSource, Feed } from '../data/source.ts'
import type { SidebarAgent } from '../folders/sidebarContract.ts'
import type { FeedSync } from '../data/feedSync.ts'
import * as Atom from 'effect/reactivity/Atom'
import type { AgentSubagent, AgentWork } from '../data/source.ts'
import type { Attention, Mission } from '@fractal/client/schema'
import * as Option from 'effect/Option'
import { isWaitingForAnswer } from '../data/attention.ts'
import { contextOccupancy } from '../data/contextOccupancy.ts'
import { missionDisplayTitle } from '../missions/missionTitle.ts'

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

const workStepName = (work: AgentWork): string => work.title || work.goal || work.path

// WorkLabel has no generation field. A matching step is strongest evidence; otherwise
// require the summary's generation to agree with the mission's current run generation.
const workRun = ({ mission, work }: { readonly mission: Mission | undefined; readonly work: AgentWork | undefined }) => {
  const runs = mission?.run_details?.filter(run => run.id === work?.runId) ?? []
  return runs.find(run => run.current_steps.some(step => step.id === work?.id) ||
    Option.getOrUndefined(run.steps)?.some(step => step.id === work?.id)) ??
    runs.find(run => run.generation_id === mission?.run_generations[run.id])
}

/** The first current mission stays on the agent row; each further distinct one is a child row. One source for both. */
const currentMissionWork = (agent: Agent) => {
  const work = agent.currentWork?.[0]
  const id = work?.missionId ?? agent.mission
  const others = new Map<string, AgentWork>()
  for (const other of agent.currentWork ?? []) if (other.missionId !== id && !others.has(other.missionId)) others.set(other.missionId, other)
  return { work, id, others: [...others.values()] }
}

export const missionChildRowId = ({ parent, mission }: { readonly parent: string; readonly mission: string }): string =>
  `wf/mission/${encodeURIComponent(parent)}/${encodeURIComponent(mission)}`

/** The seat's tree children: further missions first, then subagents. Absent when the seat reports neither. */
export const agentSeatChildren = (agent: Agent): readonly SidebarAgent[] | undefined => {
  const missions = currentMissionWork(agent).others.map(work => ({
    _tag: 'Agent' as const, id: missionChildRowId({ parent: agent.ref, mission: work.missionId }), subject: agent.ref, parent: agent.ref,
    missionId: work.missionId, label: missionDisplayTitle({ title: undefined, work }),
  }))
  const subagents = agent.subagents?.map(child => ({
    _tag: 'Agent' as const, id: subagentRowId({ parent: agent.ref, id: child.id }), subject: agent.ref, parent: agent.ref, childId: child.id,
    // Roster AgentSubagent has only the parent session_id; the child reference exists
    // only in SubagentSummary.conversation (conversation_blocks.rs), so this row cannot open.
    label: child.description ?? child.id,
  }))
  return missions.length === 0 ? subagents : [...missions, ...(subagents ?? [])]
}

/** A child mission as its row draws it: the joined feeds when they hold it, otherwise only what the roster reported. */
export const sidebarMissionChild = ({ agent, missionId, missions, attention }: {
  readonly agent: Agent; readonly missionId: string
  readonly missions?: readonly Mission[]; readonly attention?: readonly Attention[]
}): SidebarOtherMission => {
  const context = sidebarMissionContext({ agent, missions, attention })
  const found = context._tag === 'Mission' ? context.alsoCurrent?.find(other => other.id === missionId) : undefined
  return found ?? { id: missionId, slug: missionSlug(missionId), progress: { _tag: 'Unknown' }, step: { _tag: 'Unknown' } }
}

/** Join retained feeds only: no per-seat read, queue-size estimate, or synthetic decision handle. */
export const sidebarMissionContext = ({ agent, missions = [], attention = [] }: {
  readonly agent: Agent
  readonly missions?: readonly Mission[]
  readonly attention?: readonly Attention[]
}): SidebarMissionContext => {
  const { work, id, others } = currentMissionWork(agent)
  if (id === undefined) return { _tag: 'None' }
  const titleOf = (missionId: string, source: AgentWork | undefined) =>
    missionDisplayTitle({ title: missions.find(mission => mission.id === missionId)?.title ?? missionId.replace(/^mission\//, ''), work: source })
  const run = workRun({ mission: missions.find(mission => mission.id === id), work })
  const question = attention.find(card => isWaitingForAnswer(card) &&
    ((work !== undefined && (card.step_run_id === work.id || card.blocked?.step_run_id === work.id || card.source_id === work.id)) ||
      card.conversation_id === agent.ref || card.conversation_ids?.some(id => id === agent.ref) ||
      card.requester_id === agent.ref || card.source_id === agent.ref) &&
    (markdownSource(card.detail).trim().length > 0 || card.title.trim().length > 0))
  const current = new Set(others.map(other => other.missionId))
  const queued = new Map<string, { readonly id: string; readonly slug: string; readonly title: string }>()
  for (const upcoming of agent.upcomingWork ?? []) {
    if (upcoming.missionId === id || current.has(upcoming.missionId) || queued.has(upcoming.missionId)) continue
    queued.set(upcoming.missionId, { id: upcoming.missionId, slug: missionSlug(upcoming.missionId), title: titleOf(upcoming.missionId, upcoming) })
  }
  return {
    _tag: 'Mission', id, slug: missionSlug(id), title: titleOf(id, work),
    ...(others.length === 0 ? {} : { alsoCurrent: others.map(other => {
      const run = workRun({ mission: missions.find(mission => mission.id === other.missionId), work: other })
      return { id: other.missionId, slug: missionSlug(other.missionId), title: titleOf(other.missionId, other), progress: run === undefined ? { _tag: 'Unknown' as const } : { _tag: 'Known' as const, ...run.progress }, step: { _tag: 'Step' as const, name: workStepName(other) } }
    }) }),
    progress: run === undefined ? { _tag: 'Unknown' } : { _tag: 'Known', ...run.progress },
    eta: run === undefined || ['completed', 'failed', 'cancelled'].includes(run.status) || Option.isSome(run.completed_at)
      ? undefined : Option.getOrUndefined(run.eta),
    step: question !== undefined ? { _tag: 'Waiting', question: markdownPlainText(markdownSource(question.detail)) || question.title } :
      work === undefined ? { _tag: 'Unknown' } : { _tag: 'Step', name: workStepName(work) },
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
  subagentObservation: { _tag: 'NotReported', reason: { _tag: 'Harness' } }, children: [],
})

/** Presentation of one actual roster observation. A stale roster retains its last-observed status. */
export const sidebarRow = ({ agent, stale, needsMe, missions, attention, missionInfo, missionUnavailable = false, now = Date.now() }: {
  readonly agent: Agent; readonly stale: boolean; readonly needsMe?: boolean
  readonly missions?: readonly Mission[]; readonly attention?: readonly Attention[]
  readonly missionInfo?: SidebarMissionInfo; readonly missionUnavailable?: boolean
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
  return {
    ref: agent.ref, id: agent.ref, title: agent.name, host: agent.host, status,
    statusLabel: lastObserved ? 'Last verified · ' + statusLabel : statusLabel,
    freshness: lastObserved ? 'stale' : 'live', description: agent.description, harness: agent.harness,
    terminal: agent.terminal || undefined, mission: agent.mission, lastActivityAt, statusSince,
    context: contextOccupancy({ usage: agent.usage, now, observedAfter: agent.contextObservedAfter }),
    missionContext: missionUnavailable && (agent.currentWork?.[0]?.missionId ?? agent.mission) !== undefined
      ? { _tag: 'Unavailable' } : sidebarMissionContext({ agent, missions, attention }),
    missionInfo,
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
