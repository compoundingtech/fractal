export type Agent = { id: string; name: string; parent?: string; status: 'Running' | 'Finished' | 'Failed' | 'Needs you'; unread: number; task: string }
export const scenarios = ['running', 'finished', 'failed', 'many', 'nested', 'attention', 'zero', 'one'] as const
export type Scenario = typeof scenarios[number]
const names = ['Layout', 'Keyboard', 'Coverage', 'Copy', 'Routing', 'Storage', 'History', 'Focus', 'Overflow', 'Messages', 'Resources', 'Review', 'Delivery', 'Recovery']
export interface WorkspaceFixture { state: Scenario; label: string; agents: readonly Agent[] }
function workspace(state: Scenario, label: string, count: number): WorkspaceFixture {
  const root: Agent = { id: 'root', name: label, status: 'Running', unread: 0, task: 'Coordinate the workspace navigation changes.' }
  const children: Agent[] = names.slice(0, count).map((name, index) => ({ id: `child-${index + 1}`, name, parent: 'root', status: state === 'finished' ? 'Finished' : state === 'failed' && index === 0 ? 'Failed' : state === 'attention' && index === 0 ? 'Needs you' : 'Running', unread: state === 'attention' ? index === 0 ? 1 : 3 : 0, task: `Check ${name.toLowerCase()} behaviour in the workspace navigation.` }))
  if (state === 'nested') children.push({ id: 'grandchild', name: 'Key map', parent: 'child-1', status: 'Running', unread: 2, task: 'Check the key map delegated by Layout.' }, { id: 'great-grandchild', name: 'Shortcut audit', parent: 'grandchild', status: 'Finished', unread: 1, task: 'Audit the shortcuts delegated by Key map.' })
  return { state, label, agents: [root, ...children] }
}
export const workspaces: Record<Scenario, WorkspaceFixture> = {
  zero: workspace('zero', 'Notes', 0), one: workspace('one', 'Navigation', 1), running: workspace('running', 'Workspace', 3), many: workspace('many', 'Release', 14), failed: workspace('failed', 'Diagnostics', 3), attention: workspace('attention', 'Decisions', 3), nested: workspace('nested', 'Key map review', 3), finished: workspace('finished', 'Completed review', 3),
}
export const sidebarFixtures = [workspaces.zero, workspaces.one, workspaces.running, workspaces.many, workspaces.failed, workspaces.attention, workspaces.nested, workspaces.finished]
export const activeSubagents = (fixture: WorkspaceFixture) => fixture.agents.filter(agent => agent.parent !== undefined && (agent.status === 'Running' || agent.status === 'Needs you')).length
