// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { performance as timing } from 'node:perf_hooks'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { setTimeout as delay } from 'node:timers/promises'
import { Agent, decodeUnknownSync } from '@smalltalk/st3-client/schema'
import { expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
// This is a warm selection/paint test; lazy-loading behavior has its own integration suite.
import './ConversationPane.tsx'
import { cancelMeasure, workflows } from './perfMarks.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

it('sidebar selection through the real retained transcript produces exactly one switch-to-tail measure', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('performance', timing)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
  const frames = new Map<number, FrameRequestCallback>()
  let id = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++id, callback); return id })
  vi.stubGlobal('cancelAnimationFrame', (frame: number) => frames.delete(frame))
  window.localStorage.clear()
  window.history.replaceState(null, '', '/w/agent/first')
  timing.clearMeasures('fractal:switch_to_tail')
  const agent = decodeUnknownSync(Agent)({ kind: 'agent', id: 'agent/first', name: 'First Agent', host_id: 'host/example', runtime_ids: [], reachability: 'reachable', state: 'running', harness_state: 'idle', blocked_on: null, fault: null, revision: '1', updated_at: '2026-10-04T12:00:00.000Z' })
  const source = fixtureSource({ world: {
    now: 5000, gateway: 'gateway.example.invalid', agents: [agent, { ...agent, id: 'agent/second', name: 'Second Agent' }], missions: [], attention: [], events: [],
    conversations: {
      'agent/first': { items: [], hasOlder: false, observation: { empty: true } },
      'agent/second': { items: [], hasOlder: false, observation: { empty: true } },
    }, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' },
  } })
  const registry = AtomRegistry.make()
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const flushPaint = async () => {
    await act(async () => {
      const pending = [...frames.values()]
      frames.clear()
      pending.forEach(callback => callback(0))
      await delay(10)
    })
  }
  try {
    await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider>))
    await vi.waitFor(() => expect(host.querySelector('[data-testid="transcript-empty"]')).not.toBeNull())
    for (let frame = 0; frame < 3; frame++) await flushPaint()
    expect(timing.getEntriesByName('fractal:switch_to_tail', 'measure')).toHaveLength(0)
    const second = [...host.querySelectorAll<HTMLElement>('nav [data-testid="taste-agent-row"]')].find(row => row.textContent?.includes('Second Agent'))!
    const title = second.querySelector<HTMLElement>('[data-row-column="title-text"]')!
    await act(async () => title.click())
    expect(window.location.pathname).toBe('/w/agent/second')
    for (let frame = 0; frame < 5; frame++) await flushPaint()
    expect(timing.getEntriesByName('fractal:switch_to_tail', 'measure')).toHaveLength(1)
    // Extra commits/frames after the selected content settles must not emit a second sample.
    await flushPaint()
    expect(timing.getEntriesByName('fractal:switch_to_tail', 'measure')).toHaveLength(1)
  } finally {
    await act(async () => root.unmount())
    registry.dispose()
    host.remove()
    workflows.forEach(cancelMeasure)
    timing.clearMeasures('fractal:switch_to_tail')
    vi.unstubAllGlobals()
    window.history.replaceState(null, '', '/')
  }
}, 20_000)
