import { afterEach, expect, it } from 'vitest'
import { createHash } from 'node:crypto'
import { createServer, request } from 'node:http'
import type { Server } from 'node:http'
import { once } from 'node:events'
import { mkdtemp, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { handlePairing } from './pairing.mts'
import { tailnetAdmission } from './host.mts'

const cleanups: (() => Promise<void>)[] = []
afterEach(async () => { while (cleanups.length) await cleanups.pop()?.() })
const close = async (server: Server): Promise<void> => {
  const { promise, resolve } = Promise.withResolvers<void>()
  server.closeAllConnections()
  server.close(() => resolve())
  await promise
}
const origin = 'https://personal.example.ts.net'
const login = 'owner@example.com'
const publisherAuth = `Bearer ${'r'.repeat(43)}`
const device = 'd'.repeat(43)
const flags = '; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age='
const fixture = async (admitted = false) => {
  const root = await mkdtemp(join(tmpdir(), 'fractal-pairing-test-'))
  cleanups.push(() => rm(root, { recursive: true, force: true }))
  const received: { path: string; auth: string | undefined; body: Record<string, unknown>; cookie: string | undefined }[] = []
  let status = 'pending'
  let grantStatus = 200
  let malformedEnvelope = false
  const envelope = (value: unknown) => JSON.stringify(malformedEnvelope ? value : {
    api_version: 'st3.client.v0', request_id: 'request/test',
    snapshot: { id: 'snapshot/test', created_at: '2026-11-08T00:00:00Z', host_id: 'host/test',
      projection_version: 'client-projection.v0', store_index: 1 },
    value,
  })
  const daemon = createServer((req, res) => { void (async () => {
    const chunks: Buffer[] = []
    for await (const chunk of req) chunks.push(Buffer.from(chunk))
    received.push({ path: req.url ?? '', auth: req.headers.authorization, body: JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}'), cookie: req.headers.cookie })
    res.setHeader('content-type', 'application/json')
    if (req.url === '/v1/client/capabilities') {
      expect(req.method).toBe('GET')
      res.statusCode = grantStatus
      res.end(grantStatus === 200 ? envelope({ capabilities: [] }) : '')
      return
    }
    res.end(envelope(req.url?.endsWith('/redeem') ? status === 'approved' ? { bearer: device, scopes: ['control.messages'], expiresAt: 123 } : { status }
      : req.url?.endsWith('/revoke-current') ? {} : { requestId: 'request-one', displayCode: '123456', expiresAt: 123 }))
  })().catch(() => res.destroy()) })
  const socketPath = join(root, 'daemon.sock')
  daemon.listen(socketPath)
  await once(daemon, 'listening')
  cleanups.push(() => close(daemon))
  let listening: string | undefined
  const admit = tailnetAdmission({ origin, logins: [login], listener: () => listening })
  const server = createServer((req, res) => { void handlePairing(req, res, { socketPath, host: 'localhost', authorization: publisherAuth, timeoutMs: 1000 }, { admit: admitted ? () => true : admit }).catch(() => res.destroy()) })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  cleanups.push(() => close(server))
  const address = server.address()
  if (address === null || typeof address === 'string') throw new TypeError('Missing listener')
  listening = `127.0.0.1:${address.port}`
  const headers = { host: new URL(origin).host, origin, 'tailscale-user-login': login }
  // Use the HTTP transport so Host is preserved. Node fetch may replace it with the
  // loopback URL's host, which is not the HTTPS origin published by the proxy.
  const call = (action: string, cookie?: string, overrides: Record<string, string | undefined> = {}): Promise<Response> => {
    const { promise, resolve, reject } = Promise.withResolvers<Response>()
    const upstream = request({
      hostname: '127.0.0.1', port: address.port, path: `/_fractal/pair/${action}`,
      method: action === 'status' ? 'GET' : 'POST',
      headers: Object.fromEntries(Object.entries({ ...headers, ...(cookie === undefined ? {} : { cookie }), ...overrides }).filter((entry): entry is [string, string] => entry[1] !== undefined)),
    }, response => {
      const chunks: Buffer[] = []
      response.on('data', (chunk: Buffer) => chunks.push(chunk))
      response.once('error', reject)
      response.once('end', () => {
        const responseHeaders = new Headers()
        for (const [name, values] of Object.entries(response.headersDistinct)) {
          for (const value of values ?? []) responseHeaders.append(name, value)
        }
        resolve(new Response(Buffer.concat(chunks), { status: response.statusCode, headers: responseHeaders }))
      })
    })
    upstream.once('error', reject)
    upstream.end(action === 'start' ? JSON.stringify({ deviceLabel: 'Personal browser' }) : undefined)
    return promise
  }
  return { call, received, setStatus: (value: string) => { status = value }, setGrantStatus: (value: number) => { grantStatus = value }, rejectEnvelope: () => { malformedEnvelope = true } }
}

it('keeps request secrets and grants only in secure cookies and revokes before forgetting', async () => {
  const setup = await fixture()
  const start = await setup.call('start')
  expect(start.status).toBe(200)
  expect(await start.json()).toEqual({ requestId: 'request-one', displayCode: '123456', expiresAt: 123 })
  const pair = start.headers.getSetCookie()[0]!
  expect(pair).toMatch(/^fractal_pair=[A-Za-z0-9_-]+; HttpOnly; Secure; SameSite=Strict; Path=\/; Max-Age=300$/)
  const pairHeader = pair.split(';')[0]!
  const pending: { requestKey: string } = JSON.parse(Buffer.from(pairHeader.slice('fractal_pair='.length), 'base64url').toString('utf8'))
  expect(pending.requestKey).toMatch(/^[a-f0-9]{64}$/)
  expect(setup.received[0]).toEqual({ path: '/v1/client/pairing/browser-requests', auth: publisherAuth, cookie: undefined, body: {
    deviceLabel: 'Personal browser', tailnetLogin: login, requestKeyDigest: createHash('sha256').update(pending.requestKey).digest('hex'),
  } })
  const status = await setup.call('status', pairHeader)
  expect(await status.json()).toEqual({ status: 'pending', displayCode: '123456' })
  expect(status.headers.getSetCookie()).toEqual([])
  expect(setup.received).toHaveLength(1)
  expect(await (await setup.call('redeem', pairHeader)).json()).toEqual({ status: 'pending', displayCode: '123456' })
  setup.setStatus('approved')
  const approved = await setup.call('redeem', pairHeader)
  expect(await approved.json()).toEqual({ status: 'paired', credentialChanged: true })
  expect(approved.headers.getSetCookie()).toEqual([`fractal_device=${device}${flags}2592000`, `fractal_pair=${flags}0`])
  expect(setup.received[2]?.auth).toBe(publisherAuth)
  expect(setup.received[2]?.body).toEqual({ requestKey: pending.requestKey })
  const paired = await setup.call('status', `fractal_device=${device}`)
  expect(await paired.json()).toEqual({ status: 'paired' })
  expect(paired.headers.getSetCookie()).toEqual([])
  const forgotten = await setup.call('forget', `fractal_device=${device}; other=never-forward`)
  expect(await forgotten.json()).toEqual({ status: 'unpaired' })
  expect(forgotten.headers.getSetCookie()).toEqual([`fractal_device=${flags}0`, `fractal_pair=${flags}0`])
  expect(setup.received[4]).toEqual({ path: '/v1/client/pairing/browser-requests/revoke-current', auth: `Bearer ${device}`, body: {}, cookie: undefined })
})
it.each(['denied', 'expired'])('clears a %s pairing cookie', async status => {
  const setup = await fixture()
  const start = await setup.call('start')
  const cookie = start.headers.getSetCookie()[0]!.split(';')[0]!
  setup.setStatus(status)
  const read = await setup.call('status', cookie)
  expect(await read.json()).toEqual({ status: 'pending', displayCode: '123456' })
  expect(read.headers.getSetCookie()).toEqual([])
  expect(setup.received).toHaveLength(1)
  const result = await setup.call('redeem', cookie)
  expect(await result.json()).toEqual({ status })
  expect(result.headers.getSetCookie()).toEqual([`fractal_pair=${flags}0`])
})
it.each(['start', 'status', 'forget', 'redeem'])('refuses %s without admitted personal login or with cross-origin requests', async action => {
  const setup = await fixture()
  for (const overrides of [{ 'tailscale-user-login': '' }, { 'tailscale-user-login': 'tag:dev-host' }, { origin: 'https://other.example' }]) {
    expect((await setup.call(action, undefined, overrides)).status).toBe(403)
  }
  expect(setup.received).toEqual([])
})
it('reports unpaired without a cookie and clears both cookies when already forgotten', async () => {
  const setup = await fixture()
  expect(await (await setup.call('status')).json()).toEqual({ status: 'unpaired' })
  const forgotten = await setup.call('forget')
  expect(forgotten.headers.getSetCookie()).toEqual([`fractal_device=${flags}0`, `fractal_pair=${flags}0`])
  expect(setup.received).toEqual([])
})

it('validates a device cookie with the daemon before reporting paired', async () => {
  const setup = await fixture()
  const response = await setup.call('status', `fractal_device=${device}; other=private`)
  expect(await response.json()).toEqual({ status: 'paired' })
  expect(response.headers.getSetCookie()).toEqual([])
  expect(setup.received).toEqual([{ path: '/v1/client/capabilities', auth: `Bearer ${device}`, cookie: undefined, body: {} }])
})
it.each([401, 403])('clears a rejected device cookie only through POST when authentication returns %s', async status => {
  const setup = await fixture()
  setup.setGrantStatus(status)
  const response = await setup.call('status', `fractal_device=${device}`)
  expect(await response.json()).toEqual({ status: 'unpaired', credentialRejected: true })
  expect(response.headers.getSetCookie()).toEqual([])
  const redeemed = await setup.call('redeem', `fractal_device=${device}`)
  expect(await redeemed.json()).toEqual({ status: 'unpaired', credentialChanged: true })
  expect(redeemed.headers.getSetCookie()).toEqual([`fractal_device=${flags}0`])
  expect(setup.received.every(call => call.path === '/v1/client/capabilities' && call.auth === `Bearer ${device}`)).toBe(true)
})

it.each(['start', 'forget', 'redeem'])('enforces same-origin %s independently of admission', async action => {
  const setup = await fixture(true)
  for (const overrides of [
    { origin: undefined },
    { origin: 'https://other.example' },
    { origin: 'http://personal.example.ts.net' },
    { 'sec-fetch-site': 'cross-site' },
    { 'sec-fetch-site': 'same-site' },
    { 'sec-fetch-site': 'none' },
  ]) {
    const response = await setup.call(action, undefined, overrides)
    expect(response.status).toBe(403)
    expect(response.headers.getSetCookie()).toEqual([])
  }
  expect(setup.received).toEqual([])
  expect((await setup.call(action, undefined, { origin: undefined, 'sec-fetch-site': 'same-origin' })).status).toBe(200)
})

it.each([undefined, 'fractal_pair=malformed', 'fractal_device=malformed'])('keeps unpaired GET read-only for %s', async cookie => {
  const setup = await fixture()
  const response = await setup.call('status', cookie)
  expect(await response.json()).toEqual({ status: 'unpaired' })
  expect(response.headers.getSetCookie()).toEqual([])
  expect(setup.received).toEqual([])
})

it('keeps a failed capability GET read-only', async () => {
  const setup = await fixture()
  setup.setGrantStatus(500)
  const response = await setup.call('status', `fractal_device=${device}`)
  expect(response.status).toBe(502)
  expect(response.headers.getSetCookie()).toEqual([])
  expect(setup.received.map(call => call.path)).toEqual(['/v1/client/capabilities'])
})

it.each(['pending', 'approved', 'denied', 'expired', 'invalid'])('never redeems a pending cookie during GET even when the daemon outcome is %s', async status => {
  const setup = await fixture()
  const start = await setup.call('start')
  const cookie = start.headers.getSetCookie()[0]!.split(';')[0]!
  setup.setStatus(status)
  const response = await setup.call('status', cookie)
  expect(await response.json()).toEqual({ status: 'pending', displayCode: '123456' })
  expect(response.headers.getSetCookie()).toEqual([])
  expect(setup.received.map(call => call.path)).toEqual(['/v1/client/pairing/browser-requests'])
})


it('does not set a browser cookie from a response missing the daemon client envelope', async () => {
  const setup = await fixture()
  setup.rejectEnvelope()
  const response = await setup.call('start')
  expect(response.status).toBe(502)
  expect(response.headers.getSetCookie()).toEqual([])
})
