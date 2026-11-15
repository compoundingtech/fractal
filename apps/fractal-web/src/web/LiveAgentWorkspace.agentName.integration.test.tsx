// @vitest-environment jsdom
import { Agent, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import { fixtureSource } from '../data/fixtureSource.ts'
import { DataSourceProvider } from '../data/react.tsx'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))
vi.mock('./ConversationPane.tsx', () => ({ ConversationPane: () => <div data-testid="conversation">Conversation history</div> }))

const id = 'agent/example/atlas'
const seat = (name: string) => decodeUnknownSync(Agent)({ kind: 'agent', id, name, host_id: 'host/build-1', runtime_ids: [], reachability: 'reachable',
  state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-18T12:00:00.000Z' })

let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  vi.unstubAllGlobals()
})

const mount = async (width: number, name: string) => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  vi.stubGlobal('CSS', { escape: (value: string) => Array.from(value, char => /[a-zA-Z_-]/.test(char) ? char : `\\${char.codePointAt(0)?.toString(16)} `).join('') })
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => window.setTimeout(callback, 0))
  vi.stubGlobal('cancelAnimationFrame', window.clearTimeout)
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width') && window.innerWidth < 900, media: query, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }))
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  window.localStorage.clear()
  window.history.replaceState(null, '', `/w/${id}`)
  const registry = AtomRegistry.make()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const source = fixtureSource({ world: { now: 5000, gateway: 'gateway.example.invalid', agents: [seat(name)], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } })
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
  dispose = async () => {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
  }
  return host
}

// The daemon labels a seat that declares no name by its path; the id and that path are addresses, not names.
it.each([
  ['wide', 1400, 'Atlas', 'Atlas'],
  ['wide', 1400, 'example/atlas', 'atlas'],
  ['narrow', 390, 'example/atlas', 'atlas'],
  ['wide', 1400, id, 'atlas'],
] as const)('names the agent in the %s header, sidebar row and title at %ipx by its name, never its id (roster label %s shows %s)', async (_, width, label, shown) => {
  const host = await mount(width, label)
  await vi.waitFor(() => expect(host.querySelector('[data-testid="conversation"]')).not.toBeNull())
  const text = host.textContent ?? ''
  expect(text).not.toContain('example/atlas')
  expect(text).toContain(shown)
  expect(document.title).toContain(shown)
  expect(document.title).not.toContain('example/atlas')
  if (width > 900) {
    // The docked sidebar's row; the narrow drawer is closed, so its rows are not mounted.
    await vi.waitFor(() => expect(host.querySelector(`[data-sidebar-tree-key="${id}"]`)?.textContent).toContain(shown))
    expect(host.querySelector(`[data-sidebar-tree-key="${id}"]`)?.textContent).not.toContain('example/atlas')
  }
})
