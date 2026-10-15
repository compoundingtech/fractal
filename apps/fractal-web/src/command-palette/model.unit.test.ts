import { describe, expect, it } from 'vitest'
import type { Agent } from '../data/source.ts'
import type { SidebarNode } from '../folders/sidebarContract.ts'
import { searchAgents } from '../web/sidebarSearch.fixtures.ts'
import { agentFolders, commandSections, leadingRows, recentLimit, recordVisit, switchSections, type PaletteSection } from './model.ts'

// Names, folders and hosts as observed in the live roster on 2026-10-12.
const live = [
  ['agent/fleet-coordinator', 'Fleet coordinator', 'desk3'],
  ['agent/fleet-storage', 'Fleet storage steward', 'desk1'],
  ['agent/global-complexity', 'Global complexity orchestrator', 'desk1'],
  ['agent/agent-attention', 'Agent attention', 'desk1'],
  ['agent/buck2-lead', 'Buck2 migration lead', 'desk1'],
  ['agent/graph-steward', 'Graph-as-code steward', 'desk3'],
  ['agent/janitor', 'Janitor', 'desk1'],
] as const
const agents: readonly Agent[] = live.map(([ref, name, host]) => ({ ...searchAgents[0]!, ref, name, host, mission: undefined }))
const tree: readonly SidebarNode[] = [
  { _tag: 'Folder', id: 'fleet', label: 'Fleet', collapsed: false, children: [{ _tag: 'Folder', id: 'manager', label: 'manager', collapsed: true, children: [
    { _tag: 'Agent', id: 'a1', subject: 'agent/fleet-coordinator', label: '' },
    { _tag: 'Agent', id: 'a2', subject: 'agent/fleet-storage', label: '' },
  ] }, { _tag: 'Agent', id: 'a3', subject: 'agent/janitor', label: '' }] },
  { _tag: 'Folder', id: 'system', label: 'Agent system', collapsed: false, children: [{ _tag: 'Folder', id: 'control', label: 'Agent control', collapsed: false, children: [
    { _tag: 'Agent', id: 'a4', subject: 'agent/global-complexity', label: '' },
    { _tag: 'Agent', id: 'a5', subject: 'agent/agent-attention', label: '' },
  ] }] },
  { _tag: 'Agent', id: 'a6', subject: 'agent/buck2-lead', label: '' },
]
const folders = agentFolders(tree)
const titles = (sections: readonly PaletteSection[]) => sections.map(section => [section.id, section.rows.map(row => row.title)])
const sections = (overrides: Partial<Parameters<typeof switchSections>[0]> = {}) => switchSections({
  query: '', agents, folders, needsYou: new Set(), recents: [], current: '', missions: undefined, statusLabel: agent => agent.status, ...overrides,
})

describe('switch results over live names', () => {
  it('pins Needs you, then recent visits newest first, then the rest by name', () => {
    const result = sections({ needsYou: new Set(['agent/janitor']), recents: ['agent/fleet-storage', 'agent/agent-attention', 'agent/janitor'], current: 'agent/fleet-storage' })
    expect(titles(result)).toEqual([
      ['needs-you', ['Janitor']],
      ['recent', ['Fleet storage steward', 'Agent attention']],
      ['agents', ['Buck2 migration lead', 'Fleet coordinator', 'Global complexity orchestrator', 'Graph-as-code steward']],
    ])
    expect(result[1]!.rows[0]!.marks).toEqual(['Current', 'Recent'])
  })
  it('ranks word prefixes over initials and substrings, with folder paths as context', () => {
    // "fl st": both words start "Fleet storage steward"; "Fleet coordinator" has no word starting with "st".
    expect(titles(sections({ query: 'fl st' }))).toEqual([['matches', ['Fleet storage steward']]])
    // "co": a word prefix in three names, then only the folder word "control" for Agent attention.
    expect(titles(sections({ query: 'co' }))).toEqual([['matches', ['Fleet coordinator', 'Global complexity orchestrator', 'Graph-as-code steward', 'Agent attention']]])
    const control = sections({ query: 'agent con' })[0]!.rows
    expect(control.map(row => row.title)).toEqual(['Agent attention', 'Global complexity orchestrator'])
    expect(control[1]!.context[0]).toEqual({ text: 'Agent system / Agent control', highlights: [0, 1, 2, 3, 4, 21, 22, 23] })
    expect(control[1]!.context).toHaveLength(2)
  })
  it('shows a ref only when it supplies a token not matched as well by earlier fields', () => {
    const fleet = sections({ query: 'fleet' })[0]!.rows
    expect(fleet.every(row => row.context.every(segment => !segment.text.startsWith('fleet-')))).toBe(true)
    const byRef = sections({ query: 'storage' })[0]!.rows[0]!
    expect(byRef.context.map(segment => segment.text)).not.toContain('fleet-storage')
    const refOnly = sections({ query: 'buck2-lead' })[0]!.rows[0]!
    expect(refOnly.context.map(segment => segment.text)).toContain('buck2-lead')
    const mixed = sections({ query: 'buck2 buck2-lead' })[0]!.rows[0]!
    expect(mixed.context.map(segment => segment.text)).toContain('buck2-lead')
  })
  it('keeps a matching urgent agent above a better match and drops unrelated urgent agents', () => {
    const result = sections({ query: 'co', needsYou: new Set(['agent/agent-attention', 'agent/janitor']) })
    expect(titles(result)).toEqual([['needs-you', ['Agent attention']], ['matches', ['Fleet coordinator', 'Global complexity orchestrator', 'Graph-as-code steward']]])
  })
  it('breaks score ties by recency, then name', () => {
    // Janitor matches through its Fleet folder; a name match outranks the same tier in a later field.
    expect(sections({ query: 'fleet' })[0]!.rows.map(row => row.title)).toEqual(['Fleet coordinator', 'Fleet storage steward', 'Janitor'])
    expect(sections({ query: 'fleet', recents: ['agent/fleet-storage'] })[0]!.rows.map(row => row.title)).toEqual(['Fleet storage steward', 'Fleet coordinator', 'Janitor'])
  })
  it('opens a mission through the agent last visited for it and disables missions without an agent', () => {
    const working = agents.map(agent => agent.ref === 'agent/janitor' || agent.ref === 'agent/fleet-storage' ? { ...agent, mission: 'mission/ops/disk' } : agent)
    const missions = [{ id: 'mission/ops/disk', title: 'Disk steward sweep', state: 'active' }, { id: 'mission/idle', title: 'Disk archive', state: { raw: 'ready' } }]
    const result = sections({ agents: working, missions, query: 'disk', recents: ['agent/janitor'] })
    expect(result.map(section => section.id)).toEqual(['missions'])
    expect(result[0]!.rows.map(row => [row.title, row.target, row.disabledReason !== undefined])).toEqual([
      ['Disk steward sweep', { _tag: 'Mission', id: 'mission/ops/disk', agentRef: 'agent/janitor' }, false],
      ['Disk archive', { _tag: 'Mission', id: 'mission/idle', agentRef: undefined }, true],
    ])
    expect(result[0]!.rows[0]!.context[0]!.text).toBe('mission · Janitor +1')
  })
})

describe('palette state', () => {
  it('records visits newest first, unique and bounded', () => {
    expect(recordVisit(['a', 'b'], 'b')).toEqual(['b', 'a'])
    expect(recordVisit(['a'], '')).toEqual(['a'])
    expect(recordVisit(Array.from({ length: recentLimit }, (_, index) => `agent/${index}`), 'agent/new')).toHaveLength(recentLimit)
  })
  it('cuts only the tail for the first frame and keeps complete counts', () => {
    const all = sections({ needsYou: new Set(['agent/janitor']), recents: ['agent/fleet-storage'] })
    const first = leadingRows(all, 3)
    expect(titles(first)).toEqual([['needs-you', ['Janitor']], ['recent', ['Fleet storage steward']], ['agents', ['Agent attention']]])
    expect(first.map(section => section.total ?? section.rows.length)).toEqual([1, 1, 5])
  })
  it('groups commands by category and keeps a disabled reason', () => {
    const result = commandSections({ query: 'term', commands: [
      { id: 'terminal.toggle', title: 'Open terminal', detail: 'Janitor', category: 'View', disabledReason: 'This agent has no observed terminal.' },
      { id: 'theme.toggle', title: 'Use light theme', detail: 'This device', category: 'View' },
    ] })
    expect(result.map(section => [section.id, section.rows.map(row => [row.title, row.disabledReason])])).toEqual([['View', [['Open terminal', 'This agent has no observed terminal.']]]])
  })
})
