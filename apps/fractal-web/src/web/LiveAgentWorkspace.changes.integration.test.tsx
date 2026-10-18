// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import type { ConversationPage, DataSource } from '../data/source.ts'
import type * as CapturedChangesModule from './capturedChanges.ts'
import type * as InspectorModule from './ChangesInspector.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const { inspectorRequested, projectedChanges } = vi.hoisted(() => ({
  inspectorRequested: vi.fn(),
  projectedChanges: vi.fn(),
}))
// Observe the lazy boundary without replacing the inspector, transcript, or data hooks.
vi.mock('./ChangesInspector.tsx', async importOriginal => {
  inspectorRequested()
  return importOriginal<typeof InspectorModule>()
})
vi.mock('./capturedChanges.ts', async importOriginal => {
  const original = await importOriginal<typeof CapturedChangesModule>()
  return { ...original, capturedChanges: (page: ConversationPage) => {
    projectedChanges(page)
    return original.capturedChanges(page)
  } }
})

const agents = [
  ['agent/changes-a', 'Changes A', 'build-1', '/work/thread-changes'],
  ['agent/changes-peer', 'Changes peer', 'build-1', '/work/thread-changes'],
  ['agent/changes-b', 'Changes B', 'build-2', '/work/thread-changes'],
].map(([id, name, host, workspace]) => decodeUnknownSync(Agent)({
  kind: 'agent', id, name, host_id: `host/${host}`, workspace,
  runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle',
  blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-19T12:00:00.000Z',
}))
const page: ConversationPage = {
  hasOlder: false,
  items: [{
    _tag: 'ToolCall', id: 'captured/edit', callId: 'call/edit', name: 'Edit',
    input: { file_path: 'src/example.ts', old_string: 'old', new_string: 'new' },
    status: 'success', callSeen: true, at: '2026-10-19T12:00:00.000Z',
    result: { content: 'Edit applied', isError: false, at: '2026-10-19T12:00:01.000Z' },
  }],
}

let root: Root
let host: HTMLDivElement
let registry: AtomRegistry.AtomRegistry
let source: DataSource
let now: Atom.Writable<number>

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  // Keep speculative frame work out of assertions; selected threads still load normally.
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1600 })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/changes-a')
  projectedChanges.mockClear()
  registry = AtomRegistry.make()
  now = Atom.make(Date.parse('2026-10-19T12:00:02.000Z'))
  source = {
    ...fixtureSource({ world: {
      now: Date.parse('2026-10-19T12:00:02.000Z'), gateway: 'build-1', agents,
      missions: [], attention: [], events: [],
      conversations: Object.fromEntries(agents.map(agent => [agent.id, page])),
      terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
    } }),
    now,
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

// The same real source/provider/workspace composition can be mounted in a browser fixture
// for screenshots; only the CSS runtime and import/projection observers above are test-specific.
const mount = async () => {
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
  await act(async () => vi.dynamicImportSettled())
}
const toggle = () => host.querySelector<HTMLButtonElement>('button[aria-label="Toggle right panel"]')!
const separator = () => host.querySelector<HTMLElement>('[role="separator"][aria-label="Resize changes"]')!
const close = () => host.querySelector<HTMLButtonElement>('button[aria-label="Close changes"]')!
const openChanges = async () => {
  if (toggle().getAttribute('aria-pressed') !== 'true') await act(async () => toggle().click())
  await act(async () => vi.dynamicImportSettled())
}
const key = async (value: string) => {
  await act(async () => separator().dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true })))
}
const selectAgent = async (ref: string) => {
  const row = host.querySelector<HTMLElement>(`nav[aria-label="Agent roster"] [data-wf-agent-ref="${ref}"]`)!
  expect(row).not.toBeNull()
  await act(async () => row.click())
  await act(async () => vi.dynamicImportSettled())
}
const panelWidth = () => Number(separator().getAttribute('aria-valuenow'))

describe('live captured Changes dock', () => {
  it('loads inspector code and derives captured diffs only on open, then closes without losing the thread', async () => {
    expect(inspectorRequested).not.toHaveBeenCalled()
    await mount()
    expect(inspectorRequested).not.toHaveBeenCalled()
    expect(projectedChanges).not.toHaveBeenCalled()
    expect(separator()).toBeNull()
    expect(close()).toBeNull()
    expect(toggle().getAttribute('aria-pressed')).toBe('false')
    const thread = host.querySelector('[data-testid="conversation-history-host"]')
    expect(thread).not.toBeNull()

    await openChanges()
    expect(inspectorRequested).toHaveBeenCalledTimes(1)
    expect(projectedChanges).toHaveBeenCalledWith(page)
    expect(toggle().getAttribute('aria-pressed')).toBe('true')
    expect(separator().getAttribute('aria-valuemin')).toBe('320')
    expect(separator().getAttribute('aria-valuemax')).toBe('720')
    expect(host.querySelector('[data-testid="diff-code"]')).not.toBeNull()
    expect(host.textContent).toContain('src/example.ts')
    expect(host.textContent).toContain('Successful-tool captures')
    expect(close()).not.toBeNull()

    await act(async () => close().click())
    expect(separator()).toBeNull()
    expect(close()).toBeNull()
    expect(host.querySelector('[data-testid="diff-code"]')).toBeNull()
    expect(toggle().getAttribute('aria-pressed')).toBe('false')
    expect(host.querySelector('[data-testid="conversation-history-host"]')).toBe(thread)
    const projectionCount = projectedChanges.mock.calls.length
    await act(async () => registry.set(now, Date.parse('2026-10-19T12:00:03.000Z')))
    expect(projectedChanges).toHaveBeenCalledTimes(projectionCount)

    await openChanges()
    expect(inspectorRequested).toHaveBeenCalledTimes(1)
    expect(host.querySelector('[data-testid="conversation-history-host"]')).toBe(thread)
    await act(async () => toggle().click())
    expect(separator()).toBeNull()
  })

  it('clamps keyboard resizing and persists the committed width across close and remount', async () => {
    await mount()
    await openChanges()
    await key('Home')
    expect(panelWidth()).toBe(320)
    await key('ArrowRight')
    expect(panelWidth()).toBe(320)
    await key('End')
    expect(panelWidth()).toBe(720)
    await key('ArrowLeft')
    expect(panelWidth()).toBe(720)
    await key('Home')
    await key('ArrowLeft')
    const width = panelWidth()
    expect(width).toBeGreaterThan(320)
    expect(width).toBeLessThan(720)
    await act(async () => close().click())
    await openChanges()
    expect(panelWidth()).toBe(width)

    await act(async () => root.unmount())
    registry.dispose()
    registry = AtomRegistry.make()
    root = createRoot(host)
    await mount()
    await openChanges()
    expect(panelWidth()).toBe(width)
  })

  it('shares width within a workspace while isolating the same workspace path on another host', async () => {
    await mount()
    await openChanges()
    const initial = panelWidth()
    await key('End')
    expect(panelWidth()).toBe(720)

    await selectAgent('agent/changes-peer')
    await openChanges()
    expect(panelWidth()).toBe(720)

    await selectAgent('agent/changes-b')
    await openChanges()
    expect(panelWidth()).toBe(initial)
    await key('Home')
    expect(panelWidth()).toBe(320)

    await selectAgent('agent/changes-a')
    await openChanges()
    expect(panelWidth()).toBe(720)
    await selectAgent('agent/changes-b')
    await openChanges()
    expect(panelWidth()).toBe(320)
  })
})
