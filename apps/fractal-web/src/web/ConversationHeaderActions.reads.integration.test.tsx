// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Atom from 'effect/reactivity/Atom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ResourceObservation, decodeUnknownSync } from '@fractal/client/schema'
import { inspectorViewPreference } from './conversationPreferences.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { gatewayResources } from '../resources/agent/source.ts'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'
import { AgentDetailsPanel } from './AgentDetailsPanel.tsx'
import { retainedResourceAtoms } from './AgentResourcesPanel.tsx'
import { AgentTodos } from '../conversation/todos/AgentTodos.tsx'
import { todoAgent, todoAgentRef } from '../conversation/todos/fixtures.ts'
import { projectTodos } from '../conversation/todos/model.ts'
import { observed } from '../data/source.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const snapshot = { id: 'snapshot/example', host_id: 'host/example', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-10-09T00:00:00Z' }
let root: Root
let host: HTMLDivElement
let registry: AtomRegistry.AtomRegistry
const requests: string[] = []
const fetchImpl: typeof fetch = async input => {
  const path = new URL(String(input)).pathname
  requests.push(path)
  const value = path === '/v1/client/capabilities'
    ? { kind: 'capabilities', capabilities: [], limits: { max_page_items: 100, max_event_items: 100, max_wait_ms: 1000, max_response_bytes: 65536 }, schemas: [] }
    : { kind: 'resources-page', items: [], page: { limit: 50, has_more: false, next_cursor: null } }
  return new Response(JSON.stringify({ api_version: 'st3.client.v0', snapshot, value }), { headers: { 'content-type': 'application/json' } })
}
const source = {
  ...fixtureSource({ world: { now: 0, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
  resources: gatewayResources({ baseUrl: 'https://alpha.example', fetchImpl }),
}
const show = async (ref: string) => {
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><ConversationHeaderActions key={ref} agentRef={ref} /></DataSourceProvider>))
}
const showPanel = async (ref: string, view: 'details' | 'resources' = 'resources') => {
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><AgentDetailsPanel key={ref} agentRef={ref} width="dock" scheme="dark" view={view} onClose={() => undefined} onOpen={() => undefined} /></DataSourceProvider>))
  await settle()
}
const settle = async () => { await act(async () => { for (let i = 0; i < 10; i++) await new Promise<void>(resolve => setImmediate(resolve)) }) }
const click = async (name: string) => {
  const button = [...document.querySelectorAll('button')].find(node => (node.getAttribute('aria-label') ?? node.textContent) === name)
  expect(button).toBeDefined()
  await act(async () => button!.click())
  await settle()
}
beforeEach(() => {
  requests.length = 0
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  registry = AtomRegistry.make()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals(); vi.useRealTimers() })
it('adds no serial HTTP requests to a seat switch while the panel is closed', async () => {
  await show('agent/example-details')
  await settle()
  await show('agent/other')
  await settle()
  expect(requests).toEqual([])
  await showPanel('agent/other', 'details')
  expect(requests.slice(0, 2)).toEqual(['/v1/client/capabilities', '/v1/client/resources'])
})
it('keeps the authoritative poll alive for 60 seconds after closing, then releases it', async () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  await showPanel('agent/example-details')
  await show('agent/example-details')
  requests.length = 0
  await act(async () => vi.advanceTimersByTimeAsync(30_000))
  await settle()
  expect(requests.filter(path => path === '/v1/client/resources').length).toBeGreaterThan(0)
  await act(async () => vi.advanceTimersByTimeAsync(31_000))
  await settle()
  requests.length = 0
  await act(async () => vi.advanceTimersByTimeAsync(30_000))
  await settle()
  expect(requests).toEqual([])
})
it('reopens within the TTL with the retained Observed value and no second read', async () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  let reads = 0
  const resource = Atom.make(() => {
    reads++
    return observed({ value: { items: [], nextCursor: null } })
  })
  const retainedSource = { ...source, resources: { ...source.resources, byAgent: () => resource } }
  const atom = retainedResourceAtoms(retainedSource).resources('agent/example-negative-control')
  let release = registry.mount(atom)
  const first = registry.get(atom)
  expect(first._tag).toBe('Observed')
  release()
  await vi.advanceTimersByTimeAsync(1000)
  release = registry.mount(atom)
  expect(registry.get(atom)).toBe(first)
  expect(reads).toBe(1)
  release()
})
it('shows the agent Todos as the first Resources section, naming a missing snapshot', async () => {
  await showPanel('agent/example-details')
  const sections = [...document.querySelectorAll('[data-agent-resources] section[aria-label]')]
  expect(sections.slice(0, 3).map(section => section.getAttribute('aria-label'))).toEqual(['To-dos', 'Agent queue', 'Observed resources'])
  expect(sections[0]!.textContent).toBe('To-dosThis session has not shared a to-do list.')
})
it('keeps the To-dos plan with phase and progress without developer identity fields', async () => {
  const state = projectTodos({ feed: observed({ value: [todoAgent] }), agentRef: todoAgentRef })
  expect(state._tag).toBe('Observed')
  await act(async () => root.render(<AgentTodos state={state} placement="resources" />))
  if (state._tag !== 'Observed') return
  expect(host.textContent).toContain(`${state.completed} of ${state.total} done`)
  expect(host.querySelector('section[aria-label="Now"]')).not.toBeNull()
  expect(host.textContent).not.toMatch(/Source:|fact\.todo|Claim |incarnation|current binding|Tasks shared by this session/)
  expect(host.querySelector('time[data-human-time="relative"]')?.textContent).toMatch(/^Updated /)
})

it('shows Checking rather than an out-of-date warning while the roster request is pending', async () => {
  const state = projectTodos({
    feed: observed({ value: [todoAgent], freshness: 'stale' }),
    agentRef: todoAgentRef,
    sync: { _tag: 'Requested', since: 0 },
  })
  await act(async () => root.render(<AgentTodos state={state} placement="resources" />))
  expect(host.textContent).toContain('Checking to-dos…')
  expect(host.textContent).not.toContain('may be out of date')
  expect(host.textContent).not.toContain('binding verification pending')
})

it.each([
  [{ _tag: 'Waiting' } as const, 'Loading to-dos…'],
  [{ _tag: 'Unavailable', detail: 'Source: fact.todo · incarnation unavailable' } as const, "To-dos aren't available right now."],
  [{ _tag: 'Missing' } as const, 'This session has not shared a to-do list.'],
] as const)('uses plain To-dos availability copy (%j)', async (state, copy) => {
  await act(async () => root.render(<AgentTodos state={state} placement="resources" />))
  expect(host.textContent).toBe(copy)
  expect(host.textContent).not.toMatch(/fact\.todo|incarnation|harness/)
})

it('names out-of-date and partial to-do lists without exposing transport or identity fields', async () => {
  const state = projectTodos({ feed: observed({ value: [todoAgent], freshness: 'stale' }), agentRef: todoAgentRef })
  expect(state._tag).toBe('Observed')
  if (state._tag !== 'Observed') return
  const partial = { ...state, total: state.total + 2, todo: { ...state.todo, snapshot: {
    ...state.todo.snapshot, truncated: true, totals: { ...state.todo.snapshot.totals, abandoned: 2 },
  } } }
  await act(async () => root.render(<AgentTodos state={partial} placement="resources" />))
  expect(host.textContent).toContain('These to-dos may be out of date.')
  expect(host.textContent).toContain('Only some to-dos are shown. Progress counts the full list.')
  expect(host.textContent).toContain('2 abandoned, not counted')
  expect(host.textContent).not.toMatch(/Source:|fact\.todo|Claim |incarnation|stale snapshot|full-source/)
})

describe('the Resources tab', () => {
  const row = (id: string, kind: string, facts: Record<string, unknown>) =>
    decodeUnknownSync(ResourceObservation)({ id, kind, facts, observed_at: '2026-10-04T12:00:00.000Z', opened_by: 'agent/example', opened_by_run: null })
  const items = [
    row('resource/github/acme/garden/pull/5707', 'vcs.pull-request', { title: 'Wait for exec', state: 'in_progress' }),
    row('resource/custom/board/3', 'custom.acme.board', {}),
  ]
  const withItems = { ...source, resources: { ...source.resources, byAgent: () => Atom.make(observed({ value: { items, nextCursor: null } })) } }
  const onOpen = vi.fn()
  const render = (view: 'details' | 'resources') => act(async () => root.render(<DataSourceProvider source={withItems} registry={registry}><AgentDetailsPanel agentRef="agent/example" width="dock" scheme="dark" view={view} onClose={() => undefined} onOpen={onOpen} /></DataSourceProvider>))
  const rows = () => [...document.querySelectorAll<HTMLButtonElement>('button[data-resource-row]')]
  beforeEach(() => onOpen.mockClear())

  it('counts resources on the tab while another tab is showing', async () => {
    await render('details')
    await settle()
    expect(rows()).toEqual([])
    expect(document.querySelector('[role="tab"][aria-label="Resources, 2"]')?.querySelector('[data-sidebar-tab-count="2"]')).not.toBeNull()
  })
  it('opens the whole row by its ref, naming kinds in words and never as raw types', async () => {
    await render('resources')
    await settle()
    expect(rows().map(each => each.querySelector('svg')?.getAttribute('aria-label'))).toEqual(['Pull request', 'Resource'])
    expect(rows()[0]!.textContent).toContain('Wait for exec')
    expect(rows()[0]!.textContent).toContain('in progress')
    expect(rows()[1]!.textContent).toContain('Resource 3')
    expect(document.body.textContent).not.toMatch(/vcs\.pull-request|custom\.acme/)
    await act(async () => rows()[0]!.click())
    expect(onOpen).toHaveBeenCalledWith('resource/github/acme/garden/pull/5707', false)
    await act(async () => rows()[1]!.dispatchEvent(new MouseEvent('click', { bubbles: true, metaKey: true })))
    expect(onOpen).toHaveBeenLastCalledWith('resource/custom/board/3', true)
  })
  it('remembers the last tab across panels when the host does not own it', async () => {
    const uncontrolled = (ref: string) => act(async () => root.render(<DataSourceProvider source={withItems} registry={registry}><AgentDetailsPanel key={ref} agentRef={ref} width="dock" scheme="dark" onClose={() => undefined} onOpen={onOpen} /></DataSourceProvider>))
    registry.set(inspectorViewPreference, 'details')
    await uncontrolled('agent/one')
    await settle()
    const tab = document.querySelector<HTMLElement>('[role="tab"][aria-label^="Resources"]')!
    await act(async () => tab.click())
    expect(registry.get(inspectorViewPreference)).toBe('resources')
    await uncontrolled('agent/two')
    await settle()
    expect(document.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe('Resources')
    registry.set(inspectorViewPreference, 'details')
  })
  it('shows the canonical resource card on a resting pointer, and closes it after the pointer leaves', async () => {
    await render('resources')
    await settle()
    const card = () => document.querySelector<HTMLElement>('[data-hover-card] [data-entity-card="resource"]')
    await act(async () => { rows()[0]!.dispatchEvent(new MouseEvent('pointerover', { bubbles: true })) })
    expect(card()).toBeNull()
    await vi.waitFor(() => expect(card()).not.toBeNull(), { timeout: 1000 })
    expect(card()!.getAttribute('data-entity-id')).toBe('resource/github/acme/garden/pull/5707')
    expect(card()!.textContent).toContain('Wait for exec')
    expect(card()!.textContent).toContain('Pull request')
    expect(card()!.textContent).toContain('in progress')
    expect(card()!.textContent).not.toContain('resource/github')
    await act(async () => { rows()[0]!.dispatchEvent(new MouseEvent('pointerout', { bubbles: true, relatedTarget: document.body })) })
    await vi.waitFor(() => expect(card()).toBeNull(), { timeout: 1000 })
  })
})
