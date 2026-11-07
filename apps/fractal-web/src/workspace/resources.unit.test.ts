import { it } from '@effect/vitest'
import { Arbitrary, Schema } from 'effect'
import { describe, expect } from 'vitest'
import { activeTab, emptyWorkspace, tabsOf, transition, visibleTabIds, type Tab, type WorkspaceState } from './model.ts'
import { corePolicies, type DataResource } from './registry.ts'
import { defaultBudget, desiredHolds, makeResourceManager, planTiers, resourceKey, type Budget } from './resources.ts'
import { parsePane } from './subject.ts'

const between = (minimum: number, maximum: number) => Schema.Int.check(Schema.isBetween({ minimum, maximum }))
const agents = Array.from({ length: 12 }, (_, index) => `agent:agent/a${index}`)
const panes = [...agents, 'terminal:agent/a0', 'terminal:agent/a1', 'list:home', 'list:missions', 'mission:mission/m', 'usage:', 'document:doc/x', 'declaration:agent/a0', 'resource:mail/1', 'machine:machine/m']

const pane = Arbitrary.schema(Schema.Literals(panes))

const counter = () => {
  let next = 0
  return { newId: () => `tab-${next++}` }
}

const workspace = (open: readonly string[], active = open.at(-1)): WorkspaceState => {
  const context = counter()
  let state = emptyWorkspace
  for (const pane of open) state = transition(state, { _tag: 'Open', pane }, context)
  return active === undefined ? state : transition(state, { _tag: 'Open', pane: active }, context)
}

const holdsOf = (state: WorkspaceState) => desiredHolds({ tabs: tabsOf(state.layout), tiers: planTiers({ state, budget: defaultBudget }), views: state.views })

const fakeDriver = () => {
  const mounts = new Map<string, number>()
  let total = 0
  return {
    mounts,
    total: () => total,
    driver: {
      acquire: (resource: DataResource) => {
        const key = resourceKey(resource)
        mounts.set(key, (mounts.get(key) ?? 0) + 1)
        total += 1
        return () => mounts.set(key, mounts.get(key)! - 1)
      },
    },
  }
}

describe('planTiers', () => {
  const scenario = Arbitrary.all({
    open: Arbitrary.array(pane, { minLength: 1, maxLength: panes.length }).pipe(Arbitrary.map(open => [...new Set(open)])),
    dom: Arbitrary.schema(between(0, 6)),
    data: Arbitrary.schema(between(0, 12)),
  })

  it.prop('always shows the active tab and respects both budgets', { scenario }, ({ scenario: { open, dom, data } }) => {
    const state = workspace(open)
    const budget: Budget = { dom, data }
    const tiers = planTiers({ state, budget })
    const tabs = tabsOf(state.layout)
    expect(tiers.get(activeTab(state)!.id)).toBe('active')
    expect(tiers.size).toBe(tabs.length)
    const hidden = tabs.filter(tab => tab.id !== activeTab(state)!.id)
    const cost = (tab: Tab, field: 'domCost' | 'dataCost') => corePolicies[parsePane(tab.pane)._tag][field]
    expect(hidden.filter(tab => tiers.get(tab.id) === 'mounted').reduce((sum, tab) => sum + cost(tab, 'domCost'), 0)).toBeLessThanOrEqual(dom)
    expect(hidden.filter(tab => ['mounted', 'warm'].includes(tiers.get(tab.id)!)).reduce((sum, tab) => sum + cost(tab, 'dataCost'), 0)).toBeLessThanOrEqual(data)
  })

  it('evicts the least recently used conversation first, keeping the tab', () => {
    const state = workspace(agents)
    const tiers = planTiers({ state, budget: defaultBudget })
    expect(state.mru.map(id => tiers.get(id))).toEqual(['active', 'mounted', 'mounted', 'mounted', 'warm', 'warm', 'warm', 'warm', 'warm', 'cold', 'cold', 'cold'])
    expect(tabsOf(state.layout)).toHaveLength(12)
  })

  it('never evicts a surface holding an unsent draft, and other tabs give way', () => {
    const state = workspace(agents)
    const oldest = state.mru.at(-1)!
    const tiers = planTiers({ state: { ...state, views: { [oldest]: { dirty: true } } }, budget: { dom: 1, data: 8 } })
    expect(tiers.get(oldest)).toBe('mounted')
    expect(tiers.get(state.mru[1]!)).toBe('mounted')
    expect(tiers.get(state.mru[2]!)).toBe('warm')
  })

  it('shows the active tab of every split, whatever the focus', () => {
    const context = counter()
    let state = workspace(['agent:agent/a', 'agent:agent/b', 'agent:agent/c'])
    state = transition(state, { _tag: 'Move', id: tabsOf(state.layout)[0]!.id, to: { _tag: 'Edge', group: state.focus, side: 'right', first: false } }, { newId: () => `split-${context.newId()}` })
    const tiers = planTiers({ state, budget: { dom: 0, data: 0 } })
    for (const id of visibleTabIds(state.layout)) expect(tiers.get(id)).toBe('active')
    expect(visibleTabIds(state.layout).size).toBe(2)
  })

  it('holds a terminal stream only while that face is on screen, and the conversation at any face', () => {
    let state = workspace(['terminal:agent/a0', 'agent:agent/a1'], 'agent:agent/a1')
    const terminal = tabsOf(state.layout)[0]!
    expect(terminal.pane).toBe('agent:agent/a0')
    expect(holdsOf(state).get(terminal.id)).toEqual([{ _tag: 'Conversation', ref: 'agent/a0' }])
    state = transition(state, { _tag: 'Activate', id: terminal.id }, counter())
    expect(holdsOf(state).get(terminal.id)).toEqual([{ _tag: 'Conversation', ref: 'agent/a0' }, { _tag: 'TerminalScreen', ref: 'terminal/a0' }])
    state = transition(state, { _tag: 'SetView', id: terminal.id, view: { terminal: false } }, counter())
    expect(holdsOf(state).get(terminal.id)).toEqual([{ _tag: 'Conversation', ref: 'agent/a0' }])
  })
})

describe('resource manager', () => {
  it('mounts a shared feed once however many tabs hold it, and releases it with the last', () => {
    const fake = fakeDriver()
    const manager = makeResourceManager(fake.driver)
    manager.reconcile(holdsOf(workspace(['list:missions', 'mission:mission/m', 'list:home'])))
    expect(fake.mounts.get('shared:attention')).toBe(1)
    expect(manager.stats().refs).toBeGreaterThan(manager.stats().held)
    manager.dispose()
    expect([...fake.mounts.values()].every(count => count === 0)).toBe(true)
    expect(manager.stats().held).toBe(0)
  })

  it.prop('stays balanced and bounded through any sequence of switches', { sequence: Arbitrary.array(pane, { minLength: 1, maxLength: 60 }) }, ({ sequence }) => {
    const fake = fakeDriver()
    const manager = makeResourceManager(fake.driver)
    const context = counter()
    let state = emptyWorkspace
    for (const pane of sequence) {
      state = transition(state, { _tag: 'Open', pane }, context)
      manager.reconcile(holdsOf(state))
      for (const count of fake.mounts.values()) expect(count === 0 || count === 1).toBe(true)
      // The budget, the one visible subject, and a terminal on screen.
      expect(manager.stats().follows).toBeLessThanOrEqual(defaultBudget.data + 2)
    }
    manager.dispose()
    for (const count of fake.mounts.values()) expect(count).toBe(0)
    expect(manager.stats().acquired).toBe(manager.stats().released)
  })

  it('keeps a hidden mounted document held, so hiding and reactivating it neither releases nor reacquires', () => {
    const fake = fakeDriver()
    const manager = makeResourceManager(fake.driver)
    let state = workspace(['document:doc/x'])
    manager.reconcile(holdsOf(state))
    const document = tabsOf(state.layout)[0]!
    expect(fake.mounts.get('Document:doc/x')).toBe(1)
    state = transition(state, { _tag: 'Open', pane: 'agent:agent/a0' }, { newId: () => 'tab-agent' })
    expect(planTiers({ state, budget: defaultBudget }).get(document.id)).toBe('mounted')
    manager.reconcile(holdsOf(state))
    expect(fake.mounts.get('Document:doc/x')).toBe(1)
    const { acquired, released } = manager.stats()
    state = transition(state, { _tag: 'Activate', id: document.id }, counter())
    manager.reconcile(holdsOf(state))
    expect(fake.mounts.get('Document:doc/x')).toBe(1)
    expect(manager.stats().released).toBe(released)
    expect(manager.stats().acquired).toBe(acquired)
  })

  it('counts a hidden mounted document, declaration or resource against the data budget', () => {
    const fake = fakeDriver()
    const manager = makeResourceManager(fake.driver)
    manager.reconcile(holdsOf(workspace([...agents, 'resource:mail/1', 'declaration:agent/a0', 'document:doc/x', 'agent:agent/a0'])))
    expect(manager.stats().follows).toBeLessThanOrEqual(defaultBudget.data + 1)
  })

  it('does not churn a socket when activating a warm tab', () => {
    const fake = fakeDriver()
    const manager = makeResourceManager(fake.driver)
    let state = workspace(agents.slice(0, 5))
    manager.reconcile(holdsOf(state))
    const before = fake.total()
    state = transition(state, { _tag: 'Activate', id: tabsOf(state.layout)[0]!.id }, counter())
    manager.reconcile(holdsOf(state))
    expect(fake.total()).toBe(before)
  })
})
