import { useScenarioNow, useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import { statuses, type AgentStatus, type SidebarAgentRow } from '../../assistant-ui/sidebar/model'
import { scenarioSections } from '../../scenario-transcript.story-harness.ts'
import { RedesignMilestone, type MilestoneConversation, type MilestoneLoad, type MilestoneRoster, type RedesignMilestoneProps } from './RedesignMilestone'

type Agent = WireSlice<'roster'>['state']['agents'][number]
const statusFor = (state: Agent['state']): AgentStatus => {
  switch (state) {
    case 'running': return 'working'
    case 'waiting': return 'waiting'
    case 'desired': case 'starting': return 'pending'
    case 'down': return 'offline'
    case 'stopped': case 'failed': return 'ended'
    case 'suspended': return 'suspended'
    case 'retired': return 'retired'
    default: return 'unobserved'
  }
}

/** No hydrated run summaries in the roster wire contract: metrics and last turn remain unknown. */
export const milestoneRoster = (slice: WireSlice<'roster'>, query: string): MilestoneRoster => {
  const hosts = new Map(slice.state.machines.map(machine => [machine.host_id, machine.name]))
  const rank = new Map(slice.state.order.map((id, index) => [id, index]))
  const rows = [...slice.state.agents].sort((a, b) => (rank.get(a.id) ?? rank.size) - (rank.get(b.id) ?? rank.size)).map((agent): SidebarAgentRow => {
    const status = statusFor(agent.state)
    const current = agent.current_work?.[0]
    return {
      ref: agent.id, id: agent.id, title: agent.name, status, statusLabel: statuses[status].label,
      host: agent.host_id == null ? 'Unknown host' : hosts.get(agent.host_id) ?? agent.host_id,
      freshness: 'live', usage: { _tag: 'Unknown' }, duration: { _tag: 'Unknown' }, lastTurn: { _tag: 'Unknown' }, children: [],
      missionContext: current === undefined ? { _tag: 'None' } : { _tag: 'Mission', id: current.mission_id, slug: current.mission_id.split('/').at(-1) ?? current.mission_id, progress: { _tag: 'Unknown' }, step: { _tag: 'Step', name: current.title || current.path }, queued: [] },
    }
  })
  return { folders: rows.length === 0 ? [] : [{ id: 'scenario-agents', label: 'Agents', rows }], inboxCount: 0, query }
}

const load = <TValue,>(value: TValue, loading: boolean, status: WireSlice<'sync'>['status'][string] | undefined, now: number): MilestoneLoad<TValue> => {
  if (loading || status?._tag === 'Connecting' || status?._tag === 'Requested' || status?._tag === 'Progress') return { _tag: 'loading', value }
  if (status?._tag === 'Failed') return { _tag: 'failed', value, reason: 'Synchronization failed', detail: JSON.stringify(status.cause) }
  if (status?._tag === 'Stale') return { _tag: 'stale', value, observedAt: status.lastLiveAt ?? now }
  return { _tag: 'ready', value }
}

export interface MilestoneScenarioArgs {
  readonly scheme: RedesignMilestoneProps['scheme']
  readonly state: 'slice' | 'empty-thread' | 'no-match' | 'navigation'
  readonly conversation?: string
  readonly roster?: string
}

export function ScenarioMilestone({ scheme, state, persistKey = 'rm1-scenario' }: MilestoneScenarioArgs & { readonly persistKey?: string }) {
  const rosterSlice = useScenarioSlice('roster')
  const conversationSlice = useScenarioSlice('conversation')
  const sync = useScenarioSlice('sync')
  const now = useScenarioNow()
  const rosterValue = milestoneRoster(rosterSlice, state === 'no-match' ? 'zz-nothing' : '')
  const rows = rosterValue.folders.flatMap(folder => folder.rows)
  // Whole threads cross the seam; Transcript owns the mounted window, not this adapter.
  const sections = scenarioSections(conversationSlice, 'whole-thread').sections
  const threads = Object.fromEntries(rows.map(row => {
    const section = sections.find(section => section.id === `${row.ref}/0`)
    const turns = section?.turns ?? []
    return [row.ref, { turns, history: section?.history, running: turns.some(turn => turn.work.running) }]
  }))
  const primaryRef = rows.find(row => (threads[row.ref]?.turns.length ?? 0) > 0)?.ref ?? rows[0]?.ref ?? ''
  const reviewerRef = rows.find(row => row.ref !== primaryRef)?.ref ?? primaryRef
  const conversationStatus = sync.status[`conversation:${primaryRef}`]
  const observedAt = conversationStatus?._tag === 'Stale' ? conversationStatus.lastLiveAt ?? now : now
  const conversationValue: MilestoneConversation = { primaryRef, reviewerRef, threads, sync: conversationStatus, lastSeen: { at: observedAt, activity: rows.find(row => row.ref === primaryRef)?.statusLabel ?? 'Unobserved', detail: threads[primaryRef]?.turns.at(-1)?.work.calls.at(-1)?.argsSummary ?? '' } }
  const roster = load(rosterValue, rosterSlice.loading, sync.status.agents, now)
  const conversation = load(conversationValue, conversationSlice.loading, conversationStatus, now)
  const emptyRef = rows.find(row => threads[row.ref]?.turns.length === 0)?.ref
  const openRefs = rows.length === 0 ? [] : state === 'empty-thread' && emptyRef !== undefined ? [emptyRef] : [primaryRef, reviewerRef].filter((ref, index, refs) => refs.indexOf(ref) === index)
  const navigationRow = rows.find(row => row.ref !== primaryRef) ?? rows[0]
  return <div data-scenario-navigation-title={navigationRow?.title} data-scenario-primary-title={rows.find(row => row.ref === primaryRef)?.title}>
    <RedesignMilestone key={`${rows.map(row => row.ref).join(',')}:${rosterSlice.variant}:${conversationSlice.variant}:${sync.variant}:${state}:${scheme}`} scheme={scheme} now={now} roster={roster} conversation={conversation}
      changedFiles={[]} branchFiles={[]} proseResources={{}} reviewerCard={{ path: '', summary: '' }} defaultOpenRefs={openRefs} defaultChangesOpen={state === 'slice' || state === 'navigation'} defaultPaletteOpen={state === 'navigation'} persistKey={persistKey} />
  </div>
}
