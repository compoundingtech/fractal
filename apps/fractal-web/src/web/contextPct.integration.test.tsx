// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Agent, decodeUnknownSync, type AgentEncoded } from '@fractal/client/schema'
import { openAgent, type DecodedAgent } from '@fractal/sdk/effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentRowDetails } from '@fractal/ui/assistant-ui/shell'
import { DataSourceProvider, useFleet, useNow } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, type Feed } from '../data/source.ts'
import { sidebarRow } from './sidebarRow.ts'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))
// jsdom has no Range layout; the kit row measures retained text with it.
Object.defineProperty(Range.prototype, 'getBoundingClientRect', { configurable: true, value: () => new DOMRect() })
Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [] })
const makeRow = (change: Partial<AgentEncoded> = {}) => openAgent(decodeUnknownSync(Agent)({
  kind: 'agent', id: 'agent/example', name: 'Example', host_id: 'host/example', runtime_ids: [],
  reachability: 'reachable', state: 'running', revision: '1', incarnation_id: 'first', updated_at: '1970-01-01T00:00:04.000Z',
  usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214, context: { used_tokens: 61_206, window_tokens: 1_000_000, used_percent: 6.1206, observed_at_unix_ms: 3000 } },
  ...change,
}))
function ContextRows() {
  const fleet = useFleet(), now = useNow()
  if (fleet._tag !== 'Observed') return null
  return <div>{fleet.value.agents.map(agent => <AgentRowDetails key={agent.ref} row={sidebarRow({ agent, stale: fleet.freshness === 'stale', now })} now={now} />)}</div>
}
let registry: AtomRegistry.AtomRegistry
let agents: Atom.Writable<Feed<readonly DecodedAgent[]>>
let host: HTMLDivElement
let root: Root
beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', () => undefined)
  registry = AtomRegistry.make()
  agents = Atom.make(observed<readonly DecodedAgent[]>({ value: [makeRow()] }))
  const source = { ...fixtureSource({ world: { now: 10_000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), agents }
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><ContextRows /></DataSourceProvider>))
})
afterEach(async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() })

describe('native roster to reported-details popover context', () => {
  it('discloses full accessible numerator/window/as-of, not cumulative accounting', () => {
    const signal = host.querySelector('[data-context-observation]')!
    expect(signal.textContent).toBe('Reported main-session context: 6%; 61,206 used tokens; 1,000,000 token window; as of 1970-01-01T00:00:03.000Z')
    expect(signal.textContent).not.toContain('480,035,214')
    expect(host.querySelector('[data-row-field="context-percent"], [data-context-ring]')).toBeNull()
    signal.textContent = 'Context 4800%'
    expect(() => expect(signal.textContent).toContain('context: 6%')).toThrow()
  })
  it('clears across restart until a newer occupancy arrives and identifies retained stale evidence', async () => {
    await act(async () => registry.set(agents, observed({ value: [makeRow({ incarnation_id: 'second' })] })))
    expect(host.querySelector('[data-context-observation]')).toBeNull()
    const newer = makeRow({ incarnation_id: 'second', usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214, context: { used_tokens: 0, window_tokens: 1_000_000, used_percent: 0, observed_at_unix_ms: 5000 } } })
    await act(async () => registry.set(agents, observed({ value: [newer] })))
    expect(host.querySelector('[data-context-observation]')!.textContent).toContain('context: 0%')
    await act(async () => registry.set(agents, observed({ value: [newer], freshness: 'stale' })))
    expect(host.querySelector('[data-context-observation]')!.textContent).toContain('Last observed main-session context: 0%')
  })
  it('omits missing occupancy even with known large cumulative tokens', async () => {
    await act(async () => registry.set(agents, observed({ value: [makeRow({ usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 100, output_tokens: 200, cached_tokens: 300, total_tokens: 480_035_214 } })] })))
    expect(host.querySelector('[data-context-observation]')).toBeNull()
    expect(host.textContent).not.toContain('Context')
  })
})
