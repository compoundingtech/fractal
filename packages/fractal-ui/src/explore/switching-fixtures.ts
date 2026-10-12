/** One fixture feeds the sidebar rows, the switchers and the probe, so their counts always agree. */
export type AgentStatus = 'Running' | 'Finished' | 'Failed' | 'Needs you'
export interface Agent { readonly id: string; readonly name: string; readonly parent?: string; readonly status: AgentStatus; readonly unread: number; readonly task: string }
export const scenarios = ['zero', 'one', 'running', 'many', 'failed', 'attention', 'nested', 'finished'] as const
export type Scenario = typeof scenarios[number]
export interface WorkspaceFixture { readonly state: Scenario; readonly label: string; readonly agents: readonly Agent[] }

const names = ['Layout', 'Keyboard', 'Coverage', 'Copy', 'Routing', 'Storage', 'History', 'Focus', 'Overflow', 'Messages', 'Resources', 'Review', 'Delivery', 'Recovery']
function childStatus(state: Scenario, index: number): AgentStatus {
  if (state === 'finished') return 'Finished'
  if (index > 0) return 'Running'
  return state === 'failed' ? 'Failed' : state === 'attention' ? 'Needs you' : 'Running'
}
function workspace(state: Scenario, label: string, count: number): WorkspaceFixture {
  const root: Agent = { id: 'root', name: label, status: 'Running', unread: 0, task: 'Coordinate the workspace navigation changes.' }
  const children = names.slice(0, count).map((name, index): Agent => ({ id: `child-${index + 1}`, name, parent: 'root', status: childStatus(state, index), unread: state === 'attention' ? (index === 0 ? 1 : 3) : 0, task: `Check ${name.toLowerCase()} behaviour in the workspace navigation.` }))
  const nested: Agent[] = state === 'nested' ? [
    { id: 'grandchild', name: 'Key map', parent: 'child-1', status: 'Running', unread: 2, task: 'Check the key map delegated by Layout.' },
    { id: 'great-grandchild', name: 'Shortcut audit', parent: 'grandchild', status: 'Finished', unread: 1, task: 'Audit the shortcuts delegated by Key map.' },
  ] : []
  return { state, label, agents: [root, ...children, ...nested] }
}
export const workspaces: Record<Scenario, WorkspaceFixture> = {
  zero: workspace('zero', 'Notes', 0),
  one: workspace('one', 'Navigation', 1),
  running: workspace('running', 'Workspace', 3),
  many: workspace('many', 'Release', 14),
  failed: workspace('failed', 'Diagnostics', 3),
  attention: workspace('attention', 'Decisions', 3),
  nested: workspace('nested', 'Key map review', 3),
  finished: workspace('finished', 'Completed review', 3),
}
export const sidebarFixtures: readonly WorkspaceFixture[] = scenarios.map(state => workspaces[state])

const subagents = (fixture: WorkspaceFixture) => fixture.agents.filter(agent => agent.parent !== undefined)
/** Active means running or waiting on the operator; finished and failed subagents are not active. */
export const activeSubagents = (fixture: WorkspaceFixture) => subagents(fixture).filter(agent => agent.status === 'Running' || agent.status === 'Needs you').length
export const failedSubagents = (fixture: WorkspaceFixture) => subagents(fixture).filter(agent => agent.status === 'Failed').length
export const waitingSubagents = (fixture: WorkspaceFixture) => subagents(fixture).filter(agent => agent.status === 'Needs you').length
/** The count control's accessible name, in words. The visible count text is always a prefix of it. */
export function activeCountName(fixture: WorkspaceFixture): string {
  const active = activeSubagents(fixture)
  const failed = failedSubagents(fixture)
  const waiting = waitingSubagents(fixture)
  return [`${active} active ${active === 1 ? 'subagent' : 'subagents'}`, ...(failed > 0 ? [`${failed} failed`] : []), ...(waiting > 0 ? [`${waiting} ${waiting === 1 ? 'needs' : 'need'} you`] : [])].join(', ')
}
export const depthOf = (agent: Agent, agents: readonly Agent[]): number => agent.parent === undefined ? 0 : 1 + depthOf(agents.find(candidate => candidate.id === agent.parent)!, agents)
