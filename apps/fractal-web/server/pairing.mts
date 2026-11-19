import { createHash, randomBytes } from 'node:crypto'
import { request } from 'node:http'
import type { IncomingMessage, ServerResponse } from 'node:http'
import type { GatewayOptions } from './gateway.mts'
import { browserBearer, cookieValue, deviceCookie, deviceLifetime, pairCookie, secureCookie } from './browserCookie.mts'

export interface BrowserPairingOptions {
  readonly admit: (request: IncomingMessage) => boolean
}
const prefix = '/_fractal/pair/'
export const pairingRoute = (url: string): boolean => url.split('?')[0]?.startsWith(prefix) === true
const json = (res: ServerResponse, status: number, body: unknown): void => {
  res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' })
  res.end(JSON.stringify(body))
}
const daemonCall = (gateway: GatewayOptions, path: string, body: unknown, authorization = gateway.authorization, method: 'GET' | 'POST' = 'POST'): Promise<{ status: number; body: unknown }> => {
    const { promise, resolve, reject } = Promise.withResolvers<{ status: number; body: unknown }>()
    const payload = method === 'GET' ? undefined : JSON.stringify(body)
    const upstream = request({ socketPath: gateway.socketPath, host: gateway.host, path, method, headers: {
      host: gateway.host, authorization, ...(payload === undefined ? {} : { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) }),
    }, agent: false }, response => {
      const chunks: Buffer[] = []
      let size = 0
      response.on('data', (chunk: Buffer) => {
        size += chunk.length
        if (size > 64 * 1024) { response.destroy(); reject(new TypeError('Pairing response too large')); return }
        chunks.push(chunk)
      })
      response.once('error', reject)
      response.once('end', () => {
        const status = response.statusCode ?? 502
        if (status === 401 || status === 403) { resolve({ status, body: undefined }); return }
        try { resolve({ status, body: JSON.parse(Buffer.concat(chunks).toString('utf8')) }) }
        catch { reject(new TypeError('Invalid pairing response')) }
      })
    })
    upstream.once('error', reject)
    upstream.setTimeout(gateway.timeoutMs, () => upstream.destroy(new TypeError('Pairing timed out')))
    upstream.end(payload)
    return promise
}
const record = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value)
const readBody = async (req: IncomingMessage): Promise<unknown> => {
  const chunks: Buffer[] = []
  let size = 0
  for await (const chunk of req) {
    size += Buffer.byteLength(chunk)
    if (size > 4096) throw new TypeError('Pairing request too large')
    chunks.push(Buffer.from(chunk))
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'))
}
interface PendingPair { requestId: string; requestKey: string; displayCode: string }
const pendingPair = (req: IncomingMessage): PendingPair | undefined => {
  try {
    const value: unknown = JSON.parse(Buffer.from(cookieValue(req.headers, pairCookie) ?? '', 'base64url').toString('utf8'))
    if (record(value) && typeof value.requestId === 'string' && value.requestId.length <= 200 &&
      typeof value.requestKey === 'string' && /^[a-f0-9]{64}$/.test(value.requestKey) && typeof value.displayCode === 'string')
      return { requestId: value.requestId, requestKey: value.requestKey, displayCode: value.displayCode }
  } catch { /* Missing or malformed cookies are unpaired, never authority. */ }
  return undefined
}

export const handlePairing = async (req: IncomingMessage, res: ServerResponse, gateway: GatewayOptions, options: BrowserPairingOptions | undefined): Promise<void> => {
  if (options === undefined) { json(res, 404, { error: 'Browser pairing is unavailable' }); return }
  // Pairing never inherits an edge or development admission without a personal tailnet login.
  if (!options.admit(req)) { json(res, 403, { error: 'Forbidden' }); return }
  const path = req.url?.split('?')[0]
  const action = path?.slice(prefix.length)
  if ((action === 'status' ? 'GET' : 'POST') !== req.method || !['start', 'status', 'forget', 'redeem'].includes(action ?? '')) {
    json(res, 404, { error: 'Not found' }); return
  }
  // Pairing cookies are HTTPS-only, including when TLS terminates at the admitted proxy.
  const site = req.headers['sec-fetch-site']
  if (req.method === 'POST' && (site !== undefined ? site !== 'same-origin'
    : req.headers.origin === undefined || req.headers.origin !== `https://${req.headers.host}`)) {
    json(res, 403, { error: 'Forbidden' }); return
  }
  try {
    if (action === 'forget') {
      const bearer = browserBearer(req.headers)
      if (bearer !== undefined) {
        const result = await daemonCall(gateway, '/v1/client/pairing/browser-requests/revoke-current', {}, `Bearer ${bearer}`)
        if (result.status < 200 || result.status >= 300) {
          if (result.status !== 401 && result.status !== 403) { json(res, 502, { error: 'Could not revoke this browser' }); return }
        }
      }
      res.setHeader('set-cookie', [secureCookie(deviceCookie, '', 0), secureCookie(pairCookie, '', 0)])
      json(res, 200, { status: 'unpaired' }); return
    }
    if (action === 'start') {
      const body = await readBody(req)
      if (!record(body) || typeof body.deviceLabel !== 'string' || body.deviceLabel.trim().length === 0 || body.deviceLabel.length > 120) {
        json(res, 400, { error: 'A browser label is required' }); return
      }
      const requestKey = randomBytes(32).toString('hex')
      const result = await daemonCall(gateway, '/v1/client/pairing/browser-requests', {
        deviceLabel: body.deviceLabel.trim(), tailnetLogin: req.headers['tailscale-user-login'],
        requestKeyDigest: createHash('sha256').update(requestKey).digest('hex'),
      })
      const response = result.body
      if (result.status !== 200 && result.status !== 201) { json(res, result.status === 429 ? 429 : 502, { error: 'Could not request browser pairing' }); return }
      if (!record(response) || typeof response.requestId !== 'string' || typeof response.displayCode !== 'string' || response.expiresAt === undefined) {
        json(res, 502, { error: 'Invalid pairing response' }); return
      }
      const pending = { requestId: response.requestId, requestKey, displayCode: response.displayCode }
      res.setHeader('set-cookie', secureCookie(pairCookie, Buffer.from(JSON.stringify(pending)).toString('base64url'), 300))
      json(res, 200, { requestId: response.requestId, displayCode: response.displayCode, expiresAt: response.expiresAt }); return
    }
    const bearer = browserBearer(req.headers)
    if (bearer !== undefined) {
      const grant = await daemonCall(gateway, '/v1/client/capabilities', undefined, `Bearer ${bearer}`, 'GET')
      if (grant.status === 401 || grant.status === 403) {
        if (action === 'redeem') {
          res.setHeader('set-cookie', secureCookie(deviceCookie, '', 0))
          json(res, 200, { status: 'unpaired', credentialChanged: true }); return
        }
        json(res, 200, { status: 'unpaired', credentialRejected: true }); return
      }
      if (grant.status !== 200) { json(res, 502, { error: 'Could not validate this browser' }); return }
      json(res, 200, { status: 'paired' }); return
    }
    const pending = pendingPair(req)
    if (pending === undefined) { json(res, 200, { status: 'unpaired' }); return }
    // The daemon exposes no read-only browser-request status endpoint.
    if (action === 'status') { json(res, 200, { status: 'pending', displayCode: pending.displayCode }); return }
    const result = await daemonCall(gateway, `/v1/client/pairing/browser-requests/${encodeURIComponent(pending.requestId)}/redeem`, { requestKey: pending.requestKey })
    const response = result.body
    if (result.status !== 200 || !record(response)) { json(res, 502, { error: 'Could not check browser pairing' }); return }
    if (typeof response.bearer === 'string' && /^[A-Za-z0-9_-]{43}$/.test(response.bearer)) {
      res.setHeader('set-cookie', [secureCookie(deviceCookie, response.bearer, deviceLifetime), secureCookie(pairCookie, '', 0)])
      json(res, 200, { status: 'paired', credentialChanged: true }); return
    }
    if (response.status === 'pending') { json(res, 200, { status: 'pending', displayCode: pending.displayCode }); return }
    if (response.status === 'denied' || response.status === 'expired') {
      res.setHeader('set-cookie', secureCookie(pairCookie, '', 0))
      json(res, 200, { status: response.status }); return
    }
    json(res, 502, { error: 'Invalid pairing response' })
  } catch {
    // Never include daemon responses, cookies, request keys or credentials in errors/logs.
    json(res, 502, { error: 'Browser pairing unavailable' })
  }
}
