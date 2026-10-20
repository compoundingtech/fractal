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
let splitObservers: Set<() => void>

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  splitObservers = new Set()
  vi.stubGlobal('ResizeObserver', class {
    private notify: () => void
    constructor(callback: ResizeObserverCallback) { this.notify = () => callback([], this as unknown as ResizeObserver) }
    observe(node: Element) { if (node.hasAttribute('data-docked-split')) splitObservers.add(this.notify) }
    unobserve() { splitObservers.delete(this.notify) }
    disconnect() { splitObservers.delete(this.notify) }
  })
  const originalRect = HTMLElement.prototype.getBoundingClientRect
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const split = this.hasAttribute('data-docked-split') ? this : this.parentElement?.hasAttribute('data-docked-split') ? this.parentElement : undefined
    if (split) {
      const sidebar = split.closest('[data-testid="live-agent-workspace"]')?.querySelector<HTMLElement>('aside[aria-label="Agents"]')
      const available = window.innerWidth - Number.parseFloat(sidebar?.style.width ?? '0') - 1
      const width = this === split ? available : this.hasAttribute('role') ? 1 : this.hasAttribute('data-dock') ? Number.parseFloat(split.style.getPropertyValue('--dock-size')) : available - Number.parseFloat(split.style.getPropertyValue('--dock-size')) - 1
      return new DOMRect(0, 0, width, 900)
    }
    return originalRect.call(this)
  })
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
  vi.restoreAllMocks()
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
const close = () => host.querySelector<HTMLButtonElement>('button[aria-label="Close agent inspector"]')!
const openChanges = async () => {
  if (toggle().getAttribute('aria-pressed') !== 'true') await act(async () => toggle().click())
  await act(async () => vi.dynamicImportSettled())
  const changes = host.querySelector<HTMLButtonElement>('[role="tab"][aria-label="Changes"]')
    ?? [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(tab => tab.textContent?.trim() === 'Changes')
  expect(changes).not.toBeNull()
  await act(async () => changes!.click())
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
    const titles = host.querySelectorAll('h1, h2, h3, h4, h5, h6, [role="heading"]')
    expect(titles).toHaveLength(1)
    expect(titles[0]?.textContent).toBe('Changes A')
    expect(host.querySelector('[data-testid="transcript-header"]')).toBeNull()
    expect(host.querySelector('[data-testid="thread-header"] [data-testid="transcript-header-status"]')).not.toBeNull()

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
    const disclosure = host.querySelector<HTMLButtonElement>('button[aria-label="Captured changes information"]')!
    expect(disclosure.getAttribute('aria-expanded')).toBe('false')
    expect(host.querySelector('[data-testid="diff-panel-header"]')?.textContent).toBe('Changes')
    expect(host.querySelector('[aria-label="Captured change resources"]')).toBeNull()
    await act(async () => disclosure.click())
    expect(disclosure.getAttribute('aria-expanded')).toBe('true')
    const provenance = host.querySelector('[data-testid="diff-panel-info"]')!
    expect(provenance.textContent).toContain('Call: call/edit')
    expect(provenance.textContent).toContain('not current repository contents')

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
  it('keeps a 360px thread at the 900px desktop boundary when End is pressed', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 900 })
    await mount()
    await act(async () => host.querySelector('[role="separator"][aria-label="Agent sidebar width"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })))
    await openChanges()
    await key('End')
    const split = separator().parentElement!
    expect(split.firstElementChild!.getBoundingClientRect().width).toBeGreaterThanOrEqual(360)
    expect(panelWidth()).toBe(330)
    expect(separator().getAttribute('aria-valuemax')).toBe('330')
  })

  it('bounds a restored 720px preference at 900px without replacing the preference', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 900 })
    await mount()
    await act(async () => host.querySelector('[role="separator"][aria-label="Agent sidebar width"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })))
    const split = host.querySelector<HTMLElement>('[data-docked-split]')!
    const storageKey = `composition.${split.getAttribute('data-docked-split')}`
    window.localStorage.setItem(storageKey, '720')
    await openChanges()
    expect(split.firstElementChild!.getBoundingClientRect().width).toBeGreaterThanOrEqual(360)
    expect(Number.parseFloat(host.querySelector<HTMLElement>('aside[aria-label="Agents"]')!.style.width)).toBeGreaterThanOrEqual(208)
    expect(panelWidth()).toBe(split.getBoundingClientRect().width - 361)
    expect(separator().getAttribute('aria-valuemax')).toBe(String(panelWidth()))
    expect(window.localStorage.getItem(storageKey)).toBe('720')
  })

  it('constrains a shrinking window and restores the preferred width when space returns', async () => {
    await mount()
    await openChanges()
    await key('End')
    const split = separator().parentElement!
    const storageKey = `composition.${split.getAttribute('data-docked-split')}`
    expect(panelWidth()).toBe(720)
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 900 })
    await act(async () => { window.dispatchEvent(new Event('resize')) })
    await act(async () => host.querySelector('[role="separator"][aria-label="Agent sidebar width"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true })))
    await act(async () => { for (const notify of splitObservers) notify() })
    expect(split.firstElementChild!.getBoundingClientRect().width).toBeGreaterThanOrEqual(360)
    expect(panelWidth()).toBeLessThanOrEqual(330)
    expect(separator().getAttribute('aria-valuemax')).toBe(String(panelWidth()))
    expect(window.localStorage.getItem(storageKey)).toBe('720')
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1600 })
    await act(async () => { window.dispatchEvent(new Event('resize')) })
    await act(async () => { for (const notify of splitObservers) notify() })
    expect(panelWidth()).toBe(720)
    expect(window.localStorage.getItem(storageKey)).toBe('720')
  })
})
