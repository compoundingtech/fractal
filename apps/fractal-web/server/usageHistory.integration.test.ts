import { afterEach, describe, expect, it } from 'vitest'
import { once } from 'node:events'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import type { IncomingMessage, Server } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Tracer } from 'effect'
import { createServer as createViteServer } from 'vite'
import { createFractalWebServer } from './core.mts'
import type { MiddlewareOptions } from './core.mts'
import { usageHistoryPath, usageHistoryProducer, usageHistoryRoute } from './usageHistory.mts'
import { webfractalGateway } from '../scripts/vite-gateway.mjs'

// Assembled so the source itself never carries a literal bearer credential pair.
const publisherAuthorization = 'Bearer' + ' fx-publisher'
const tagAuthorization = 'Bearer' + ' fx-tag-reader'
const browserBearer = 'b'.repeat(43)
const accountsBody = '{"schemaVersion":3,"accounts":[{"id":"harness/alpha","ledgerAccountId":"vendor/alpha desk"},{"id":"harness/beta","ledgerAccountId":"vendor/beta"}]}'
const historyBody = '{"lens":"usage_over_time","rows":[],"fixture":"history"}'
const capabilities = (granted: boolean) => JSON.stringify({ api_version: 'fractal.client.v0', value: { kind: 'capabilities', capabilities: [
  { id: 'read.projections', state: granted ? 'granted' : 'ungranted' }, { id: 'read.arrangements', state: 'granted' },
] } })
const historyUrl = (account: string) => `${usageHistoryRoute}?${new URLSearchParams({ account })}`

interface Received { readonly path: string | undefined; readonly authorization: string | undefined; readonly cookie: string | undefined; readonly origin: string | undefined }
const received = (req: IncomingMessage): Received => ({ path: req.url, authorization: req.headers.authorization, cookie: req.headers.cookie, origin: req.headers.origin })

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

/** A gateway socket answering capabilities and forwarding nothing else, plus a loopback producer. */
const upstreams = async () => {
  const root = await mkdtemp(join(tmpdir(), 'usage-history-test-'))
  cleanups.push(() => rm(root, { recursive: true, force: true }))
  const state = { grantStatus: 200, granted: true, quotaStatus: 200, quotaBody: accountsBody, historyStatus: 200, historyBody }
  const grants: Received[] = []
  const forwarded: Received[] = []
  const producerReads: Received[] = []
  const gateway = createServer((req, res) => {
    if (req.url === '/v1/client/capabilities') {
      grants.push(received(req))
      res.writeHead(state.grantStatus, { 'content-type': 'application/json' })
      res.end(capabilities(state.granted))
      return
    }
    forwarded.push(received(req))
    res.writeHead(404, { 'content-type': 'application/json' })
    res.end('{"error":"gateway-fixture"}')
  })
  const socketPath = join(root, 'gateway.sock')
  await listen(gateway, socketPath)
  cleanups.push(() => close(gateway))
  const producer = createServer((req, res) => {
    producerReads.push(received(req))
    const history = req.url?.startsWith('/api/v1/lens/') === true
    res.writeHead(history ? state.historyStatus : state.quotaStatus, { 'content-type': 'application/json' })
    res.end(history ? state.historyBody : state.quotaBody)
  })
  await listen(producer)
  cleanups.push(() => close(producer))
  return { root, state, grants, forwarded, producerReads, socketPath, producer, producerUrl: `http://127.0.0.1:${portOf(producer)}` }
}

const fixture = async ({ enabled = true, admit = () => true, pairing = false }: {
  readonly enabled?: boolean
  readonly admit?: MiddlewareOptions['admit']
  readonly pairing?: boolean
} = {}) => {
  const up = await upstreams()
  await writeFile(join(up.root, 'index.html'), '<html><head></head><body>fixture</body></html>')
  const app = createFractalWebServer({ dist: up.root, host: '127.0.0.1', port: 0, admit,
    tracer: Tracer.make({ span: (options) => new Tracer.NativeSpan(options) }),
    ...(pairing ? { pairing: { admit: () => true } } : {}),
    gateway: { socketPath: up.socketPath, host: 'paired-gateway', authorization: publisherAuthorization, tagAuthorization, timeoutMs: 1000 },
    identity: { deploymentId: 'fixture-build', buildIdentity: { baseVersion: '1.0.0', displayVersion: '1.0.0-test', machineVersion: '1.0.0+test', sourceKind: 'local', dirty: false } },
    usageHistory: enabled ? usageHistoryProducer(up.producerUrl) : undefined,
  })
  app.listen()
  await once(app.server, 'listening')
  cleanups.push(() => app.close())
  return { ...up, base: `http://127.0.0.1:${portOf(app.server)}` }
}
const expectFailure = async (response: Response, status: number, code: string) => {
  expect(response.status).toBe(status)
  expect(response.headers.get('cache-control')).toBe('no-store')
  const body = await response.json() as { code: string; message: string; reason?: string }
  expect(body.code).toBe(code)
  expect(typeof body.message).toBe('string')
  return body
}

describe('usageHistoryProducer', () => {
  it('is off when unset and accepts only explicit loopback base URLs', () => {
    expect(usageHistoryProducer(undefined)).toBeUndefined()
    expect(usageHistoryProducer('  ')).toBeUndefined()
    expect(usageHistoryProducer('http://127.0.0.1:4100')).toEqual({ hostname: '127.0.0.1', port: 4100 })
    expect(usageHistoryProducer('http://127.0.0.1:4100/')).toEqual({ hostname: '127.0.0.1', port: 4100 })
    expect(usageHistoryProducer('http://[::1]:4100')).toEqual({ hostname: '::1', port: 4100 })
    for (const value of ['https://127.0.0.1:4100', 'http://localhost:4100', 'http://192.0.2.10:4100', 'http://hub.example.test:4100',
      'http://127.0.0.1', 'http://127.0.0.1:4100/api', 'http://127.0.0.1:4100/?x=1', 'http://user:pw@127.0.0.1:4100', 'not a url']) {
      expect(() => usageHistoryProducer(value), value).toThrow(/WF_USAGE_HISTORY_URL/)
    }
  })
})

describe('interim usage-history relay', () => {
  it('relays the producer day series for a published account, granting with the gateway credential', async () => {
    const setup = await fixture()
    const response = await fetch(setup.base + historyUrl('vendor/alpha desk'), { headers: { cookie: 'browser=fixture' } })
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toBe('application/json')
    expect(response.headers.get('cache-control')).toBe('no-store')
    expect(await response.text()).toBe(historyBody)
    const expected = '/api/v1/lens/usage_over_time?window=all&group_by=account&bucket=day&group=vendor%2Falpha+desk'
    expect(usageHistoryPath('vendor/alpha desk')).toBe(expected)
    expect(setup.grants).toEqual([{ path: '/v1/client/capabilities', authorization: publisherAuthorization, cookie: undefined, origin: undefined }])
    expect(setup.producerReads).toEqual([
      { path: '/api/v1/quota', authorization: undefined, cookie: undefined, origin: undefined },
      { path: expected, authorization: undefined, cookie: undefined, origin: undefined },
    ])
    expect(setup.forwarded).toEqual([])
    setup.state.historyBody = '{"changed":true}'
    expect(await (await fetch(setup.base + historyUrl('vendor/alpha desk'))).text()).toBe(historyBody)
    expect(await (await fetch(setup.base + historyUrl('vendor/beta'))).text()).toBe('{"changed":true}')
    expect(setup.producerReads.map((read) => read.path)).toEqual(['/api/v1/quota', expected, usageHistoryPath('vendor/beta')])
    expect(setup.grants).toHaveLength(3)
  })

  it('registers no route when unconfigured: the gateway answers history and quota', async () => {
    const setup = await fixture({ enabled: false })
    expect((await fetch(setup.base + historyUrl('vendor/beta'))).status).toBe(404)
    expect((await fetch(setup.base + '/v1/client/usage/quota')).status).toBe(404)
    expect(setup.forwarded.map((request) => request.path)).toEqual([historyUrl('vendor/beta'), '/v1/client/usage/quota'])
    expect(setup.producerReads).toEqual([])
  })

  it('never shadows the gateway quota route or other client routes', async () => {
    const setup = await fixture()
    for (const path of ['/v1/client/usage/quota', usageHistoryRoute + '/extra', usageHistoryRoute + 'x?account=vendor%2Fbeta']) {
      await (await fetch(setup.base + path)).text()
    }
    expect(setup.forwarded.map((request) => request.path)).toEqual(['/v1/client/usage/quota', usageHistoryRoute + '/extra', usageHistoryRoute + 'x?account=vendor%2Fbeta'])
    expect(setup.grants).toEqual([])
    expect(setup.producerReads).toEqual([])
  })

  it('fails closed for unpublished accounts and an empty allowlist without reading history', async () => {
    const setup = await fixture()
    for (const account of ['vendor/gamma', 'harness/alpha', 'vendor/alpha']) {
      await expectFailure(await fetch(setup.base + historyUrl(account)), 404, 'unknown-account')
    }
    expect(setup.producerReads.map((read) => read.path)).toEqual(['/api/v1/quota'])
    const empty = await fixture()
    empty.state.quotaBody = '{"schemaVersion":3,"accounts":[]}'
    await expectFailure(await fetch(empty.base + historyUrl('vendor/beta')), 404, 'unknown-account')
    const missing = await fixture()
    missing.state.quotaBody = '{"schemaVersion":3}'
    await expectFailure(await fetch(missing.base + historyUrl('vendor/beta')), 404, 'unknown-account')
    expect([...empty.producerReads, ...missing.producerReads].map((read) => read.path)).toEqual(['/api/v1/quota', '/api/v1/quota'])
  })

  it('fails closed when the quota read fails, and does not cache the failure', async () => {
    const setup = await fixture()
    setup.state.quotaStatus = 503
    expect((await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')).reason).toBe('quota HTTP 503')
    setup.state.quotaStatus = 200
    setup.state.quotaBody = 'not JSON'
    await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')
    expect(setup.producerReads.map((read) => read.path)).toEqual(['/api/v1/quota', '/api/v1/quota'])
    setup.state.quotaBody = accountsBody
    expect(await (await fetch(setup.base + historyUrl('vendor/beta'))).text()).toBe(historyBody)
    const down = await fixture()
    await close(down.producer)
    expect((await expectFailure(await fetch(down.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')).reason).toMatch(/ECONNREFUSED/)
  })

  it('returns explicit 502 for history producer failures and does not cache them', async () => {
    const setup = await fixture()
    setup.state.historyStatus = 503
    expect((await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')).reason).toBe('HTTP 503')
    setup.state.historyStatus = 200
    setup.state.historyBody = 'not JSON'
    await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')
    setup.state.historyBody = historyBody
    expect(await (await fetch(setup.base + historyUrl('vendor/beta'))).text()).toBe(historyBody)
  })

  it('requires a read.projections grant on every request, including cache hits', async () => {
    for (const grantStatus of [401, 403]) {
      const setup = await fixture()
      setup.state.grantStatus = grantStatus
      await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 403, 'forbidden')
      expect(setup.producerReads).toEqual([])
    }
    const setup = await fixture()
    setup.state.granted = false
    await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 403, 'forbidden')
    expect(setup.producerReads).toEqual([])
    setup.state.granted = true
    expect(await (await fetch(setup.base + historyUrl('vendor/beta'))).text()).toBe(historyBody)
    setup.state.granted = false
    await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 403, 'forbidden')
    setup.state.granted = true
    setup.state.grantStatus = 503
    expect((await expectFailure(await fetch(setup.base + historyUrl('vendor/beta')), 502, 'usage-unavailable')).reason).toBe('capabilities HTTP 503')
    expect(setup.producerReads).toHaveLength(2)
  })

  it('refuses refused admission, other methods and malformed queries before the grant', async () => {
    const refused = await fixture({ admit: () => false })
    expect((await fetch(refused.base + historyUrl('vendor/beta'))).status).toBe(403)
    expect(refused.grants).toEqual([])
    const setup = await fixture()
    for (const method of ['POST', 'PUT', 'DELETE']) await expectFailure(await fetch(setup.base + historyUrl('vendor/beta'), { method }), 405, 'method-not-allowed')
    const head = await fetch(setup.base + historyUrl('vendor/beta'), { method: 'HEAD' })
    expect(head.status).toBe(405)
    expect(await head.text()).toBe('')
    for (const query of ['', '?', '?account=', '?account=vendor%2Fbeta&account=vendor%2Fbeta', '?account=vendor%2Fbeta&bucket=hour', '?group=vendor%2Fbeta', `?account=${'a'.repeat(257)}`]) {
      await expectFailure(await fetch(setup.base + usageHistoryRoute + query), 400, 'invalid-query')
    }
    expect(setup.grants).toEqual([])
    expect(setup.producerReads).toEqual([])
  })

  it('grants with the browser pairing credential, and the tag credential for tagged readers', async () => {
    const paired = await fixture({ pairing: true })
    await expectFailure(await fetch(paired.base + historyUrl('vendor/beta')), 401, 'pairing-required')
    expect(paired.grants).toEqual([])
    expect((await fetch(paired.base + historyUrl('vendor/beta'), { headers: { cookie: `fractal_device=${browserBearer}` } })).status).toBe(200)
    expect(paired.grants.map((grant) => grant.authorization)).toEqual([`Bearer ${browserBearer}`])
    const tagged = await fixture({ admit: () => ({ kind: 'tag', value: 'tag:reader' }) })
    expect((await fetch(tagged.base + historyUrl('vendor/beta'))).status).toBe(200)
    expect(tagged.grants.map((grant) => grant.authorization)).toEqual([tagAuthorization])
  })
})

describe('Vite gateway registration', () => {
  it('registers the relay only when WF_USAGE_HISTORY_URL is set', { timeout: 30_000 }, async () => {
    const up = await upstreams()
    const credentialFile = join(up.root, 'credential')
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
      const vite = await createViteServer({ configFile: false, root: up.root, logLevel: 'silent', plugins: [webfractalGateway()],
        server: { host: '127.0.0.1', port: 0, strictPort: true, hmr: false, ws: false, watch: null } })
      cleanups.push(() => vite.close())
      await vite.listen()
      if (vite.httpServer === null) throw new TypeError('Missing Vite listener')
      return `http://127.0.0.1:${portOf(vite.httpServer as Server)}`
    }
    const gateway = { WF_ST_GATEWAY: `unix:${up.socketPath}`, WF_ST_CREDENTIAL_FILE: credentialFile }
    const off = await start({ ...gateway, WF_USAGE_HISTORY_URL: undefined })
    expect((await fetch(off + historyUrl('vendor/beta'))).status).toBe(404)
    expect(up.forwarded.map((request) => request.path)).toEqual([historyUrl('vendor/beta')])
    const on = await start({ ...gateway, WF_USAGE_HISTORY_URL: up.producerUrl })
    const response = await fetch(on + historyUrl('vendor/beta'))
    expect(response.status).toBe(200)
    expect(await response.text()).toBe(historyBody)
    expect(up.grants.map((grant) => grant.authorization)).toEqual([publisherAuthorization])
    expect(up.producerReads.map((read) => read.path)).toEqual(['/api/v1/quota', usageHistoryPath('vendor/beta')])
  })
})
