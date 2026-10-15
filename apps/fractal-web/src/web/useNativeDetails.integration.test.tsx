// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { St3Client } from '@smalltalk/st3-client'
import * as Native from '@smalltalk/st3-client/schema'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { describe, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { gatewayNativeDetails } from '../data/nativeDetailsSource.ts'
import { gatewaySubjectReads } from '../data/subjectReadPort.ts'
import { observed, type DataSource, type Feed } from '../data/source.ts'
import recording from '../data/subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { guardedSessionTiming, useNativeDetails, type NativeDetailsObservation } from './useNativeDetails.ts'

const nativeRow = (incarnation: string) => Native.decodeUnknownSync(Native.Agent)({ id: 'agent/example', kind: 'agent', name: 'Native title', revision: '1', updated_at: '1970-01-01T00:00:05Z', driver: 'claude', state: 'running', reachability: 'local', runtime_ids: [], incarnation_id: incarnation, current_session_id: `session/${incarnation}`, subagents: [], usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 1, output_tokens: 1, cached_tokens: 0, total_tokens: 99999999, context: { used_percent: 42, observed_at_unix_ms: 4000 } } })

describe('native Details mounted hook with real getter adapters', () => {
  it('keeps a forward-moving session start unknown across stable subsequent polls, and resets only on corrected start or incarnation', () => {
    const session = (start: number, incarnation = 'first') => Native.decodeUnknownSync(Native.Session)({ id: 'session/example', kind: 'session', owner_id: 'agent/example', runtime_incarnation: incarnation, revision: '1', started_at: new Date(start).toISOString(), updated_at: '1970-01-01T00:00:09Z', state: 'running', timeline_cursor: 'cursor/0' })
    const input = Atom.make<Feed<Native.Session>>(observed({ value: session(1000) }))
    const guarded = guardedSessionTiming(input)
    const registry = AtomRegistry.make()
    const release = registry.mount(guarded)
    try {
      expect(registry.get(guarded)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(2000) }))
      expect(registry.get(guarded)._tag).toBe('Unavailable')
      registry.set(input, observed({ value: session(2000) }))
      expect(registry.get(guarded)._tag).toBe('Unavailable')
      registry.set(input, observed({ value: session(500) }))
      expect(registry.get(guarded)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(3000, 'second') }))
      expect(registry.get(guarded)._tag).toBe('Observed')
    } finally { release(); registry.dispose() }
  })
  it('mounts selected facts only, fences restarts, and removes its getter demand when hidden', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const registry = AtomRegistry.make()
    const host = document.createElement('div')
    document.body.append(host)
    const root = createRoot(host)
    const requests: URL[] = []
    let holdGetters = false
    const selectedPaths = ['/v1/client/usage', '/v1/client/agent-declarations/agent%2Fexample', '/v1/client/sessions/second', '/v1/client/agent-queues/agent%2Fexample']
    const blocked = new Map<string, { readonly signal: AbortSignal; readonly cancellation: Promise<void> }>()
    const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      requests.push(url)
      let value: unknown
      if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
      if (holdGetters) {
        expect(selectedPaths).toContain(url.pathname)
        const signal = init?.signal
        if (signal === undefined || signal === null) throw new TypeError('Native getter must carry a cancellation signal')
        const { promise, reject } = Promise.withResolvers<Response>()
        const { promise: cancellation, resolve: cancelled } = Promise.withResolvers<void>()
        signal.addEventListener('abort', () => { cancelled(); reject(new DOMException('Aborted', 'AbortError')) }, { once: true })
        blocked.set(url.pathname, { signal, cancellation })
        return promise
      }
      if (url.pathname.endsWith('/usage')) value = { since_ms: 1000, until_ms: 5000, rows: [] }
      else if (url.pathname.includes('/agent-declarations/')) value = { id: 'agent/example', revision: '1', revisions: ['1'], kdl: '', tree: { name: 'agent', children: [] } }
      else if (url.pathname.includes('/agent-queues/')) value = { kind: 'agent-queue', agent_id: 'agent/example', current_work_ids: [], next_work_id: null, move_count: 0, moves: [], runs: [] }
      else if (url.pathname.includes('/sessions/')) {
        // The SDK strips "session/" for routing; the DTO retains the canonical ref.
        const incarnation = decodeURIComponent(url.pathname.split('/').at(-1)!)
        value = { id: `session/${incarnation}`, kind: 'session', owner_id: 'agent/example', runtime_incarnation: incarnation, revision: '1', started_at: '1970-01-01T00:00:01Z', updated_at: '1970-01-01T00:00:05Z', state: 'running', timeline_cursor: 'cursor/0' }
      }
      else throw new TypeError('Unexpected native request')
      return Response.json({ ...recording.cases.agent.payload, value })
    } }
    const client = new St3Client(options)
    const agents = Atom.make<Feed<readonly Native.Agent[]>>(observed({ value: [nativeRow('first')] }))
    const source: DataSource = { ...fixtureSource({ world: { now: 10000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), mode: 'live', agents,
      subjectReads: gatewaySubjectReads({ options, registry, discovery: client }),
      nativeDetails: gatewayNativeDetails({ options, registry, discovery: client, now: () => 10000 }),
    }
    let last: NativeDetailsObservation | undefined
    const Pane = () => { last = useNativeDetails({ agentRef: 'agent/example' }); return <div>{last.overview.title}</div> }
    try {
      expect(requests).toHaveLength(0)
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Pane /></DataSourceProvider>))
      await act(async () => { await vi.waitFor(() => expect(last?.overview.durations?.[0]?.value).toBe('9s')) })
      expect(last?.overview.durations).toMatchObject([
        { label: expect.stringMatching(/^Managed-session elapsed/), value: '9s' },
        { label: expect.stringMatching(/^Current-incarnation session age/), value: '9s' },
      ])
      expect(last?.overview.age).toBe('Incarnation launch age unreported')
      expect(last?.overview.title).toBe('Native title')
      expect(last?.harness).toBe('claude')
      expect(last?.nativeSessionId).toBeUndefined()
      expect(last?.trace.meters.context.reportedPercent).toMatchObject({ _tag: 'Known', value: 42 })
      expect(requests.filter(url => url.pathname.endsWith('/usage'))).toHaveLength(1)
      expect(requests.some(url => url.pathname.includes('root-detail') || url.pathname.includes('conversation'))).toBe(false)
      expect(last?.sections.find(section => section.title === 'Missions / queue')?.items._tag).toBe('Known')
      await act(async () => registry.set(agents, observed({ value: [nativeRow('second')] })))
      expect(last?.trace.meters.context.reportedPercent._tag).toBe('Unknown')
      await act(async () => { await vi.waitFor(() => expect(requests.filter(url => url.pathname.includes('/sessions/')).map(url => url.pathname)).toEqual(['/v1/client/sessions/first', '/v1/client/sessions/second'])) })
      await act(async () => { await vi.waitFor(() => expect(last?.overview.durations?.[0]?.value).toBe('9s')) })
      expect(last?.incarnationId).toBe('second')
      expect(last?.nativeSessionId).toBeUndefined()
      expect(last?.trace.meters.context.reportedPercent._tag).toBe('Unknown')
      // Revalidate all selected getters while mounted, then hide without disposing the registry.
      holdGetters = true
      await act(async () => {
        registry.refresh(source.subjectReads.session.feed('session/second'))
        registry.refresh(source.nativeDetails!.usagePeriod)
        registry.refresh(source.nativeDetails!.rawDeclarations({ agentRef: 'agent/example', revision: '1' }))
        registry.refresh(source.subjectReads.agentQueue.feed('agent/example'))
      })
      await act(async () => { await vi.waitFor(() => expect([...blocked.keys()].sort()).toEqual([...selectedPaths].sort())) })
      expect(blocked.size).toBe(selectedPaths.length)
      expect([...blocked.values()].every(({ signal }) => !signal.aborted)).toBe(true)
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>{null}</DataSourceProvider>))
      expect(host.textContent).toBe('')
      await vi.waitFor(() => expect([...blocked.values()].every(({ signal }) => signal.aborted)).toBe(true))
      await Promise.all([...blocked.values()].map(({ cancellation }) => cancellation))
      expect([...blocked.values()].every(({ signal }) => signal.aborted)).toBe(true)
      expect(requests.some(url => url.pathname.includes('root-detail') || url.pathname.includes('conversation'))).toBe(false)
    } finally {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    }
  })
})
