import type { MissionRowAgent, MissionRowData, MissionRowTone } from '@fractal/ui/assistant-ui/shell'
import type { SidebarOtherMission } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { DateTime } from 'effect'

import { markdownPlainText } from '../../../../packages/fractal-ui/src/assistant-ui/composition/markdown-source.ts'

import type { MissionsListView } from '../state/cells/lists.ts'
import { missionWord, MISSION_WORDS, type MissionWord } from './missionWords.ts'
import { foldedInSidebar, isAdHocWork, missionEta, missionNeedsYou, missionReaderTitle, type MissionPresentation } from './presentation.ts'
import type { MissionRow } from './useMissionRows.ts'

// The one adapter from a presented mission to the kit's mission row, shared by the sidebar's Missions tab, the right
// panel's per-agent Missions section and the agent row's child missions. Grouping and sorting for the list live here too.

/** The state groups of the Missions tab, in reading order. */
export type MissionStateGroup = 'needs_you' | 'stalled' | 'working' | 'queued' | 'done'
export const STATE_GROUPS: readonly MissionStateGroup[] = ['needs_you', 'stalled', 'working', 'queued', 'done']
export const STATE_GROUP_LABEL: Record<MissionStateGroup, string> = { needs_you: 'Needs you', stalled: 'Stalled', working: 'Working', queued: 'Queued', done: 'Recently done' }

const GROUP_OF: Partial<Record<MissionWord, MissionStateGroup>> = {
  decision: 'needs_you', stalled: 'stalled', failed: 'stalled', unstaffed: 'stalled',
  working: 'working', watching: 'working',
  queued: 'queued', unclaimed: 'queued', held: 'queued', idle: 'queued', not_started: 'queued', unknown: 'queued',
  done: 'done', cancelled: 'done',
}

export const missionStateGroup = (row: MissionRow): MissionStateGroup =>
  missionNeedsYou(row) ? 'needs_you' : GROUP_OF[row.presentation.word] ?? 'queued'

const toneOf = (row: MissionRow): MissionRowTone => {
  if (missionNeedsYou(row)) return 'needs-you'
  const word = row.presentation.word
  return word === 'failed' || word === 'stalled' ? 'failed'
    : word === 'working' || word === 'watching' ? 'working'
      : word === 'done' || word === 'cancelled' ? 'settled'
        : 'waiting'
}

const capital = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** Agents holding or queued for this mission's steps, the one working first. Fractal's own steps are not agents. */
export const missionAgents = (presentation: MissionPresentation): readonly MissionRowAgent[] => {
  const seen = new Map<string, MissionRowAgent>()
  if (presentation.who.kind === 'agent' && presentation.who.agent !== undefined) seen.set(presentation.who.agent, { id: presentation.who.agent, label: presentation.who.label })
  for (const step of presentation.steps) {
    if (step.owner.kind === 'agent' && step.owner.agent !== undefined && !seen.has(step.owner.agent)) seen.set(step.owner.agent, { id: step.owner.agent, label: step.owner.label })
  }
  return [...seen.values()]
}

/** What happens now, in one line: the step waiting on you, why it is stuck, the latest progress or the step in hand. */
export const missionNowText = (presentation: MissionPresentation): string | undefined => {
  const steps = presentation.steps
  const you = steps.find(step => step.word === 'needs_you')
  if (you !== undefined) return `Waiting on you: ${you.title}`
  if (presentation.stuck !== null) {
    const stuck = steps.find(step => step.id === presentation.stuck!.step)
    if (stuck !== undefined) return presentation.word === 'failed' || stuck.word === 'failed' ? `Failed: ${stuck.title}` : stuck.note ?? `Stuck: ${stuck.title}`
  }
  if (presentation.last_progress !== null && presentation.last_progress !== '') return markdownPlainText(presentation.last_progress)
  const working = steps.find(step => step.word === 'working')
  if (working !== undefined) return working.title
  const next = steps.find(step => step.word === 'ready' || step.word === 'waiting' || step.word === 'pending')
  return next === undefined ? undefined : next.note ?? `Next: ${next.title}`
}

/** The kit row for one presented mission. */
export const missionRowData = (row: MissionRow): MissionRowData => {
  const { mission, presentation } = row
  const needsYou = missionNeedsYou(row)
  const step = missionNowText(presentation)
  return {
    id: mission.id,
    title: missionReaderTitle(mission),
    tone: toneOf(row),
    stateLabel: needsYou ? 'Needs you' : capital(missionWord(presentation.word).name),
    progress: presentation.progress.total > 0 ? { _tag: 'Known', done: presentation.progress.done, total: presentation.progress.total } : { _tag: 'Unknown' },
    ...(step === undefined ? {} : { step }),
    agents: missionAgents(presentation),
    eta: presentation.settled_at === null ? missionEta(mission) : null,
    updatedAt: DateTime.toEpochMillis(mission.updated_at),
  }
}

/** The same mission as `row-build`'s agent child row model, for the agent row and its hover card. */
export const missionChildOf = (row: MissionRow): SidebarOtherMission => {
  const data = missionRowData(row)
  const slug = row.mission.id.replace(/^mission\//, '')
  const you = row.presentation.steps.find(step => step.word === 'needs_you')
  return {
    id: row.mission.id,
    slug,
    title: data.title,
    progress: data.progress,
    step: you !== undefined ? { _tag: 'Waiting', question: you.title } : data.step === undefined ? { _tag: 'Unknown' } : { _tag: 'Step', name: data.step },
  }
}

/** The project a mission belongs to: the first segment of its path (`mission/webfractal/foundation` → `webfractal`). */
export const missionProject = (id: string): string => {
  if (isAdHocWork(id)) return 'Ad hoc work'
  const path = id.replace(/^mission\//, '').split('/')
  return path.length > 1 ? path[0]! : 'Other'
}

const etaHigh = (row: MissionRow): number => {
  const eta = row.presentation.settled_at === null ? missionEta(row.mission) : null
  return eta === null ? Number.POSITIVE_INFINITY : eta.remaining_high_ms
}
const fraction = ({ presentation: { progress } }: MissionRow) => progress.total > 0 ? progress.done / progress.total : -1
const updated = (row: MissionRow) => DateTime.toEpochMillis(row.mission.updated_at)

/** Order within a group: soonest ETA, most recent activity, or furthest along; ties keep the contract's word order. */
export const sortMissionRows = (rows: readonly MissionRow[], sort: MissionsListView['sort']): readonly MissionRow[] => {
  const word = (row: MissionRow) => MISSION_WORDS.indexOf(row.presentation.word)
  const by = sort === 'eta' ? (a: MissionRow, b: MissionRow) => etaHigh(a) - etaHigh(b) || updated(b) - updated(a)
    : sort === 'progress' ? (a: MissionRow, b: MissionRow) => fraction(b) - fraction(a) || updated(b) - updated(a)
      : (a: MissionRow, b: MissionRow) => updated(b) - updated(a)
  return [...rows].sort((a, b) => by(a, b) || word(a) - word(b))
}

export type MissionGroup = {
  /** Stable key, `state:<group>`, `project:<name>` or `agent:<id>`, stored when collapsed. */
  readonly key: string
  readonly label: string
  readonly tone?: MissionRowTone
  readonly rows: readonly MissionRow[]
}

/** Groups in display order; empty groups are dropped. A mission with several agents lists under its lead agent. */
export const groupMissionRows = (rows: readonly MissionRow[], view: Pick<MissionsListView, 'group' | 'sort'>): readonly MissionGroup[] => {
  if (view.group === 'state') {
    return STATE_GROUPS.map((group): MissionGroup => ({
      key: `state:${group}`,
      label: STATE_GROUP_LABEL[group],
      ...(group === 'needs_you' ? { tone: 'needs-you' as const } : {}),
      rows: sortMissionRows(rows.filter(row => missionStateGroup(row) === group), view.sort),
    })).filter(group => group.rows.length > 0)
  }
  const keyed = new Map<string, { label: string; rows: MissionRow[] }>()
  for (const row of rows) {
    const agent = missionAgents(row.presentation)[0]
    const [key, label] = view.group === 'project'
      ? [`project:${missionProject(row.mission.id)}`, missionProject(row.mission.id)]
      : agent === undefined ? ['agent:', 'No agent'] : [`agent:${agent.id}`, agent.label]
    const entry = keyed.get(key) ?? { label, rows: [] }
    entry.rows.push(row)
    keyed.set(key, entry)
  }
  return [...keyed.entries()]
    .map(([key, { label, rows: grouped }]) => ({ key, label, rows: sortMissionRows(grouped, view.sort) }))
    .sort((a, b) => (a.key.endsWith(':') ? 1 : 0) - (b.key.endsWith(':') ? 1 : 0) || a.label.localeCompare(b.label))
}

/** The fold: settled before today, never started, Fractal's own and quiet ad hoc work hide until "Show all". */
export const visibleMissionRows = ({ rows, showAll, current, startOfToday }: { readonly rows: readonly MissionRow[]; readonly showAll: boolean; readonly current?: string | undefined; readonly startOfToday: number }): readonly MissionRow[] =>
  showAll ? rows : rows.filter(row => row.mission.id === current || !foldedInSidebar({ ...row, startOfToday }))
