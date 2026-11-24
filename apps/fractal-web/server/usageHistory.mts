import { request } from 'node:http'
import type { IncomingMessage, ServerResponse } from 'node:http'
import { gatewayCredential, type GatewayOptions } from './gateway.mts'
import { readCapabilities, scopeGranted } from './tagAuthorization.mts'

/**
 * Interim usage-history relay for `src/monitor/source.ts`.
 *
 * The client gateway serves `GET /v1/client/usage/quota` but not history yet; this relay answers
 * only `GET /v1/client/usage/history?account=<ledgerAccountId>` until it does, and is then
 * deleted (https://github.com/compoundingtech/fractal/issues/7115). It is off unless the host
 * configures a loopback usage-hub base URL, and quota always stays on the gateway.
 *
 * Authorization reuses the gateway's: the request was admitted by the host, and the credential
 * the gateway would forward for it (`gatewayCredential`: tag, browser grant or publisher) must
 * read `GET /v1/client/capabilities` with `read.projections` granted, the scope quota requires.
 * That check runs on every request, before any producer read or cache lookup.
 *
 * The producer is read at two fixed paths only, without credentials: its quota answer, whose
 * `ledgerAccountId`s form the account allowlist, and the day series of one allowlisted account.
 * Each read must finish within 5 s of elapsed time. The allowlist is one shared,
 * credential-independent cache entry (30 s); history is cached per account (60 s, at most 64
 * entries and at most 64 distinct reads in flight, beyond which a request gets 503 without a
 * producer read). Failures are never cached. A failed quota read is a 502 and an account the
 * quota answer does not publish, including when it publishes none, is a 404: no history is read
 * without a successful allowlist that names the account.
 */
export const usageHistoryRoute = '/v1/client/usage/history'

/** The configured loopback producer the relay may read. */
export interface UsageHistoryProducer {
  readonly hostname: '127.0.0.1' | '::1'
  readonly port: number
}

/** Parse the optional producer base URL; unset or empty means the relay is off. */
export const usageHistoryProducer = (value: string | undefined, name = 'WF_USAGE_HISTORY_URL'): UsageHistoryProducer | undefined => {
  const text = value?.trim()
  if (!text) return undefined
  let url: URL
  try { url = new URL(text) } catch { throw new TypeError(`${name} must be a URL`) }
  const hostname = url.hostname === '[::1]' ? '::1' : url.hostname
  // URL drops an explicit default port (`:80`), so the port must be read from the raw authority.
  const explicitPort = /^[a-z][a-z0-9+.-]*:\/\/[^/?#]*:(\d+)(?:[/?#]|$)/i.exec(text)?.[1] !== undefined
  const port = url.port === '' ? 80 : Number(url.port)
  if (url.protocol !== 'http:' || (hostname !== '127.0.0.1' && hostname !== '::1') || !explicitPort || port < 1 || port > 65535 ||
      url.username !== '' || url.password !== '' || url.pathname !== '/' || url.search !== '' || url.hash !== '')
    throw new TypeError(`${name} must be a loopback base URL such as http://127.0.0.1:PORT, with a nonzero port and no credentials, path or query`)
  return { hostname, port }
}

/** The one producer history read: day buckets of one published quota account. */
export const usageHistoryPath = (ledgerAccountId: string): string =>
  `/api/v1/lens/usage_over_time?${new URLSearchParams({ window: 'all', group_by: 'account', bucket: 'day', group: ledgerAccountId })}`

export const isUsageHistoryRequest = (url: string): boolean => url === usageHistoryRoute || url.startsWith(`${usageHistoryRoute}?`)

const maxBytes = 2 * 1024 * 1024

/** A GET that must finish within `deadlineMs` of elapsed time, requires 200 and a JSON body, and returns that body verbatim. */
const producerJson = (producer: UsageHistoryProducer, path: string, deadlineMs: number): Promise<string> => {
  const { promise, resolve, reject } = Promise.withResolvers<string>()
  const upstream = request({ hostname: producer.hostname, port: producer.port, path, method: 'GET', headers: { accept: 'application/json' }, agent: false }, (response) => {
    const chunks: Buffer[] = []
    let size = 0
    response.on('error', reject)
    response.on('data', (chunk: Buffer) => {
      size += chunk.length
      if (size > maxBytes) { response.destroy(new Error('response too large')); return }
      chunks.push(chunk)
    })
    response.on('end', () => {
      if (response.statusCode !== 200) { reject(new Error(`HTTP ${response.statusCode}`)); return }
      const body = Buffer.concat(chunks).toString('utf8')
      try { JSON.parse(body) } catch { reject(new Error('response is not JSON')); return }
      resolve(body)
    })
  })
  upstream.on('error', reject)
  // Elapsed, not idle: a producer that keeps trickling bytes must not hold the single-flight slot.
  const deadline = setTimeout(() => {
    reject(new Error('deadline'))
    upstream.destroy()
  }, deadlineMs)
  upstream.end()
  return promise.finally(() => clearTimeout(deadline))
}

/** Too many distinct loads are already in flight; nothing was requested. */
class LoadsBusy extends Error {}

/**
 * Bounded insertion-ordered TTL cache: at most `entries` stored values and `entries` distinct
 * in-flight loads. Concurrent loads of one key share work; failures are never stored.
 */
const ttlCache = <T,>(ttlMs: number, entries: number) => {
  const cache = new Map<string, { value: T; expiresAt: number }>()
  const pending = new Map<string, Promise<T>>()
  return (key: string, load: () => Promise<T>): Promise<T> => {
    const cached = cache.get(key)
    if (cached !== undefined && performance.now() < cached.expiresAt) return Promise.resolve(cached.value)
    const current = pending.get(key)
    if (current !== undefined) return current
    if (pending.size >= entries) return Promise.reject(new LoadsBusy('too many producer reads in flight'))
    const loading = load().then((value) => {
      cache.delete(key)
      cache.set(key, { value, expiresAt: performance.now() + ttlMs })
      for (const stale of cache.keys()) {
        if (cache.size <= entries) break
        cache.delete(stale)
      }
      return value
    }).finally(() => pending.delete(key))
    pending.set(key, loading)
    return loading
  }
}

/** `ledgerAccountId`s the producer's quota answer publishes; nothing else may be asked for. */
const publishedAccounts = (body: string): ReadonlySet<string> => {
  const parsed: unknown = JSON.parse(body)
  const accounts = typeof parsed === 'object' && parsed !== null && 'accounts' in parsed && Array.isArray(parsed.accounts) ? parsed.accounts : []
  return new Set(accounts.flatMap((account: unknown) =>
    typeof account === 'object' && account !== null && 'ledgerAccountId' in account && typeof account.ledgerAccountId === 'string' ? [account.ledgerAccountId] : []))
}

/** The exact query: one nonempty `account` parameter of at most 256 characters and nothing else. */
const requestedAccount = (url: string): string | undefined => {
  const query = url.slice(usageHistoryRoute.length)
  if (!query.startsWith('?')) return undefined
  const params = new URLSearchParams(query.slice(1))
  const keys = [...params.keys()]
  const account = params.get('account')
  return keys.length === 1 && keys[0] === 'account' && account !== null && account.length > 0 && account.length <= 256 ? account : undefined
}

const json = (res: ServerResponse, status: number, body: string): void => {
  if (res.destroyed) return
  res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' })
  res.end(body)
}
const failure = (res: ServerResponse, status: number, code: string, message: string, reason?: string): void =>
  json(res, status, JSON.stringify(reason === undefined ? { code, message } : { code, message, reason }))
const reasonOf = (error: unknown): string => error instanceof Error ? error.message : 'failed'

export interface UsageHistoryRelay {
  /** Answer an admitted request for `usageHistoryRoute`; `preview` marks tag admission. */
  handle(req: IncomingMessage, res: ServerResponse, preview: boolean): Promise<void>
}

export const createUsageHistoryRelay = ({ producer, gateway, producerDeadlineMs = 5000 }: {
  readonly producer: UsageHistoryProducer
  readonly gateway: GatewayOptions
  /** Elapsed deadline for each producer read. */
  readonly producerDeadlineMs?: number
}): UsageHistoryRelay => {
  const allowlist = ttlCache<ReadonlySet<string>>(30_000, 1)
  const history = ttlCache<string>(60_000, 64)
  return {
    handle: async (req, res, preview) => {
      if (req.method !== 'GET') return failure(res, 405, 'method-not-allowed', 'Usage history requires GET')
      const account = requestedAccount(req.url ?? '')
      if (account === undefined) return failure(res, 400, 'invalid-query', 'Usage history requires exactly one account parameter')
      const credential = gatewayCredential(req.headers, gateway, preview)
      if (credential === undefined) return failure(res, preview ? 403 : 401, preview ? 'forbidden' : 'pairing-required', preview ? 'Forbidden' : 'Browser pairing required')
      try {
        const grant = await readCapabilities(gateway, credential, 'Usage history')
        if (grant.status === 401 || grant.status === 403) return failure(res, 403, 'forbidden', 'Forbidden')
        if (grant.status !== 200) throw new Error(`capabilities HTTP ${grant.status}`)
        if (!scopeGranted(grant.body, 'read.projections')) return failure(res, 403, 'forbidden', 'Forbidden')
      } catch (error) {
        return failure(res, 502, 'usage-unavailable', 'Usage history unavailable', reasonOf(error))
      }
      let accounts: ReadonlySet<string>
      try {
        accounts = await allowlist('quota', async () => publishedAccounts(await producerJson(producer, '/api/v1/quota', producerDeadlineMs)))
      } catch (error) {
        return failure(res, 502, 'usage-unavailable', 'Usage history unavailable', `quota ${reasonOf(error)}`)
      }
      if (!accounts.has(account)) return failure(res, 404, 'unknown-account', 'The quota answer publishes no such ledger account')
      try {
        json(res, 200, await history(account, () => producerJson(producer, usageHistoryPath(account), producerDeadlineMs)))
      } catch (error) {
        if (error instanceof LoadsBusy) failure(res, 503, 'usage-busy', 'Usage history is busy', error.message)
        else failure(res, 502, 'usage-unavailable', 'Usage history unavailable', reasonOf(error))
      }
    },
  }
}
