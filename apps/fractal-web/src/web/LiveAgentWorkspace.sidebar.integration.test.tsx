// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, decodeUnknownSync, type Attention, type Mission, type TerminalScreen } from '@smalltalk/st3-client/schema'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as DateTime from 'effect/DateTime'
import * as Option from 'effect/Option'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { initialFeedSync, type FeedSyncObservation } from '../data/feedSync.ts'
import { observed, waiting, type ConversationPage, type DataSource, type Feed } from '../data/source.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import * as dataHooks from '../data/react.tsx'
import * as rowProjection from './sidebarRow.ts'
import * as treeProjection from './liveSidebarTree.ts'
import * as folderComponents from '../shell/AgentFolders.tsx'
import type { ConnectionState } from '@st3/sdk/effect'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

const gateway = 'gateway.example.invalid'
const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-04T12:00:00.000Z' })
let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
let agents: Atom.Writable<Feed<readonly Agent[]>>
let gatewaySync: Atom.Writable<FeedSyncObservation>
let source: DataSource

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  // Keep this regression on the shell: no frame requests speculative transcript code.
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  window.localStorage.clear()
  window.history.replaceState(null, '', '/?open=resource/example:detail')
  registry = AtomRegistry.make()
  // Live gateway feeds are keepAlive and outlive the roster's subscription; an autoDispose fixture would reset on unmount.
  agents = Atom.make<Feed<readonly Agent[]>>(waiting).pipe(Atom.keepAlive)
  gatewaySync = Atom.make<FeedSyncObservation>({ status: { _tag: 'Live', since: 0 }, observedAt: 0 })
  source = {
    ...fixtureSource({ world: { now: 5000, gateway, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }),
    agents,
    sync: {
      gateway: gatewaySync,
      agents: Atom.make(initialFeedSync<readonly Agent[]>(0)),
      missions: Atom.make(initialFeedSync<readonly Mission[]>(0)),
      attention: Atom.make(initialFeedSync<readonly Attention[]>(0)),
      conversation: () => Atom.make(initialFeedSync<ConversationPage>(0)),
      terminal: () => Atom.make(initialFeedSync<TerminalScreen>(0)),
    },
  }
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  registry.dispose()
  host.remove()
  vi.unstubAllGlobals()
  window.history.replaceState(null, '', '/')
})

const mount = async (width: number) => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
}
const sidebar = () => host.querySelector('aside[aria-label="Agents"]')!
const roster = () => sidebar().querySelector('nav[aria-label="Agent roster"]')!
const band = () => host.querySelector('[data-connection-state]')

it('keeps workspace providers and unchanged roster rows out of the one-second clock render', async () => {
  const now = Atom.make(5000)
  source = { ...source, now }
  registry.set(agents, observed({ value: [agent, { ...agent, id: 'agent/other', name: 'Other Agent' }] }))
  const workspaceRenders = vi.spyOn(dataHooks, 'useFleet')
  const projectedRows = vi.spyOn(rowProjection, 'sidebarRow')
  const projectedTree = vi.spyOn(treeProjection, 'liveSidebarTree')
  try {
    await mount(1440)
    const workspaceCount = workspaceRenders.mock.calls.length
    const rowCount = projectedRows.mock.calls.length
    const treeCount = projectedTree.mock.calls.length
    for (let second = 1; second <= 10; second += 1) {
      await act(async () => registry.set(now, 5000 + second * 1000))
    }
    expect(workspaceRenders).toHaveBeenCalledTimes(workspaceCount)
    expect(projectedRows).toHaveBeenCalledTimes(rowCount)
    expect(projectedTree).toHaveBeenCalledTimes(treeCount)
  } finally {
    workspaceRenders.mockRestore()
    projectedRows.mockRestore()
    projectedTree.mockRestore()
  }
})

it('keeps tree render callbacks and unchanged rows stable when one agent receives a data frame', async () => {
  const first = { ...agent, last_activity_at: Option.some(DateTime.makeUnsafe(1000)) }
  const other: Agent = { ...agent, id: 'agent/other', name: 'Other Agent' }
  registry.set(agents, observed({ value: [first, other] }))
  const rows = vi.spyOn(dataHooks, 'useAgent')
  const folders = vi.spyOn(folderComponents, 'AgentFolders')
  const calls = (ref: string) => rows.mock.calls.filter(([value]) => value === ref).length
  try {
    await mount(1440)
    const firstCount = calls(first.id)
    const otherCount = calls(other.id)
    const before = folders.mock.calls.at(-1)![0]
    if (!('tree' in before)) throw new Error('Expected the bound live sidebar')
    await act(async () => registry.set(agents, observed({ value: [{ ...first, last_activity_at: Option.some(DateTime.makeUnsafe(2000)) }, other] })))
    const after = folders.mock.calls.at(-1)![0]
    if (!('tree' in after)) throw new Error('Expected the bound live sidebar')
    expect(calls(first.id)).toBeGreaterThan(firstCount)
    expect(calls(other.id)).toBe(otherCount)
    expect(after.renderAgent).toBe(before.renderAgent)
    expect(after.renderFolderSignal).toBe(before.renderFolderSignal)
    expect(after.accessibleNames).toBe(before.accessibleNames)
    expect(after.tree).toBe(before.tree)
    expect(roster().querySelector('[data-wf-agent-ref="agent/example"] [data-row-column="time"] time')?.textContent).toBe('3s')
  } finally {
    rows.mockRestore()
    folders.mockRestore()
  }
})

it('marks retained live-feed rows stale on a connection-only transition', async () => {
  const connection = Atom.make<ConnectionState>({ _tag: 'Live' })
  source = { ...source, connection }
  const retained = observed({ value: [agent] })
  registry.set(agents, retained)
  await mount(1440)
  const row = () => roster().querySelector('[role="row"][data-wf-agent-ref="agent/example"]') ?? roster().querySelector('[data-wf-agent-ref="agent/example"]')!.closest('[role="row"]')!
  const freshness = () => row().querySelector('[data-observation-freshness]')!.getAttribute('data-observation-freshness')
  const status = () => row().querySelector('[data-row-field="status"] svg')!.getAttribute('aria-label')!
  expect(freshness()).toBe('live')
  expect(row().getAttribute('aria-label')).toContain('Example Agent, idle')
  expect(status()).toContain('idle;')
  await act(async () => registry.set(connection, { _tag: 'Connecting', attempt: 1 }))
  expect(registry.get(agents)).toBe(retained)
  expect(freshness()).toBe('stale')
  expect(row().getAttribute('aria-label')).toContain('Example Agent, Last verified · idle')
  expect(status()).toContain('Last verified · idle;')
  expect(status()).toContain('stale observation')
  await act(async () => registry.set(connection, { _tag: 'Live' }))
  expect(freshness()).toBe('live')
  expect(row().getAttribute('aria-label')).toContain('Example Agent, idle')
  expect(status()).toContain('live observation')
})

it('refreshes sidebar activity labels at their exact second and minute boundaries', async () => {
  const now = Atom.make(5000)
  source = { ...source, now }
  const timedAgent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '1970-01-01T00:00:01.000Z', last_activity_at: '1970-01-01T00:00:01.000Z' })
  registry.set(agents, observed({ value: [timedAgent] }))
  await mount(1440)
  const time = () => roster().querySelector('[data-row-column="time"] time')!.textContent
  expect(time()).toBe('4s')
  await act(async () => registry.set(now, 6000))
  expect(time()).toBe('5s')
  await act(async () => registry.set(now, 60_000))
  expect(time()).toBe('59s')
  await act(async () => registry.set(now, 61_000))
  expect(time()).toBe('1m')
  await act(async () => registry.set(now, 120_000))
  expect(time()).toBe('1m')
  await act(async () => registry.set(now, 121_000))
  expect(time()).toBe('2m')
})

it('reveals a slightly future activity timestamp when only the leaf clock catches up', async () => {
  const now = Atom.make(5000)
  source = { ...source, now }
  const timedAgent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '1970-01-01T00:00:05.500Z', last_activity_at: '1970-01-01T00:00:05.500Z' })
  registry.set(agents, observed({ value: [timedAgent] }))
  const projectedRows = vi.spyOn(rowProjection, 'sidebarRow')
  try {
    await mount(1440)
    const count = projectedRows.mock.calls.length
    expect(roster().querySelector('[data-row-column="time"] time')).toBeNull()
    await act(async () => registry.set(now, 6000))
    expect(roster().querySelector('[data-row-column="time"] time')?.textContent).toBe('0s')
    expect(projectedRows).toHaveBeenCalledTimes(count)
  } finally {
    projectedRows.mockRestore()
  }
})

it('reveals a future status boundary in ClockedThreadHeader without rendering the header projection', async () => {
  const now = Atom.make(5000)
  source = { ...source, now }
  const at = '1970-01-01T00:00:05.500Z'
  const suspended = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/example', name: 'Example Agent', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'suspended', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: at, suspension: { action: 'suspend', blocking: [], operation_id: 'operation/example', phase: 'suspended', suspended_at: at, updated_at: at } })
  registry.set(agents, observed({ value: [suspended] }))
  window.history.replaceState(null, '', '/w/agent/example')
  const projectedRows = vi.spyOn(rowProjection, 'sidebarRow')
  try {
    await mount(1440)
    const header = host.querySelector('section[aria-label="Agent workspace"] header')!
    expect(header).not.toBeNull()
    const count = projectedRows.mock.calls.length
    expect(header.querySelector('time')).toBeNull()
    await act(async () => registry.set(now, 6000))
    expect(header.querySelector('time')?.textContent).toBe('0s')
    expect(projectedRows).toHaveBeenCalledTimes(count)
  } finally {
    projectedRows.mockRestore()
  }
})

// Both the real data derivation and the kit's SidebarAgentRow DOM participate.
describe.each([1280, 1440, 2980])('sidebar footer at %ipx', width => {
  it('dismisses the hovered row on navigation while preserving every roster node', async () => {
    const other: Agent = { ...agent, id: 'agent/other', name: 'Other Agent' }
    registry.set(agents, observed({ value: [agent, other] }))
    await mount(width)
    const before = [...roster().querySelectorAll('[data-testid="taste-agent-row"]')]
    const hovered = before[0]!.closest<HTMLElement>('[data-wf-agent-ref]')!
    const escape = vi.fn()
    hovered.addEventListener('keydown', escape)
    await act(async () => hovered.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })))
    await act(async () => {
      window.history.pushState(null, '', '/w/agent/other?open=thread')
      window.dispatchEvent(new PopStateEvent('popstate'))
    })
    expect(escape).toHaveBeenCalledOnce()
    expect(escape.mock.calls[0]![0].key).toBe('Escape')
    const after = [...roster().querySelectorAll('[data-testid="taste-agent-row"]')]
    expect(after).toHaveLength(before.length)
    before.forEach((node, index) => expect(after[index]).toBe(node))
    expect(before.every(node => node.isConnected)).toBe(true)
    expect(roster().textContent).toContain(agent.name)
    expect(roster().textContent).toContain(other.name)
  })
  it('keeps the connection out of the sidebar and shows a band only once the connection is lost', async () => {
    registry.set(agents, observed({ value: [agent] }))
    await mount(width)
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(1)
    expect(roster().textContent).toContain(agent.name)
    expect(band()).toBeNull()
    expect(sidebar().querySelector('footer [role="status"]')).toBeNull()
    await act(async () => registry.set(gatewaySync, { status: { _tag: 'Connecting', attempt: 1, since: 0 }, observedAt: 0 }))
    expect(band()).toBeNull()
    await act(async () => registry.set(gatewaySync, { status: { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: 7000, issue: 'socket closed' } }, observedAt: 0 }))
    expect(band()?.getAttribute('data-connection-state')).toBe('reconnecting')
    expect(sidebar().contains(band())).toBe(false)
    expect(host.textContent).not.toContain(gateway)
  })

  it('removes the waiting notice and skeleton as soon as roster rows are published', async () => {
    await mount(width)
    expect(sidebar().querySelector('[data-system-state="loading"]')?.textContent).toBe('Loading agents…')
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(0)
    expect(roster().querySelector('[data-wf-roster-skeleton]')).not.toBeNull()
    await act(async () => registry.set(agents, { _tag: 'Observed', value: [agent], freshness: 'stale', coverage: { _tag: 'Partial' } }))
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(1)
    expect(roster().textContent).toContain(agent.name)
    expect(sidebar().querySelector('[data-system-state="loading"]')).toBeNull()
    for (const node of host.querySelectorAll('[role="status"], [aria-live], [aria-label], [aria-description]')) {
      expect(node.textContent).not.toContain('Loading agents')
      expect(node.getAttribute('aria-label') ?? '').not.toContain('Loading agents')
      expect(node.getAttribute('aria-description') ?? '').not.toContain('Loading agents')
    }
    expect(roster().querySelector('[data-wf-roster-skeleton]')).toBeNull()
    expect(sidebar().querySelector('[data-system-state="stale"]')?.textContent).toBe('Partial roster')
    await act(async () => registry.set(agents, observed({ value: [agent] })))
    expect(sidebar().querySelector('[data-system-state]')).toBeNull()
    await act(async () => registry.set(agents, observed({ value: [] })))
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(0)
    expect(sidebar().querySelector('[data-system-state]')).toBeNull()
  })
})

describe('shipped live sidebar search', () => {
  it('applies typed hosts to the real roster, matches free text on names and IDs only, and strips facets from highlights', async () => {
    const candidates: readonly Agent[] = [
      { ...agent, id: 'agent/compiler', name: 'Compiler Alpha', host_id: Option.some('host/synthetic-138/1') },
      { ...agent, id: 'agent/sidebar', name: 'Sidebar Beta', host_id: Option.some('host/SYNTHETIC-138/2') },
      // Text looks like a matching host but must not satisfy a machine facet.
      { ...agent, id: 'agent/decoy', name: 'Compiler synthetic-138/1 decoy', host_id: Option.some('host/laptop24') },
    ]
    registry.set(agents, observed({ value: candidates }))
    await mount(1440)
    const input = sidebar().querySelector<HTMLInputElement>('input[aria-label="Search agents"]')!
    expect(input.getAttribute('aria-description')).toBe('Free text matches agent names and IDs. Use is:asking for agents waiting for an answer, repo:, state:, or tool: to search other facts, and machine:build-1 or host:build-1,build-2 to filter host prefixes, ignoring case. Comma-separated values match any host; separate tokens must all match.')
    const search = async (value: string) => {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value)
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
    }
    const names = () => [...roster().querySelectorAll('[data-testid="taste-agent-row"]')].map(row => row.textContent)
    expect(names()).toHaveLength(3)
    for (const [query, expected] of [
      ['machine:synthetic-138/1', ['Compiler Alpha']],
      ['machine:synthetic-138/1,synthetic-138/2', ['Compiler Alpha', 'Sidebar Beta']],
      ['HOST:SYNTHETIC-138', ['Compiler Alpha', 'Sidebar Beta']],
      ['machine:synthetic-138 host:synthetic-138/1', ['Compiler Alpha']],
      ['machine:synthetic-138/1 host:synthetic-138/2', []],
      ['machine:synthetic-138/9', []],
      ['machine: host:,,', ['Compiler Alpha', 'Sidebar Beta', 'Compiler synthetic-138/1 decoy']],
      ['Alpha machine:SYNTHETIC-138', ['Compiler Alpha']],
      ['cmp machine:synthetic-138', ['Compiler Alpha']],
      // Free text reaches the decoy's displayed name, not the host.
      ['synthetic-138/1', ['Compiler synthetic-138/1 decoy']],
      ['agent/sidebar', ['Sidebar Beta']],
      ['machine:host/synthetic-138/1', []],
    ] as const) {
      await search(query)
      expect(names()).toHaveLength(expected.length)
      expected.forEach(name => expect(roster().textContent).toContain(name))
    }
    await search('machine:synthetic-138/1')
    expect(roster().querySelectorAll('mark')).toHaveLength(0)
    await search('Alpha machine:synthetic-138/1')
    expect([...roster().querySelectorAll('mark')].map(mark => mark.textContent).join('')).toContain('Alpha')
    expect([...roster().querySelectorAll('mark')].some(mark => mark.textContent?.includes('machine:'))).toBe(false)
    // Complete machine choices are remembered browser-wide; clear it so later mounts start unfiltered.
    await search('')
  })

  it('names each native tree row with the displayed freshness-qualified status', async () => {
    registry.set(agents, observed({ value: [agent] }))
    await mount(1440)
    const rowName = () => roster().querySelector('[role="row"][data-sidebar-tree-key="agent/example"]')?.getAttribute('aria-label')
    expect(rowName()).toBe('Example Agent, idle')
    await act(async () => registry.set(agents, { _tag: 'Observed', value: [agent], freshness: 'stale' }))
    expect(rowName()).toBe('Example Agent, Last verified · idle')
  })
})

describe('workspace overlays', () => {
  // Outside the themed root only the legacy document palette applies (the olive/teal menus).
  it('portals a row actions menu into the themed workspace root, not document.body', async () => {
    // React Aria's menu selection escapes keys with CSS.escape, which jsdom does not provide.
    if (globalThis.CSS?.escape === undefined) vi.stubGlobal('CSS', { escape: (value: string) => value.replace(/[^\w-]/g, (char) => `\\${char}`) })
    registry.set(agents, observed({ value: [agent] }))
    await mount(1440)
    const trigger = roster().querySelector<HTMLElement>('[role="row"][data-sidebar-tree-key="agent/example"] [data-agent-menu]')!
    expect(trigger).not.toBeNull()
    // Enter the RAC collection before focusing a descendant control: initial entry
    // deliberately restores the tree's owning row, as keyboard Tab entry does.
    await act(async () => trigger.closest<HTMLElement>('[role="row"]')!.focus())
    await act(async () => trigger.focus())
    expect(document.activeElement).toBe(trigger)
    await act(async () => {
      // RAC reserves vertical arrows for row navigation, even from a row control.
      trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }))
    })
    const menu = document.querySelector('[role="menu"]')
    expect(menu).not.toBeNull()
    expect(host.querySelector('[data-testid="live-agent-workspace"]')!.contains(menu)).toBe(true)
  })
})

// Remembered machines live in module state shared across tests; keep these after the suites that expect an unfiltered roster.
describe('shipped live sidebar Show filter', () => {
  it('filters Waiting for answer from st questions only and keeps the persisted query across remount', async () => {
    registry.set(agents, observed({ value: [
      { ...agent, id: 'agent/asker', name: 'Asker', host_id: Option.some('host/host-b'), blocked_on: Option.some('human'), ask: Option.some('question') },
      { ...agent, id: 'agent/permission', name: 'Permission', host_id: Option.some('host/host-b'), blocked_on: Option.some('human'), ask: Option.some('permission') },
      { ...agent, id: 'agent/elsewhere', name: 'Elsewhere', host_id: Option.some('host/host-a'), blocked_on: Option.some('human'), ask: Option.some('question') },
    ] }))
    await mount(1440)
    const input = () => sidebar().querySelector<HTMLInputElement>('input')!
    const names = () => [...roster().querySelectorAll('[data-testid="taste-agent-row"]')].map(row => row.textContent ?? '')
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input(), 'is:asking machine:host-b')
      input().dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(names()).toHaveLength(1)
    expect(names()[0]).toContain('Asker')
    const trigger = [...sidebar().querySelectorAll('button')].find(button => button.textContent?.startsWith('Show'))
    expect(trigger?.textContent).toContain(', 2 active')
    await act(async () => root.unmount())
    root = createRoot(host)
    await mount(1440)
    // The gateway's persisted filters return the whole query; remembered machines only seed it.
    expect(input().value).toBe('is:asking machine:host-b')
    expect(names()).toHaveLength(1)
  })

  it('keeps complete remembered machine groups when a partial prefix is typed before remount', async () => {
    registry.set(agents, observed({ value: [
      { ...agent, id: 'agent/a', host_id: Option.some('host/host-a') },
      { ...agent, id: 'agent/b', host_id: Option.some('host/host-b') },
    ] }))
    await mount(1440)
    const input = () => sidebar().querySelector<HTMLInputElement>('input')!
    for (const query of ['machine:host-a machine:host-b', 'machine:h']) {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input(), query)
        input().dispatchEvent(new Event('input', { bubbles: true }))
      })
      expect(input().value).toBe(query)
    }
    await act(async () => root.unmount())
    root = createRoot(host)
    await mount(1440)
    expect(input().value).toBe('machine:host-a machine:host-b')
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(0)
  })

  it('updates remembered machines after a token with duplicate values changes', async () => {
    registry.set(agents, observed({ value: [
      { ...agent, id: 'agent/a', host_id: Option.some('host/host-a') },
      { ...agent, id: 'agent/b', host_id: Option.some('host/host-b') },
    ] }))
    await mount(1440)
    const input = () => sidebar().querySelector<HTMLInputElement>('input')!
    for (const query of ['machine:host-a,host-a', 'machine:host-a,host-b']) {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input(), query)
        input().dispatchEvent(new Event('input', { bubbles: true }))
      })
    }
    await act(async () => root.unmount())
    root = createRoot(host)
    await mount(1440)
    expect(input().value).toBe('machine:host-a,host-b')
    expect(roster().querySelectorAll('[data-testid="taste-agent-row"]')).toHaveLength(2)
  })
})
