// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { Schema } from 'effect'
import { persistedAtom } from '../state/persistence.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
const renders = vi.hoisted(() => ({ thread: 0, mounts: 0, viewportStore: undefined as unknown }))
vi.mock('./ConversationPane.tsx', () => ({ ConversationPane: () => {
  renders.thread++
  renders.viewportStore = React.useContext(ViewportStoreContext)
  const [instance] = React.useState(() => ++renders.mounts)
  const [draft, setDraft] = React.useState('')
  return <div data-testid="retained-conversation" data-instance={instance} style={{ overflow: 'auto', height: 200 }}><div style={{ height: 2000 }}>Conversation history</div><textarea aria-label="Message agent" value={draft} onChange={event => setDraft(event.target.value)} /></div>
} }))
vi.mock('./ChangesInspector.tsx', () => ({ ChangesInspector: () => <section aria-label="Captured changes">Captured changes</section> }))
let root: Root
let host: HTMLDivElement
let registry: AtomRegistry.AtomRegistry
const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/metric-track', name: 'Metric track', host_id: 'host/build-1', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-18T12:00:00.000Z' })
beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  // jsdom does not supply the CSSOM escaping used by React Aria collection keys.
  vi.stubGlobal('CSS', { escape: (value: string) => Array.from(value, char => /[a-zA-Z_-]/.test(char) ? char : `\\${char.codePointAt(0)?.toString(16)} `).join('') })
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => window.setTimeout(callback, 0))
  vi.stubGlobal('cancelAnimationFrame', window.clearTimeout)
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 390 })
  const visualViewport = new EventTarget()
  Object.defineProperty(visualViewport, 'height', { configurable: true, value: 844 })
  vi.stubGlobal('visualViewport', visualViewport)
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width') && window.innerWidth < 900, media: query, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }))
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/metric-track')
  registry = AtomRegistry.make()
  registry.set(persistedAtom({ key: 'round2.phoneAgentTabs', schema: Schema.Array(Schema.String), defaultValue: [] }), [])
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  const source = fixtureSource({ world: { now: 5000, gateway: 'gateway.example.invalid', agents: [agent, { ...agent, id: 'agent/reviewer', name: 'Reviewer' }], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } })
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
})
afterEach(async () => {
  await act(async () => root.unmount())
  registry.dispose()
  host.remove()
  vi.unstubAllGlobals()
})

it('opens, traps focus, closes with Escape and returns focus without remounting the roster or rendering the thread', async () => {
  const trigger = host.querySelector<HTMLButtonElement>('[aria-label="Expand sidebar"]')!
  const roster = host.querySelector('[aria-label="Agent roster"]')!
  const count = renders.thread
  await act(async () => { trigger.focus(); trigger.click() })
  const dialog = host.querySelector<HTMLElement>('[role="dialog"][aria-label="Agents"]')!
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  expect(dialog.contains(document.activeElement)).toBe(true)
  await act(async () => trigger.focus())
  await vi.waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
  await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })))
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  await vi.waitFor(() => expect(document.activeElement).toBe(trigger))
  expect(host.querySelector('[aria-label="Agent roster"]')).toBe(roster)
  expect(renders.thread).toBe(count)
})

it('closes the drawer when an agent is selected and supports its explicit close button', async () => {
  const trigger = host.querySelector<HTMLButtonElement>('[aria-label="Expand sidebar"]')!
  await act(async () => { trigger.focus(); trigger.click() })
  const row = host.querySelector<HTMLElement>('[role="row"][data-sidebar-tree-key="agent/reviewer"]')!
  expect(row).not.toBeNull()
  await act(async () => {
    row.focus()
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }))
    row.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }))
  })
  expect(window.location.pathname).toBe('/w/agent/reviewer')
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  await vi.waitFor(() => expect(document.activeElement).toBe(trigger))
  await act(async () => trigger.click())
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Close agents"]')!.click())
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  await vi.waitFor(() => expect(document.activeElement).toBe(trigger))
})

it('switches Thread and Changes with arrow keys and connects the selected tab to its panel', async () => {
  const tabs = [...host.querySelectorAll<HTMLElement>('[role="tab"]')]
  expect(tabs.map(tab => tab.textContent)).toEqual(['Thread', 'Changes'])
  await act(async () => { tabs[0]!.focus(); tabs[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true })) })
  expect(tabs[1]!.getAttribute('aria-selected')).toBe('true')
  expect(document.activeElement).toBe(tabs[1])
  expect(host.querySelector('[aria-label="Captured changes"]')).not.toBeNull()
  expect(document.getElementById(tabs[1]!.getAttribute('aria-controls')!)).toBe(host.querySelector('[role="tabpanel"]'))
  await act(async () => tabs[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true })))
  expect(tabs[0]!.getAttribute('aria-selected')).toBe('true')
  expect(host.querySelector('[aria-label="Captured changes"]')).toBeNull()
})

it('keeps the workspace within the visual viewport when the on-screen keyboard resizes it', async () => {
  Object.defineProperty(window.visualViewport, 'height', { configurable: true, value: 400 })
  await act(async () => window.visualViewport!.dispatchEvent(new Event('resize')))
  expect(host.querySelector<HTMLElement>('[data-testid="live-agent-workspace"]')!.style.height).toContain('400px')
})

it('retains the draft, scroll position and WorkspaceBody viewport store across the breakpoint in both directions', async () => {
  const field = host.querySelector<HTMLTextAreaElement>('[aria-label="Message agent"]')!
  const history = host.querySelector<HTMLDivElement>('[data-testid="retained-conversation"]')!
  const paneHost = history.parentElement!.parentElement!.parentElement!
  const viewportStore = renders.viewportStore
  const mounts = renders.mounts
  const instance = history.dataset.instance
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, 'Unsent rotation draft')
    field.dispatchEvent(new Event('input', { bubbles: true }))
    history.scrollTop = 672
    history.dispatchEvent(new Event('scroll', { bubbles: true }))
  })
  for (const width of [900, 899, 1200, 390]) {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
    await act(async () => window.dispatchEvent(new Event('resize')))
    expect(host.querySelector('[aria-label="Expand sidebar"]') !== null).toBe(width < 900)
    expect(host.querySelector('[aria-label="Message agent"]')).toBe(field)
    expect(field.value).toBe('Unsent rotation draft')
    expect(host.querySelector('[data-testid="retained-conversation"]')).toBe(history)
    expect(history.scrollTop).toBe(672)
    expect(history.dataset.instance).toBe(instance)
    expect(history.parentElement!.parentElement!.parentElement!).toBe(paneHost)
    expect(renders.mounts).toBe(mounts)
    // This store is allocated by WorkspaceBody itself, so its identity detects a parent remount.
    expect(renders.viewportStore).toBe(viewportStore)
  }
})
