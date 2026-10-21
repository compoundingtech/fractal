// @vitest-environment jsdom
import * as React from 'react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { openAgent, type DecodedAgent } from '@fractal/sdk/effect'
import { DateTime, Option } from 'effect'
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

const nativeRow = (incarnation: string) => openAgent(Native.decodeUnknownSync(Native.Agent)({ id: 'agent/example-details', kind: 'agent', name: 'Session title', revision: '1', updated_at: '1970-01-01T00:00:05Z', driver: 'claude', state: 'running', reachability: 'local', runtime_ids: [], incarnation_id: incarnation, current_session_id: `session/${incarnation}`, subagents: [], usage: { aggregation: 'cumulative-per-incarnation-else-response-deltas', incarnation_count: 1, input_tokens: 1, output_tokens: 1, cached_tokens: 0, total_tokens: 99999999, context: { used_percent: 42, observed_at_unix_ms: 4000 } } }))
const emptySource = (): DataSource => fixtureSource({ world: { now: 10000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } })

describe('native Details mounted hook with real getter adapters', () => {
  it('retains earliest and rejected start memory across new guard atoms, scoped to source/agent/session/incarnation', () => {
    const session = (start: number, incarnation = 'first', id = 'session/example', owner = 'agent/example-details') => Native.decodeUnknownSync(Native.Session)({ id, kind: 'session', owner_id: owner, runtime_incarnation: incarnation, revision: '1', started_at: new Date(start).toISOString(), updated_at: '1970-01-01T00:00:09Z', state: 'running', timeline_cursor: 'cursor/0' })
    const source = emptySource()
    const input = Atom.make<Feed<Native.Session>>(observed({ value: session(1000) }))
    const registry = AtomRegistry.make()
    const mount = (owner = source, agentRef = 'agent/example-details') => {
      const atom = guardedSessionTiming({ input, source: owner, agentRef })
      return { atom, release: registry.mount(atom) }
    }
    let mounted = mount()
    try {
      expect(registry.get(mounted.atom)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(2000) }))
      expect(registry.get(mounted.atom)).toMatchObject({ _tag: 'Unavailable', detail: 'Session start changed between observations; elapsed time and age are unreported.' })
      mounted.release()
      mounted = mount()
      expect(registry.get(mounted.atom)._tag).toBe('Unavailable')
      const separate = mount(emptySource())
      expect(registry.get(separate.atom)._tag).toBe('Observed')
      separate.release()
      registry.set(input, observed({ value: session(500) }))
      expect(registry.get(mounted.atom)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(3000, 'second'), snapshotCreatedAt: DateTime.makeUnsafe(3000) }))
      expect(registry.get(mounted.atom)).toMatchObject({ _tag: 'Unavailable', detail: expect.stringContaining('producer snapshot time') })
      mounted.release()
      mounted = mount()
      // A later producer snapshot does not rehabilitate the same previously rejected start.
      registry.set(input, observed({ value: session(3000, 'second'), snapshotCreatedAt: DateTime.makeUnsafe(5000) }))
      expect(registry.get(mounted.atom)._tag).toBe('Unavailable')
      registry.set(input, observed({ value: session(2500, 'second'), snapshotCreatedAt: DateTime.makeUnsafe(5000) }))
      expect(registry.get(mounted.atom)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(3000, 'third'), snapshotCreatedAt: DateTime.makeUnsafe(5000) }))
      expect(registry.get(mounted.atom)._tag).toBe('Observed')
      registry.set(input, observed({ value: session(4000, 'third', 'session/unrelated'), snapshotCreatedAt: DateTime.makeUnsafe(5000) }))
      expect(registry.get(mounted.atom)._tag).toBe('Observed')
      const otherAgent = mount(source, 'agent/example-other')
      registry.set(input, observed({ value: session(4000, 'third', 'session/example', 'agent/example-other'), snapshotCreatedAt: DateTime.makeUnsafe(5000) }))
      expect(registry.get(otherAgent.atom)._tag).toBe('Observed')
      otherAgent.release()
    } finally { mounted.release(); registry.dispose() }
  })
  it('never renders fresh seven-second durations from the real producer snapshot, including Details remount and earlier correction', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const registry = AtomRegistry.make()
    const host = document.createElement('div')
    document.body.append(host)
    const root = createRoot(host)
    let startedAt = '1970-01-01T00:00:03Z'
    let snapshotAt = '1970-01-01T00:00:03Z'
    let sessionReads = 0
    const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
      if (url.pathname.includes('/agent-queues/')) return Response.json({ ...recording.cases.agent.payload, value: { kind: 'agent-queue', agent_id: 'agent/example-details', current_work_ids: [], next_work_id: null, move_count: 0, moves: [], runs: [] } })
      if (!url.pathname.includes('/sessions/')) throw new TypeError('Unexpected native request')
      sessionReads++
      return Response.json({ ...recording.cases.agent.payload, snapshot: { ...recording.cases.agent.payload.snapshot, created_at: snapshotAt },
        value: { id: 'session/first', kind: 'session', owner_id: 'agent/example-details', runtime_incarnation: 'first', revision: '1', started_at: startedAt, updated_at: '1970-01-01T00:00:05Z', state: 'running', timeline_cursor: 'cursor/0' } })
    } }
    const agents = Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: [nativeRow('first')] }))
    const source: DataSource = { ...emptySource(), agents, subjectReads: gatewaySubjectReads({ options, registry }) }
    let last: NativeDetailsObservation | undefined
    const Pane = () => { last = useNativeDetails({ agentRef: 'agent/example-details' }); return <div>{last.overview.durations?.map(item => <div key={item.label}>{item.value}<span>{item.detail}</span></div>)}</div> }
    try {
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Pane /></DataSourceProvider>))
      await act(async () => { await vi.waitFor(() => expect(last?.overview.durations?.[0]?.detail).toContain('producer snapshot time')) })
      expect(last?.overview.durations?.map(item => item.value)).toEqual(['Unreported', 'Unreported'])
      expect(host.textContent).not.toContain('7s')
      expect(last?.incarnationId).toBe('first')
      expect(last?.trace.meters.spend.tokens).toMatchObject({ _tag: 'Known', value: 99999999 })
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}>{null}</DataSourceProvider>))
      // A new response's snapshot is no longer equal, but the original rejection must survive.
      snapshotAt = '1970-01-01T00:00:08Z'
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Pane /></DataSourceProvider>))
      await act(async () => { await vi.waitFor(() => expect(sessionReads).toBe(2)) })
      expect(last?.overview.durations?.map(item => item.value)).toEqual(['Unreported', 'Unreported'])
      expect(host.textContent).toContain('producer snapshot time')
      expect(host.textContent).not.toContain('7s')
      startedAt = '1970-01-01T00:00:01Z'
      await act(async () => registry.refresh(source.subjectReads.session.feed('session/first')))
      await act(async () => { await vi.waitFor(() => expect(last?.overview.durations?.map(item => item.value)).toEqual(['9s', '9s'])) })
      expect(last?.overview.durations?.every(item => item.detail === undefined)).toBe(true)
      expect(host.textContent).not.toContain('snapshot time')
    } finally {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    }
  })
  it('mounts selected facts only, fences restarts, and removes its getter demand when hidden', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const registry = AtomRegistry.make()
    const host = document.createElement('div')
    document.body.append(host)
    const root = createRoot(host)
    const requests: URL[] = []
    let holdGetters = false
    const selectedPaths = ['/v1/client/usage', '/v1/client/agent-declarations/agent%2Fexample-details', '/v1/client/sessions/second', '/v1/client/agent-queues/agent%2Fexample-details']
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
      else if (url.pathname.includes('/agent-declarations/')) value = { id: 'agent/example-details', revision: '1', revisions: ['1'], kdl: '', tree: { name: 'agent', children: [] } }
      else if (url.pathname.includes('/agent-queues/')) value = { kind: 'agent-queue', agent_id: 'agent/example-details', current_work_ids: [], next_work_id: null, move_count: 0, moves: [], runs: [] }
      else if (url.pathname.includes('/sessions/')) {
        // The SDK strips "session/" for routing; the DTO retains the canonical ref.
        const incarnation = decodeURIComponent(url.pathname.split('/').at(-1)!)
        value = { id: `session/${incarnation}`, kind: 'session', owner_id: 'agent/example-details', runtime_incarnation: incarnation, revision: '1', started_at: '1970-01-01T00:00:01Z', updated_at: '1970-01-01T00:00:05Z', state: 'running', timeline_cursor: 'cursor/0' }
      }
      else throw new TypeError('Unexpected native request')
      return Response.json({ ...recording.cases.agent.payload, value })
    } }
    const client = new FractalClient(options)
    const agents = Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: [nativeRow('first')] }))
    const source: DataSource = { ...fixtureSource({ world: { now: 10000, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' } } }), mode: 'live', agents,
      subjectReads: gatewaySubjectReads({ options, registry, discovery: client }),
      nativeDetails: gatewayNativeDetails({ options, registry, discovery: client, now: () => 10000 }),
    }
    let last: NativeDetailsObservation | undefined
    const Pane = () => { last = useNativeDetails({ agentRef: 'agent/example-details' }); return <div>{last.overview.title}</div> }
    try {
      expect(requests).toHaveLength(0)
      await act(async () => root.render(<DataSourceProvider source={source} registry={registry}><Pane /></DataSourceProvider>))
      await act(async () => { await vi.waitFor(() => expect(last?.overview.durations?.[0]?.value).toBe('9s')) })
      expect(last?.overview.durations).toMatchObject([
        { label: 'Session elapsed', value: '9s' },
        { label: 'Session age', value: '9s' },
      ])
      expect(last?.overview.age).toBeUndefined()
      expect(last?.overview.title).toBe('Session title')
      expect(last?.harness).toBe('claude')
      expect(last?.nativeSessionId).toBeUndefined()
      expect(last?.trace.meters.context.reportedPercent).toMatchObject({ _tag: 'Known', value: 42 })
      expect(requests.filter(url => url.pathname.endsWith('/usage'))).toHaveLength(1)
      expect(requests.some(url => url.pathname.includes('root-detail') || url.pathname.includes('conversation'))).toBe(false)
      expect(last?.sections.find(section => section.title === 'Missions')?.items._tag).toBe('Known')
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
        registry.refresh(source.nativeDetails!.usagePeriod('agent/example-details'))
        registry.refresh(source.nativeDetails!.rawDeclarations({ agentRef: 'agent/example-details', revision: '1' }))
        registry.refresh(source.subjectReads.agentQueue.feed('agent/example-details'))
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
  it('retries each failed getter alone, refreshes spend on request, and reads spend fresh for each agent', async () => {
    vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
    const registry = AtomRegistry.make()
    const host = document.createElement('div')
    document.body.append(host)
    const root = createRoot(host)
    const reads: string[] = []
    const failing = new Set(['/v1/client/usage', '/v1/client/sessions/first', '/v1/client/agent-queues/agent%2Fexample-details'])
    const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
      reads.push(url.pathname)
      if (failing.has(url.pathname)) throw new TypeError('Connection lost')
      let value: unknown
      if (url.pathname.endsWith('/usage')) value = { since_ms: 1000, until_ms: 5000, rows: [] }
      else if (url.pathname.includes('/agent-declarations/')) value = { id: decodeURIComponent(url.pathname.split('/').at(-1)!), revision: '1', revisions: ['1'], kdl: '', tree: { name: 'agent', children: [] } }
      else if (url.pathname.includes('/agent-queues/')) value = { kind: 'agent-queue', agent_id: decodeURIComponent(url.pathname.split('/').at(-1)!), current_work_ids: [], next_work_id: null, move_count: 0, moves: [], runs: [] }
      else if (url.pathname.includes('/sessions/')) value = { id: 'session/first', kind: 'session', owner_id: 'agent/example-details', runtime_incarnation: 'first', revision: '1', started_at: '1970-01-01T00:00:01Z', updated_at: '1970-01-01T00:00:05Z', state: 'running', timeline_cursor: 'cursor/0' }
      else throw new TypeError('Unexpected native request')
      return Response.json({ ...recording.cases.agent.payload, value })
    } }
    const count = (path: string) => reads.filter(read => read === path).length
    const client = new FractalClient(options)
    const other = { ...nativeRow('first'), id: Native.decodeUnknownSync(Native.AgentId)('agent/example-other'), current_session_id: Option.none() }
    const agents = Atom.make<Feed<readonly DecodedAgent[]>>(observed({ value: [nativeRow('first'), other] }))
    const source: DataSource = { ...emptySource(), mode: 'live', agents,
      subjectReads: gatewaySubjectReads({ options, registry, discovery: client }),
      nativeDetails: gatewayNativeDetails({ options, registry, discovery: client, now: () => 10000 }),
    }
    let last: NativeDetailsObservation | undefined
    const Pane = ({ agentRef }: { readonly agentRef: string }) => { last = useNativeDetails({ agentRef }); return null }
    const show = (agentRef: string | undefined) => act(async () => root.render(<DataSourceProvider source={source} registry={registry}>{agentRef === undefined ? null : <Pane agentRef={agentRef} />}</DataSourceProvider>))
    const missions = () => last?.sections.find(section => section.title === 'Missions')
    try {
      await show('agent/example-details')
      await act(async () => { await vi.waitFor(() => {
        expect(last?.spendRead?.failure?.message).toBe("Couldn't load spend.")
        expect(last?.sessionFailure?.message).toBe("Couldn't load session details.")
        expect(missions()?.failure?.message).toBe("Couldn't load the agent queue.")
      }) })
      // Healthy sections keep their content beside the failures.
      expect(last?.overview.title).toBe('Session title')
      failing.clear()
      const before = { usage: count('/v1/client/usage'), session: count('/v1/client/sessions/first'), queue: count('/v1/client/agent-queues/agent%2Fexample-details') }
      // Each Try again re-reads only its own getter.
      await act(async () => last?.sessionFailure?.onRetry())
      await act(async () => { await vi.waitFor(() => expect(last?.sessionFailure).toBeUndefined()) })
      expect([count('/v1/client/usage'), count('/v1/client/sessions/first'), count('/v1/client/agent-queues/agent%2Fexample-details')]).toEqual([before.usage, before.session + 1, before.queue])
      await act(async () => missions()?.failure?.onRetry())
      await act(async () => { await vi.waitFor(() => expect(missions()?.failure).toBeUndefined()) })
      expect([count('/v1/client/usage'), count('/v1/client/agent-queues/agent%2Fexample-details')]).toEqual([before.usage, before.queue + 1])
      await act(async () => last?.spendRead?.failure?.onRetry())
      await act(async () => { await vi.waitFor(() => expect(last?.spendRead).toMatchObject({ updatedAt: 5000, refreshing: false })) })
      expect(last?.spendRead?.failure).toBeUndefined()
      expect(count('/v1/client/usage')).toBe(before.usage + 1)
      // Refresh is exactly one more spend read.
      await act(async () => last?.spendRead?.onRefresh())
      await act(async () => { await vi.waitFor(() => expect(count('/v1/client/usage')).toBe(before.usage + 2)) })
      await act(async () => { await vi.waitFor(() => expect(last?.spendRead?.refreshing).toBe(false)) })
      // Switching agent reads that agent's spend once; hidden Details reads nothing more.
      await show('agent/example-other')
      await act(async () => { await vi.waitFor(() => expect(last?.spendRead?.refreshing).toBe(false)) })
      expect(count('/v1/client/usage')).toBe(before.usage + 3)
      await show(undefined)
      const settled = Promise.withResolvers<void>()
      setTimeout(settled.resolve, 50)
      await settled.promise
      expect(count('/v1/client/usage')).toBe(before.usage + 3)
    } finally {
      await act(async () => root.unmount())
      registry.dispose()
      host.remove()
      vi.unstubAllGlobals()
    }
  })
})
