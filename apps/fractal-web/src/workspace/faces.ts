import type { ResourceObservation } from '@fractal/client/schema'
import type { IconName } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { resourceState, resourceTitle } from '../data/agentResources/model.ts'
import type { ThingAct, ThingFace } from '../../../../packages/fractal-ui/src/assistant-ui/things/face.ts'
import type { Agent, AgentSubagent } from '../data/source.ts'

/** Only observed state and causes supply marks and act labels; identity is not status. */
export const agentFace = (agent: Agent, attention?: { readonly label: string }): ThingFace => ({
  kind: 'Agent', icon: 'person', title: agent.name,
  mark: { shape: agent.endedAt._tag === 'Known' ? 'ended' : agent.activity === 'working' ? 'filled' : 'hollow', word: agent.status },
  ...(agent.activity === 'errored' ? { act: { tone: 'failed', label: agent.blockedOn._tag === 'Known' ? agent.blockedOn.value : agent.status } as const }
    : attention !== undefined ? { act: { tone: 'act', label: attention.label } as const } : {}),
  ...(agent.lastActivityAt._tag === 'Known' ? { at: { at: agent.lastActivityAt.value, verb: 'active' } } : {}),
  ...(agent.description === undefined ? {} : { summary: agent.description }),
})

export const missionFace = (mission: { readonly title: string; readonly state?: string; readonly updatedAt?: number }, act?: ThingAct): ThingFace => ({
  kind: 'Mission', icon: 'checklist', title: mission.title,
  mark: { shape: mission.state === undefined ? 'none' : mission.state === 'running' || mission.state === 'active' ? 'filled' : mission.state === 'completed' || mission.state === 'cancelled' ? 'ended' : 'hollow', word: mission.state ?? '' },
  ...(act === undefined ? {} : { act }),
  ...(mission.updatedAt === undefined ? {} : { at: { at: mission.updatedAt, verb: 'updated' } }),
})

export const terminalFace = (terminal: { readonly title: string; readonly state?: string; readonly startedAt?: number; readonly exitCode?: number }): ThingFace => ({
  kind: 'Terminal', icon: 'terminal', title: terminal.title,
  mark: { shape: terminal.exitCode !== undefined ? 'ended' : terminal.state === undefined ? 'none' : terminal.state === 'running' ? 'filled' : 'hollow', word: terminal.exitCode === undefined ? terminal.state ?? '' : `exit ${terminal.exitCode}` },
  ...(terminal.exitCode !== undefined && terminal.exitCode !== 0 ? { act: { tone: 'failed', label: `exit ${terminal.exitCode}` } as const } : {}),
  ...(terminal.startedAt === undefined ? {} : { at: { at: terminal.startedAt, verb: 'started' } }),
})

export const subagentFace = (subagent: AgentSubagent): ThingFace => ({
  kind: 'Subagent', icon: 'person', title: subagent.description ?? subagent.subagent_type ?? 'Subagent',
  // The roster reports identity and a lease, not the child's current activity.
  mark: { shape: 'none', word: '' },
  ...(subagent.started_at === undefined ? {} : { at: { at: Date.parse(subagent.started_at), verb: 'started' } }),
})

export const workspaceFace = (path: string): ThingFace => ({ kind: 'Workspace', icon: 'folder', title: path, titleStyle: 'path', mark: { shape: 'none', word: '' } })
export const worktreeFace = (path: string): ThingFace => ({ ...workspaceFace(path), kind: 'Worktree' })
/** Declared category names only: targets are intentionally withheld by the source. */
export const bindingFace = (name: string): ThingFace => ({ kind: 'Declared binding', icon: 'attach', title: name, mark: { shape: 'none', word: '' } })

const resourceKinds: Readonly<Record<string, { readonly icon: IconName; readonly label: string }>> = {
  'vcs.pull-request': { icon: 'pull-request', label: 'Pull request' },
  'vcs.issue': { icon: 'alert', label: 'Issue' },
  'vcs.commit': { icon: 'dot', label: 'Commit' },
  'vcs.ref': { icon: 'swap', label: 'Branch' },
  'vcs.repository': { icon: 'folder', label: 'Repository' },
  'ci.run': { icon: 'check', label: 'CI run' },
  'filesystem.file': { icon: 'pencil', label: 'File' },
  'human.review': { icon: 'check', label: 'Review' },
  'uri.reference': { icon: 'globe', label: 'Link' },
  'harness.session-file': { icon: 'message', label: 'Session file' },
  arrangement: { icon: 'checklist', label: 'Arrangement' },
}

export const observedResourceFace = (resource: ResourceObservation): ThingFace => {
  const kind = resourceKinds[resource.kind] ?? { icon: 'info', label: 'Resource' }
  const state = resourceState(resource)
  const facts = resource.facts
  const conclusion = typeof facts.conclusion === 'string' ? facts.conclusion : state
  const exit = typeof facts.exit_code === 'number' ? facts.exit_code : undefined
  const failure = exit !== undefined && exit !== 0 ? `exit ${exit}`
    : facts.review_decision === 'changes_requested' ? 'changes requested'
      : facts.merge_state === 'unstable' || facts.checks === 'failing' ? 'checks failing'
        : conclusion !== undefined && ['failure', 'failed', 'errored', 'timed_out', 'startup_failure', 'action_required'].includes(conclusion) ? conclusion.replaceAll('_', ' ') : undefined
  const running = state !== undefined && ['running', 'in_progress', 'active', 'working'].includes(state)
  const ended = state !== undefined && ['closed', 'merged', 'completed', 'cancelled', 'exited'].includes(state)
  const moment = resource.kind === 'ci.run' ? running ? facts.started_at : facts.completed_at : facts.updated_at
  const at = typeof moment === 'string' ? Date.parse(moment) : typeof moment === 'number' ? moment : undefined
  const spoken = ['vcs.pull-request', 'vcs.issue', 'ci.run'].includes(resource.kind) && (typeof facts.number === 'number' || typeof facts.number === 'string') ? `#${facts.number}` : undefined
  const title = resourceTitle(resource)
  return {
    kind: kind.label, icon: kind.icon, title: title === resource.id ? kind.label : title,
    ...(typeof facts.path === 'string' && title === facts.path ? { titleStyle: 'path' as const } : {}),
    ...(spoken === undefined ? {} : { id: spoken }),
    mark: { shape: state === undefined ? 'none' : running ? 'filled' : ended || failure !== undefined ? 'ended' : 'hollow', word: state?.replaceAll('_', ' ') ?? '' },
    ...(failure === undefined ? {} : { act: { tone: 'failed', label: failure } as const }),
    // observed_at is receipt evidence, not a claim about when the thing changed.
    ...(at === undefined || !Number.isFinite(at) ? {} : { at: { at, verb: resource.kind === 'ci.run' ? running ? 'started' : 'finished' : 'updated' } }),
  }
}
