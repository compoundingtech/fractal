import { afterEach, describe, expect, it } from 'vitest'
import { once } from 'node:events'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { AgentEncoded, UsageRowEncoded } from '@fractal/client/schema/read'
import { Tracer } from 'effect'
import { createServer as createViteServer } from 'vite'
import { createFractalWebServer } from './core.mts'
import type { MiddlewareOptions } from './core.mts'
import { loopbackProducer } from './relayHttp.mts'
import { createRootDetailRelay, rootDetailRoute, type RootDetailWindow } from './rootDetail.mts'
import { webfractalGateway } from '../scripts/vite-gateway.mjs'
import { admittedNativeApiVersions } from './nativeEnvelope.mts'

// Assembled so the source itself never carries a literal bearer credential pair.
const publisherAuthorization = 'Bearer' + ' fx-publisher'
const tagAuthorization = 'Bearer' + ' fx-tag-reader'
const browserBearer = 'b'.repeat(43)
const root = '8cf4a5a1-3423-4e43-86cc-615f442f4ce2'
const otherRoot = '43d1ab0d-5570-4f28-8a68-e51f9819d6ee'
const agentRef = 'agent/fictional/relay'
const nativeOwner = 'host/fixture-owner'
const detailUrl = (window: string = '1d', agent = agentRef) => `${rootDetailRoute}?${new URLSearchParams({ agent, window })}`
const producerPath = (window: RootDetailWindow = '1d') => `/api/v1/lens/root_detail?root=${root}&window=${window}`

const agent = (): AgentEncoded => ({
  id: agentRef, kind: 'agent', name: 'Fictional relay control', revision: '1', updated_at: '2026-10-11T00:00:00Z',
  host_id: nativeOwner, runtime_ids: [], reachability: 'local', state: 'running', driver: 'omp', incarnation_id: 'incarnation-fixture',
  todo: { claim_id: 'claim/fictional', accepted_at: '2026-10-11T00:00:00Z', stale: false, snapshot: {
    harness: 'omp', incarnation_id: 'incarnation-fixture', session_id: root, observed_at: '2026-10-11T00:00:00Z', source_op: 'block', phases: [], truncated: false,
    totals: { pending: 0, in_progress: 0, completed: 0, blocked: 0 },
  } },
})
const suspension = (nativeSessionId: string): AgentEncoded['suspension'] => ({
  action: 'resume', phase: 'resumed', operation_id: 'operation/fictional', blocking: [], incarnation_id: 'incarnation-fixture',
  native_session_id: nativeSessionId, harness: 'omp', host: nativeOwner, updated_at: '2026-10-11T00:00:00Z',
})
const usage = (): UsageRowEncoded => ({
  agent: agentRef, host: nativeOwner, native_session_id: root, cache_write_1h_tokens: 0, cache_write_tokens: 0, cached_tokens: 0, cost_microusd: 10,
  input_tokens: 1, output_tokens: 1, reported_cost_microusd: 10, total_tokens: 2, unpriced_tokens: 0,
})
const produced = (window: RootDetailWindow = '1d') => ({
  key: 'root_detail', lens_version: 15, root_session_id: root, window, host: 'fictional-ledger-host', usd: 543.21, child_count: 25, coverage_ok: true,
  coverage: { account_share: 0.98 }, wall_hours: 2.4, active_hours: 0.7, worktree: '/not-disclosed', seat: 'undisclosed-seat', hourly: [],
  children: Array.from({ length: 25 }, (_, i) => ({ session_id: `child-${i}`, role: 'subagent', model: i === 0 ? '.unknown' : 'reported-model', usd: 1 + i / 10, requests: i + 1 })),
})
const envelope = (value: unknown, apiVersion = 'fractal.client.v0') => JSON.stringify({ api_version: apiVersion, value })
const capabilities = (granted: boolean, apiVersion?: string) => envelope({ kind: 'capabilities', capabilities: [{ id: 'read.projections', state: granted ? 'granted' : 'ungranted' }] }, apiVersion)

interface Received { readonly path: string | undefined; readonly authorization: string | undefined; readonly cookie: string | undefined }
const received = (req: IncomingMessage): Received => ({ path: req.url, authorization: req.headers.authorization, cookie: req.headers.cookie })

const cleanups: (() => Promise<void>)[] = []
afterEach(async () => { while (cleanups.length) await cleanups.pop()?.() })
const listen = async (server: Server, socketPath?: string): Promise<void> => {
  if (socketPath === undefined) server.listen(0, '127.0.0.1')
  else server.listen(socketPath)
  await once(server, 'listening')
}
const close = async (server: Server): Promise<void> => {
  if (!server.listening) return
  server.closeAllConnections()
  const closed = Promise.withResolvers<void>()
  server.close(() => closed.resolve())
  await closed.promise
}
const portOf = (server: Server): number => {
  const address = server.address()
  if (address === null || typeof address === 'string') throw new TypeError('Missing fixture listener')
  return address.port
}

interface FixtureState {
  versions: Partial<Record<'capabilities' | 'agent' | 'usage', string>>
  grantStatus: number
  granted: boolean
  usageStatus: number
  agent: AgentEncoded
  agentBody?: string
  usage: UsageRowEncoded[]
  body: string
  producerStatus: number
  producerMode: 'answer' | 'trickle'
  revokeOnProducerRead: boolean
  echoWindow: boolean
}
/** A gateway socket answering capabilities, the agent and its usage period, plus a loopback producer. */
const upstreams = async () => {
  const dir = await mkdtemp(join(tmpdir(), 'root-detail-test-'))
  cleanups.push(() => rm(dir, { recursive: true, force: true }))
  const state: FixtureState = {
    versions: {},
    grantStatus: 200, granted: true, usageStatus: 200, agent: agent(), usage: [usage()], body: JSON.stringify(produced()),
    producerStatus: 200, producerMode: 'answer', revokeOnProducerRead: false, echoWindow: true,
  }
  const gatewayReads: Received[] = []
  const forwarded: Received[] = []
  const producerReads: Received[] = []
  const gateway = createServer((req, res) => {
    const path = req.url ?? ''
    if (path === '/v1/client/capabilities') {
      gatewayReads.push(received(req))
      res.writeHead(state.grantStatus, { 'content-type': 'application/json' })
      res.end(capabilities(state.granted, state.versions.capabilities))
    } else if (path === `/v1/client/agents/${encodeURIComponent(agentRef)}`) {
      gatewayReads.push(received(req))
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(state.agentBody ?? envelope(state.agent, state.versions.agent))
    } else if (path.startsWith('/v1/client/usage?')) {
      gatewayReads.push(received(req))
      res.writeHead(state.usageStatus, { 'content-type': 'application/json' })
      res.end(envelope({ since_ms: 0, until_ms: 1, rows: state.usage }, state.versions.usage))
    } else {
      forwarded.push(received(req))
      res.writeHead(404)
      res.end()
    }
  })
  const socketPath = join(dir, 'gateway.sock')
  await listen(gateway, socketPath)
  cleanups.push(() => close(gateway))
  const producer = createServer((req, res) => {
    producerReads.push(received(req))
    if (state.revokeOnProducerRead) state.granted = false
    if (state.producerMode === 'trickle') {
      // Real time on purpose: the relay's elapsed deadline must fire while these bytes keep the socket active.
      res.writeHead(200, { 'content-type': 'application/json' })
      const trickle = setInterval(() => res.write(' '), 20)
      res.once('close', () => clearInterval(trickle))
      return
    }
    const window = new URL(req.url ?? '/', 'http://producer.invalid').searchParams.get('window')
    res.writeHead(state.producerStatus, { 'content-type': 'application/json' })
    res.end(state.echoWindow && (window === '1d' || window === '7d') && state.body.startsWith('{') ? JSON.stringify({ ...JSON.parse(state.body), window }) : state.body)
  })
  await listen(producer)
  cleanups.push(() => close(producer))
  return { dir, state, gatewayReads, forwarded, producerReads, socketPath, producer, producerUrl: `http://127.0.0.1:${portOf(producer)}` }
}

const fixture = async ({ enabled = true, admit = () => true, pairing = false }: {
  readonly enabled?: boolean
  readonly admit?: MiddlewareOptions['admit']
  readonly pairing?: boolean
} = {}) => {
  const up = await upstreams()
  await writeFile(join(up.dir, 'index.html'), '<html><head></head><body>fixture</body></html>')
  const app = createFractalWebServer({ dist: up.dir, host: '127.0.0.1', port: 0, admit,
    tracer: Tracer.make({ span: (options) => new Tracer.NativeSpan(options) }),
    ...(pairing ? { pairing: { admit: () => true } } : {}),
    gateway: { socketPath: up.socketPath, host: 'paired-gateway', authorization: publisherAuthorization, tagAuthorization, timeoutMs: 1000 },
    identity: { deploymentId: 'fixture-build', buildIdentity: { baseVersion: '1.0.0', displayVersion: '1.0.0-test', machineVersion: '1.0.0+test', sourceKind: 'local', dirty: false } },
    rootDetail: enabled ? loopbackProducer(up.producerUrl, 'WF_ROOT_DETAIL_URL') : undefined,
  })
  app.listen()
  await once(app.server, 'listening')
  cleanups.push(() => app.close())
  return { ...up, base: `http://127.0.0.1:${portOf(app.server)}` }
}

/** The relay alone on a plain listener, for a producer deadline short enough to test. */
const directRelay = async (up: { readonly producerUrl: string; readonly socketPath: string }, producerDeadlineMs: number) => {
  const producer = loopbackProducer(up.producerUrl, 'WF_ROOT_DETAIL_URL')
  if (producer === undefined) throw new TypeError('Missing fixture producer')
  const relay = createRootDetailRelay({ producer, producerDeadlineMs,
    gateway: { socketPath: up.socketPath, host: 'paired-gateway', authorization: publisherAuthorization, timeoutMs: 1000 } })
  const server = createServer((req, res) => { void relay.handle(req, res, false).catch(() => res.destroy()) })
  await listen(server)
  cleanups.push(() => close(server))
  return `http://127.0.0.1:${portOf(server)}`
}

const expectFailure = async (response: Response, status: number, code: string) => {
  expect(response.status).toBe(status)
  expect(response.headers.get('cache-control')).toBe('no-store')
  const body = await response.json() as { code: string; message: string }
  expect(body.code).toBe(code)
  expect(typeof body.message).toBe('string')
  return body
}

describe('root-detail relay', () => {
  it('admits the version emitted by the native server', async () => {
    // Generated sender defaults still use the legacy name; check the actual server authority.
    const source = await readFile(new URL('../../../crates/fractal-api-core/src/api.rs', import.meta.url), 'utf8')
    const declarations = [...source.matchAll(/^\s*pub\s+const\s+CLIENT_API_VERSION\s*:\s*&\s*(?:'static\s+)?str\s*=\s*"([^"]+)"\s*;/gm)]
    expect(declarations, 'Expected exactly one native CLIENT_API_VERSION string declaration; update this contract test if its syntax changes').toHaveLength(1)
    expect(admittedNativeApiVersions, 'The native server changed CLIENT_API_VERSION; update receiver compatibility before shipping').toContain(declarations[0]?.[1])
  })

  it.each(admittedNativeApiVersions)('accepts %s envelopes and additive producer fields', async apiVersion => {
    const setup = await fixture()
    setup.state.versions = { capabilities: apiVersion, agent: apiVersion, usage: apiVersion }
    const response = await fetch(setup.base + detailUrl())
    expect(response.status).toBe(200)
    const body = await response.json()
    expect(body).toMatchObject({ inclusiveUsd: 543.21 })
    for (const key of ['lens_version', 'worktree', 'seat', 'hourly', 'coverage_ok']) expect(body).not.toHaveProperty(key)
  })

  it.each((['capabilities', 'agent', 'usage'] as const).flatMap(target => ['fractal.client.v999', 'future.client.v0'].map(apiVersion => ({ target, apiVersion }))))('names unknown $target envelope version $apiVersion without reading the producer', async ({ target, apiVersion }) => {
    const setup = await fixture()
    setup.state.versions[target] = apiVersion
    const response = await fetch(setup.base + detailUrl())
    expect(response.status).toBe(502)
    expect(await response.json()).toEqual({
      code: 'root-detail-gateway-unavailable',
      message: `native envelope; api_version=${apiVersion}; admitted=${admittedNativeApiVersions.join(',')}; $.api_version: expected "st3.client.v0" | "fractal.client.v0"`,
    })
    expect(setup.producerReads).toHaveLength(0)
  })

  it('names the native admitted set for malformed JSON without disclosing response data', async () => {
    const setup = await fixture()
    setup.state.agentBody = `not JSON ${JSON.stringify(produced())}`
    const response = await fetch(setup.base + detailUrl())
    const body = await expectFailure(response, 502, 'root-detail-gateway-unavailable')
    expect(body).toEqual({
      code: 'root-detail-gateway-unavailable',
      message: `native envelope; api_version=unavailable; admitted=${admittedNativeApiVersions.join(',')}; $: expected JSON object`,
    })
    expect(setup.producerReads).toHaveLength(0)
  })

  it.each(['wrong type', 'missing'] as const)('names a native field with %s without disclosing response data', async kind => {
    const setup = await fixture()
    if (kind === 'missing') Reflect.deleteProperty(setup.state.agent, 'name')
    else Reflect.set(setup.state.agent, 'name', { private: produced() })
    const response = await fetch(setup.base + detailUrl())
    expect(response.status).toBe(502)
    const body = await response.json()
    expect(body.code).toBe('root-detail-gateway-unavailable')
    expect(body.message).toContain('native envelope; api_version=fractal.client.v0; admitted=')
    expect(body.message).toContain('$.value.name: expected string')
    for (const value of [agentRef, nativeOwner, root, '543.21', 'private']) expect(body.message).not.toContain(value)
  })

  it('derives the root from the gateway and returns only allowlisted fields, at most 20 descendants', async () => {
    const setup = await fixture()
    const response = await fetch(setup.base + detailUrl(), { headers: { cookie: 'browser=fixture' } })
    expect(response.status).toBe(200)
    expect(response.headers.get('cache-control')).toBe('no-store')
    const body = await response.json() as Record<string, unknown> & { children: readonly Record<string, unknown>[] }
    expect(Object.keys(body).sort()).toEqual(['activeHours', 'agent', 'attributedFraction', 'childCount', 'children', 'host', 'incarnationId',
      'inclusiveUsd', 'nativeSessionId', 'producerHost', 'wallHours', 'window'])
    expect(body).toMatchObject({ agent: agentRef, host: nativeOwner, incarnationId: 'incarnation-fixture', nativeSessionId: root, window: '1d',
      inclusiveUsd: 543.21, childCount: 25, attributedFraction: 0.98, wallHours: 2.4, activeHours: 0.7, producerHost: 'fictional-ledger-host' })
    expect(body.children).toHaveLength(20)
    expect(body.children[0]).toEqual({ sessionId: 'child-0', role: 'subagent', model: '.unknown', usd: 1, requests: 1 })
    expect(setup.producerReads).toEqual([{ path: producerPath(), authorization: undefined, cookie: undefined }])
    const paths = setup.gatewayReads.map((read) => read.path?.replace(/\?.*$/u, '?…'))
    expect(paths).toEqual(['/v1/client/capabilities', `/v1/client/agents/${encodeURIComponent(agentRef)}`, '/v1/client/usage?…',
      '/v1/client/capabilities', `/v1/client/agents/${encodeURIComponent(agentRef)}`, '/v1/client/usage?…'])
    expect(setup.gatewayReads.every((read) => read.authorization === publisherAuthorization && read.cookie === undefined)).toBe(true)
    expect(setup.forwarded).toEqual([])
  })

  it('reads the requested 7-day window and refuses an answer for another window', async () => {
    const setup = await fixture()
    const response = await fetch(setup.base + detailUrl('7d'))
    expect(response.status).toBe(200)
    expect((await response.json() as { window: string }).window).toBe('7d')
    expect(setup.producerReads.map((read) => read.path)).toEqual([producerPath('7d')])
    setup.state.echoWindow = false
    setup.state.body = JSON.stringify(produced('7d'))
    await expectFailure(await fetch(setup.base + detailUrl('1d')), 502, 'root-detail-incompatible')
  })

  it('serves a verified root from cache for 30 s, re-verifying on every request', async () => {
    const setup = await fixture()
    expect((await fetch(setup.base + detailUrl())).status).toBe(200)
    setup.state.body = JSON.stringify({ ...produced(), usd: 1 })
    expect((await (await fetch(setup.base + detailUrl())).json() as { inclusiveUsd: number }).inclusiveUsd).toBe(543.21)
    expect(setup.producerReads).toHaveLength(1)
    expect(setup.gatewayReads.filter((read) => read.path === '/v1/client/capabilities')).toHaveLength(4)
    setup.state.agent = { ...setup.state.agent, incarnation_id: 'new-incarnation' }
    await expectFailure(await fetch(setup.base + detailUrl()), 422, 'root-detail-not-attributed')
    expect(setup.producerReads).toHaveLength(1)
  })

  it('registers no route when unconfigured: the gateway answers', async () => {
    const setup = await fixture({ enabled: false })
    expect((await fetch(setup.base + detailUrl())).status).toBe(404)
    expect(setup.forwarded.map((request) => request.path)).toEqual([detailUrl()])
    expect(setup.producerReads).toEqual([])
  })

  it('never shadows neighbouring client routes', async () => {
    const setup = await fixture()
    for (const path of [rootDetailRoute + '/extra', rootDetailRoute + 'x?agent=a&window=1d']) await (await fetch(setup.base + path)).text()
    expect(setup.forwarded.map((request) => request.path)).toEqual([rootDetailRoute + '/extra', rootDetailRoute + 'x?agent=a&window=1d'])
    expect(setup.gatewayReads).toEqual([])
  })

  it('refuses other methods and any query but one agent and window=1d|7d, before any read', async () => {
    const setup = await fixture()
    for (const method of ['POST', 'PUT', 'DELETE']) await expectFailure(await fetch(setup.base + detailUrl(), { method }), 405, 'method-not-allowed')
    expect((await fetch(setup.base + detailUrl(), { method: 'HEAD' })).status).toBe(405)
    for (const query of ['', '?', `?agent=${agentRef}`, `?agent=${agentRef}&window=all`, `?agent=${agentRef}&window=30d`, `?root=${root}&window=1d`,
      `?agent=${agentRef}&window=1d&root=${root}`, `?agent=${agentRef}&window=1d&agent=agent/other`, `?agent=${agentRef}&window=1d&window=1d`,
      '?agent=agent%2F..%2Fescape&window=1d', `?agent=${'a'.repeat(513)}&window=1d`]) {
      await expectFailure(await fetch(setup.base + rootDetailRoute + query), 400, 'invalid-query')
    }
    expect(setup.gatewayReads).toEqual([])
    expect(setup.producerReads).toEqual([])
  })

  it('requires read.projections and the native reads to succeed with the caller credential', async () => {
    for (const grantStatus of [401, 403]) {
      const setup = await fixture()
      setup.state.grantStatus = grantStatus
      await expectFailure(await fetch(setup.base + detailUrl()), 403, 'forbidden')
      expect(setup.producerReads).toEqual([])
    }
    const setup = await fixture()
    setup.state.granted = false
    await expectFailure(await fetch(setup.base + detailUrl()), 403, 'forbidden')
    setup.state.granted = true
    setup.state.usageStatus = 403
    await expectFailure(await fetch(setup.base + detailUrl()), 403, 'forbidden')
    setup.state.usageStatus = 200
    setup.state.grantStatus = 503
    await expectFailure(await fetch(setup.base + detailUrl()), 502, 'root-detail-gateway-unavailable')
    expect(setup.producerReads).toEqual([])
  })

  it('refuses a grant revoked during the producer read', async () => {
    const setup = await fixture()
    setup.state.revokeOnProducerRead = true
    await expectFailure(await fetch(setup.base + detailUrl()), 403, 'forbidden')
    expect(setup.producerReads).toHaveLength(1)
  })

  it('uses the browser pairing credential, and the tag credential for tagged readers', async () => {
    const paired = await fixture({ pairing: true })
    await expectFailure(await fetch(paired.base + detailUrl()), 401, 'pairing-required')
    expect(paired.gatewayReads).toEqual([])
    expect((await fetch(paired.base + detailUrl(), { headers: { cookie: `fractal_device=${browserBearer}` } })).status).toBe(200)
    expect(new Set(paired.gatewayReads.map((read) => read.authorization))).toEqual(new Set([`Bearer ${browserBearer}`]))
    const tagged = await fixture({ admit: () => ({ kind: 'tag', value: 'tag:reader' }) })
    expect((await fetch(tagged.base + detailUrl())).status).toBe(200)
    expect(new Set(tagged.gatewayReads.map((read) => read.authorization))).toEqual(new Set([tagAuthorization]))
    const refused = await fixture({ admit: () => false })
    expect((await fetch(refused.base + detailUrl())).status).toBe(403)
    expect(refused.gatewayReads).toEqual([])
  })
})

describe('root binding', () => {
  it('reports other harnesses as not covered without a producer read', async () => {
    for (const driver of ['claude', 'codex']) {
      const setup = await fixture()
      setup.state.agent = { ...setup.state.agent, driver }
      await expectFailure(await fetch(setup.base + detailUrl()), 422, 'root-detail-not-covered')
      expect(setup.producerReads).toEqual([])
    }
  })

  it('refuses a missing, stale, partial, conflicting or uncorroborated root without a producer read', async () => {
    const todo = agent().todo
    if (todo === undefined || todo === null) throw new TypeError('Missing fixture todo')
    const mutations: readonly ((state: FixtureState) => void)[] = [
      (state) => { state.agent = { ...state.agent, todo: undefined } },
      (state) => { state.agent = { ...state.agent, todo: { ...todo, stale: true } } },
      (state) => { state.agent = { ...state.agent, incarnation_id: 'new-incarnation' } },
      (state) => { state.agent = { ...state.agent, host_id: undefined } },
      (state) => {
        state.agent = { ...state.agent, todo: { ...todo, snapshot: { ...todo.snapshot, session_id: '8cf4a5a13423' } } }
        state.usage = state.usage.map((row) => ({ ...row, native_session_id: '8cf4a5a13423' }))
      },
      (state) => {
        state.agent = { ...state.agent, suspension: suspension(otherRoot) }
        state.usage = [...state.usage, { ...usage(), native_session_id: otherRoot }]
      },
      (state) => { state.usage = state.usage.map((row) => ({ ...row, native_session_id: undefined })) },
      (state) => { state.usage = state.usage.map((row) => ({ ...row, native_session_id: otherRoot })) },
      (state) => { state.usage = state.usage.map((row) => ({ ...row, agent: 'agent/unrelated' })) },
    ]
    for (const mutate of mutations) {
      const setup = await fixture()
      mutate(setup.state)
      await expectFailure(await fetch(setup.base + detailUrl()), 422, 'root-detail-not-attributed')
      expect(setup.producerReads).toEqual([])
    }
  })

  it('accepts a completed suspension binding and ignores historical roots in the usage period', async () => {
    const setup = await fixture()
    setup.state.agent = { ...setup.state.agent, todo: undefined, suspension: suspension(root) }
    setup.state.usage = [...setup.state.usage, { ...usage(), native_session_id: otherRoot }, { ...usage(), native_session_id: undefined }]
    const response = await fetch(setup.base + detailUrl())
    expect(response.status).toBe(200)
    expect((await response.json() as { nativeSessionId: string }).nativeSessionId).toBe(root)
    expect(setup.producerReads.map((read) => read.path)).toEqual([producerPath()])
  })
})

describe('producer answers', () => {
  it.each(['usd', 'nested', 'missing'] as const)('names the producer lens and failing %s field without response values', async kind => {
    const setup = await fixture()
    const detail = produced()
    if (kind === 'usd') Reflect.set(detail, 'usd', { private: agentRef, amount: 9876.54 })
    else if (kind === 'nested') Reflect.set(detail.children[0]!, 'usd', { private: root })
    else Reflect.deleteProperty(detail, 'root_session_id')
    setup.state.body = JSON.stringify(detail)
    const response = await fetch(setup.base + detailUrl())
    expect(response.status).toBe(502)
    const body = await response.json()
    expect(body.code).toBe('root-detail-incompatible')
    expect(body.message).toContain('producer lens; lens_version=15;')
    expect(body.message).toContain(kind === 'nested' ? '$.children[*].usd: expected' : kind === 'missing' ? '$.root_session_id: expected string' : '$.usd: expected')
    expect(body.message).toContain(kind === 'missing' ? 'string' : 'number')
    for (const value of [agentRef, nativeOwner, root, detail.host, '9876.54', 'private']) expect(body.message).not.toContain(value)
  })

  it('refuses incompatible answers with explicit codes and caches none of them', async () => {
    const cases: readonly (readonly [string, string])[] = [
      [`not JSON ${JSON.stringify(produced())}`, 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), root_session_id: otherRoot }), 'root-detail-wrong-root'],
      [JSON.stringify({ ...produced(), host: 'host/wrong-owner' }), 'root-detail-wrong-host'],
      [JSON.stringify({ ...produced(), usd: -1 }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), usd: undefined }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), coverage: { account_share: 2 } }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), child_count: 3 }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), children: [...produced().children, { session_id: root, usd: 1, requests: 1 }], child_count: 26 }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), children: Array.from({ length: 201 }, (_, i) => ({ session_id: `child-${i}`, usd: 1, requests: 1 })), child_count: 201 }), 'root-detail-incompatible'],
      [JSON.stringify({ ...produced(), padding: 'x'.repeat(300 * 1024) }), 'root-detail-oversized'],
    ]
    for (const [body, code] of cases) {
      const setup = await fixture()
      setup.state.body = body
      const refusal = await expectFailure(await fetch(setup.base + detailUrl()), 502, code)
      if (body.startsWith('not JSON')) expect(refusal).toEqual({
        code: 'root-detail-incompatible',
        message: 'producer lens; lens_version=unavailable; $: expected JSON object',
      })
      setup.state.body = JSON.stringify(produced())
      expect((await fetch(setup.base + detailUrl())).status, code).toBe(200)
      expect(setup.producerReads).toHaveLength(2)
    }
  })

  it('keeps the inclusive total and full count when only the top 20 descendants are listed', async () => {
    const setup = await fixture()
    setup.state.body = JSON.stringify({ ...produced(), child_count: 66, children: produced().children.slice(0, 20) })
    const body = await (await fetch(setup.base + detailUrl())).json() as { childCount: number; children: readonly unknown[]; inclusiveUsd: number }
    expect(body).toMatchObject({ childCount: 66, inclusiveUsd: 543.21 })
    expect(body.children).toHaveLength(20)
  })

  it('turns a no-request guidance answer into not-found, never a zero', async () => {
    const setup = await fixture()
    setup.state.body = JSON.stringify({ key: 'root_detail', root_session_id: root, window: '1d', message: 'Selected root has no requests in this window.', children: [] })
    await expectFailure(await fetch(setup.base + detailUrl()), 404, 'root-detail-not-found')
  })

  it('reports producer failures as 502 and an unreachable producer as unavailable', async () => {
    const setup = await fixture()
    setup.state.producerStatus = 503
    await expectFailure(await fetch(setup.base + detailUrl()), 502, 'root-detail-producer-unavailable')
    await close(setup.producer)
    await expectFailure(await fetch(setup.base + detailUrl()), 502, 'root-detail-producer-unavailable')
  })

  it('ends a producer read at its elapsed deadline even while bytes keep arriving', async () => {
    const up = await upstreams()
    const base = await directRelay(up, 200)
    up.state.producerMode = 'trickle'
    const started = performance.now()
    await expectFailure(await fetch(base + detailUrl()), 502, 'root-detail-deadline')
    expect(performance.now() - started).toBeLessThan(2000)
    up.state.producerMode = 'answer'
    expect((await fetch(base + detailUrl())).status).toBe(200)
    expect(up.producerReads).toHaveLength(2)
  })
})

describe('Vite gateway registration', () => {
  it('registers the relay only when WF_ROOT_DETAIL_URL is set', { timeout: 30_000 }, async () => {
    const up = await upstreams()
    const credentialFile = join(up.dir, 'credential')
    await writeFile(credentialFile, `${publisherAuthorization}\n`, { mode: 0o600 })
    const start = async (values: Record<string, string | undefined>) => {
      const previous = Object.fromEntries(Object.keys(values).map((key) => [key, process.env[key]]))
      cleanups.push(async () => {
        for (const [key, value] of Object.entries(previous)) {
          if (value === undefined) delete process.env[key]
          else process.env[key] = value
        }
      })
      for (const [key, value] of Object.entries(values)) {
        if (value === undefined) delete process.env[key]
        else process.env[key] = value
      }
      const vite = await createViteServer({ configFile: false, root: up.dir, logLevel: 'silent', plugins: [webfractalGateway()],
        server: { host: '127.0.0.1', port: 0, strictPort: true, hmr: false, ws: false, watch: null } })
      cleanups.push(() => vite.close())
      await vite.listen()
      if (vite.httpServer === null) throw new TypeError('Missing Vite listener')
      return `http://127.0.0.1:${portOf(vite.httpServer as Server)}`
    }
    const gateway = { WF_ST_GATEWAY: `unix:${up.socketPath}`, WF_ST_CREDENTIAL_FILE: credentialFile, WF_USAGE_HISTORY_URL: undefined }
    const off = await start({ ...gateway, WF_ROOT_DETAIL_URL: undefined })
    expect((await fetch(off + detailUrl())).status).toBe(404)
    expect(up.forwarded.map((request) => request.path)).toEqual([detailUrl()])
    const on = await start({ ...gateway, WF_ROOT_DETAIL_URL: up.producerUrl })
    expect((await fetch(on + detailUrl())).status).toBe(200)
    expect(up.producerReads.map((read) => read.path)).toEqual([producerPath()])
  })
})
