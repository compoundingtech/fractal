import { createServer } from 'node:http'
import type { IncomingMessage, ServerResponse } from 'node:http'
import { once } from 'node:events'
import { expect, it, vi } from 'vitest'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { DetailsSupplementQuery } from '../extensions/contract.ts'
import { createDetailsSupplementAtom, createDetailsSupplementCache, relayRootDetailSource } from './rootDetail.ts'

const root = '8cf4a5a1-3423-4e43-86cc-615f442f4ce2'
const query: DetailsSupplementQuery = { agentRef: 'agent/fictional', harness: 'omp', host: 'host/fictional', incarnationId: 'fixture-incarnation', nativeSessionId: root }
const answer = { agent: query.agentRef, host: query.host, incarnationId: query.incarnationId, nativeSessionId: root, window: '1d', inclusiveUsd: 99, childCount: 0, children: [], attributedFraction: 0.98 }

/** A loopback relay stand-in; the source reads it at `${origin}/v1/client/usage/root-detail`. */
const withRelay = async (handler: (req: IncomingMessage, res: ServerResponse) => void, use: (origin: string) => Promise<void>) => {
  const server = createServer(handler)
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const address = server.address()
  if (address === null || typeof address === 'string') throw new TypeError('Missing fixture listener')
  try { await use(`http://127.0.0.1:${address.port}`) }
  finally {
    server.closeAllConnections()
    const closed = Promise.withResolvers<void>()
    server.close(() => closed.resolve())
    await closed.promise
  }
}
const relay = (origin: string, refreshMs = 30_000) => relayRootDetailSource({ origin, fetchImpl: fetch, refreshMs })

it('waits for native root facts and reads nothing for an ineligible root', async () => {
  const registry = AtomRegistry.make()
  const fetchImpl = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => Response.json(answer))
  const cache = createDetailsSupplementCache(relayRootDetailSource({ fetchImpl }))
  try {
    expect(registry.get(cache({ ...query, nativeSessionId: undefined, nativeRootPending: true, canRefresh: false }))).toEqual({ _tag: 'Waiting' })
    expect(registry.get(cache({ ...query, nativeSessionId: undefined }))).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-attributed' })
    for (const harness of ['codex', 'claude']) {
      const atom = cache({ ...query, harness })
      const release = registry.mount(atom)
      expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-not-covered' })
      release()
    }
    expect(fetchImpl).not.toHaveBeenCalled()
    const ready = cache({ ...query, nativeRootPending: false })
    const release = registry.mount(ready)
    await vi.waitFor(() => expect(registry.get(ready)).toMatchObject({ _tag: 'Observed', value: { totalUsd: { value: 99 } } }))
    expect(fetchImpl).toHaveBeenCalledTimes(1)
    expect(fetchImpl.mock.calls[0]?.[0]).toBe('/v1/client/usage/root-detail?agent=agent%2Ffictional&window=1d')
    expect(cache(query)).toBe(ready)
    release()
  } finally { registry.dispose() }
})

it('reads the declared window, once per mounted demand, and aborts the read on release', async () => {
  const started = Promise.withResolvers<void>()
  const closed = Promise.withResolvers<void>()
  const registry = AtomRegistry.make()
  const paths: (string | undefined)[] = []
  await withRelay((req, res) => {
    paths.push(req.url)
    res.once('close', closed.resolve)
    res.writeHead(200, { 'content-type': 'application/json', connection: 'close' })
    res.write('{')
    started.resolve()
  }, async (origin) => {
    try {
      const atom = createDetailsSupplementAtom({ source: relayRootDetailSource({ origin, fetchImpl: fetch, window: '7d' }), query })
      expect(paths).toEqual([])
      const first = registry.mount(atom)
      const second = registry.mount(atom)
      await started.promise
      first()
      second()
      await closed.promise
      expect(paths).toEqual(['/v1/client/usage/root-detail?agent=agent%2Ffictional&window=7d'])
    } finally { registry.dispose() }
  })
})

it('bounds a cold read at 215 s and aborts its transport', async () => {
  vi.useFakeTimers()
  const registry = AtomRegistry.make()
  let aborted = false
  const fetchImpl: typeof fetch = (_input, init) => {
    const pending = Promise.withResolvers<Response>()
    init?.signal?.addEventListener('abort', () => { aborted = true; pending.reject(new DOMException('Aborted', 'AbortError')) }, { once: true })
    return pending.promise
  }
  const atom = createDetailsSupplementAtom({ source: relayRootDetailSource({ fetchImpl }), query })
  const release = registry.mount(atom)
  try {
    await vi.advanceTimersByTimeAsync(214_999)
    expect(registry.get(atom)).toEqual({ _tag: 'Waiting' })
    expect(aborted).toBe(false)
    await vi.advanceTimersByTimeAsync(2)
    expect(aborted).toBe(true)
    expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'failed' })
  } finally { release(); registry.dispose(); vi.useRealTimers() }
})

it('rejects an answer for another identity rather than showing its spend', async () => {
  const registry = AtomRegistry.make()
  await withRelay((_req, res) => res.end(JSON.stringify(answer)), async (origin) => {
    const atom = createDetailsSupplementAtom({ source: relay(origin), query: { ...query, incarnationId: 'replacement-incarnation' } })
    const release = registry.mount(atom)
    try { await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', code: 'root-detail-not-attributed' })) }
    finally { release(); registry.dispose() }
  })
})

it('maps relay refusals to explicit unavailable states, never an empty supplement', async () => {
  const cases = [
    [403, 'forbidden', 'ungranted'],
    [404, 'root-detail-not-found', 'unsupported'],
    [422, 'root-detail-not-attributed', 'unsupported'],
    [502, 'root-detail-deadline', 'failed'],
  ] as const
  for (const [status, code, reason] of cases) {
    const registry = AtomRegistry.make()
    await withRelay((_req, res) => { res.statusCode = status; res.end(JSON.stringify({ code, message: 'Fixture refusal' })) }, async (origin) => {
      const atom = createDetailsSupplementAtom({ source: relay(origin), query })
      const release = registry.mount(atom)
      try { await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason, code })) }
      finally { release(); registry.dispose() }
    })
  }
})

it('treats a 404 the relay did not write as no relay, and stops reading for every root of that source', async () => {
  const registry = AtomRegistry.make()
  let requests = 0
  await withRelay((_req, res) => { requests++; res.statusCode = 404; res.end() }, async (origin) => {
    const cache = createDetailsSupplementCache(relay(origin, 5))
    const atom = cache(query)
    const release = registry.mount(atom)
    try {
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-unconfigured' }))
      const other = cache({ ...query, agentRef: 'agent/other' })
      const releaseOther = registry.mount(other)
      expect(registry.get(other)).toMatchObject({ code: 'root-detail-unconfigured' })
      releaseOther()
      // Real time on purpose: the loopback relay answers over a socket, so fake timers cannot drive it.
      // 50 ms spans ten refresh periods of the 5 ms cadence; a latched source issues no further reads.
      const pause = Promise.withResolvers<void>()
      setTimeout(pause.resolve, 50)
      await pause.promise
      expect(requests).toBe(1)
    } finally { release(); registry.dispose() }
  })
})

it('refreshes under demand, marks a transient failure stale and clears the answer when the grant is refused', async () => {
  const registry = AtomRegistry.make()
  const second = Promise.withResolvers<ServerResponse>()
  const third = Promise.withResolvers<ServerResponse>()
  const fourth = Promise.withResolvers<void>()
  const fourthClosed = Promise.withResolvers<void>()
  let requests = 0
  await withRelay((_req, res) => {
    requests++
    if (requests === 1) res.end(JSON.stringify(answer))
    else if (requests === 2) second.resolve(res)
    else if (requests === 3) third.resolve(res)
    else {
      res.once('close', fourthClosed.resolve)
      res.writeHead(200, { 'content-type': 'application/json', connection: 'close' })
      res.write('{')
      fourth.resolve()
    }
  }, async (origin) => {
    const atom = createDetailsSupplementAtom({ source: relay(origin, 30), query })
    const release = registry.mount(atom)
    try {
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', freshness: 'live' }))
      const failed = await second.promise
      failed.statusCode = 503
      failed.end(JSON.stringify({ code: 'root-detail-busy', message: 'Fixture outage' }))
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', freshness: 'stale', value: { totalUsd: { value: 99 } }, error: { reason: 'failed' } }))
      const denied = await third.promise
      denied.statusCode = 403
      denied.end(JSON.stringify({ code: 'forbidden', message: 'Forbidden' }))
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'ungranted' }))
      await fourth.promise
      release()
      await fourthClosed.promise
      expect(requests).toBe(4)
    } finally { registry.dispose() }
  })
})

it('keeps a successful answer through a failure, serves it paused without reading, and isolates roots and sources', async () => {
  const registry = AtomRegistry.make()
  let status = 200
  let requests = 0
  let now = 1000
  await withRelay((_req, res) => {
    requests++
    res.writeHead(status, { 'content-type': 'application/json' })
    res.end(JSON.stringify(status === 200 ? answer : { code: status === 403 ? 'forbidden' : 'root-detail-unavailable', message: 'Fixture refusal' }))
  }, async (origin) => {
    const source = relay(origin, 5)
    const cache = createDetailsSupplementCache(source, () => now)
    const atom = cache(query)
    let release = registry.mount(atom)
    try {
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', freshness: 'live', value: { totalUsd: { value: 99 }, receivedAtMs: 1000 } }))
      status = 502
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', freshness: 'stale', error: { reason: 'failed' }, value: { totalUsd: { value: 99 } } }))
      now = 2000 // A failed or paused read keeps the last successful receipt time.
      release()
      const paused = cache({ ...query, canRefresh: false })
      release = registry.mount(paused)
      expect(registry.get(paused)).toMatchObject({ _tag: 'Observed', freshness: 'stale', value: { receivedAtMs: 1000 } })
      const before = requests
      expect(registry.get(cache({ ...query, canRefresh: false }))).toMatchObject({ _tag: 'Observed', value: { totalUsd: { value: 99 } } })
      expect(requests).toBe(before)
      expect(registry.get(cache({ ...query, incarnationId: 'different', canRefresh: false }))._tag).toBe('Unavailable')
      expect(registry.get(createDetailsSupplementCache(source)({ ...query, canRefresh: false }))._tag).toBe('Unavailable')
      release()
      status = 403
      release = registry.mount(cache(query))
      await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable', reason: 'ungranted' }))
      expect(registry.get(cache({ ...query, canRefresh: false }))).toMatchObject({ _tag: 'Unavailable', reason: 'ungranted' })
    } finally { release(); registry.dispose() }
  })
})
