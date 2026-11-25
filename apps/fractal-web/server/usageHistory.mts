import type { IncomingMessage, ServerResponse } from 'node:http'
import { gatewayCredential, type GatewayOptions } from './gateway.mts'
import { LoadsBusy, failure, json, producerJson, reasonOf, ttlCache, type LoopbackProducer } from './relayHttp.mts'
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

/** The one producer history read: day buckets of one published quota account. */
export const usageHistoryPath = (ledgerAccountId: string): string =>
  `/api/v1/lens/usage_over_time?${new URLSearchParams({ window: 'all', group_by: 'account', bucket: 'day', group: ledgerAccountId })}`

export const isUsageHistoryRequest = (url: string): boolean => url === usageHistoryRoute || url.startsWith(`${usageHistoryRoute}?`)

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

export interface UsageHistoryRelay {
  /** Answer an admitted request for `usageHistoryRoute`; `preview` marks tag admission. */
  handle(req: IncomingMessage, res: ServerResponse, preview: boolean): Promise<void>
}

export const createUsageHistoryRelay = ({ producer, gateway, producerDeadlineMs = 5000 }: {
  readonly producer: LoopbackProducer
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
