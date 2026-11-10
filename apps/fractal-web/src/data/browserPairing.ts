import { Schema } from 'effect'
import type { BrowserPairingPort } from './source.ts'

const decodeStatus = Schema.decodeUnknownSync(Schema.Struct({
  status: Schema.Literals(['unpaired', 'paired', 'pending', 'denied', 'expired']),
  displayCode: Schema.optional(Schema.String),
  credentialChanged: Schema.optional(Schema.Boolean),
  credentialRejected: Schema.optional(Schema.Boolean),
}))
const decodeStarted = Schema.decodeUnknownSync(Schema.Struct({ requestId: Schema.String, displayCode: Schema.String }))

/** A pairing endpoint answered, but not with success; network failures stay plain fetch errors. */
export class PairingHttpError extends Error {
  readonly status: number
  constructor(status: number) {
    super(`Browser pairing request failed with HTTP ${status}`)
    this.status = status
  }
}

/**
 * The serving host's pairing endpoints. Requests carry only same-origin cookies; the credential
 * the host sets on approval is never visible to this code.
 */
export const browserPairingPort = ({ baseUrl, fetch: fetchImpl = (input, init) => globalThis.fetch(input, init) }: {
  readonly baseUrl: string
  readonly fetch?: (input: URL, init: RequestInit) => Promise<Response>
}): BrowserPairingPort => {
  const request = (path: 'start' | 'status' | 'forget' | 'redeem', body?: unknown) => fetchImpl(new URL(`/_fractal/pair/${path}`, baseUrl), {
    method: path === 'status' ? 'GET' : 'POST',
    credentials: 'same-origin',
    cache: 'no-store',
    headers: body === undefined ? { accept: 'application/json' } : { accept: 'application/json', 'content-type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  })
  const json = async (response: Response) => {
    if (!response.ok) throw new PairingHttpError(response.status)
    return response.json() as Promise<unknown>
  }
  return {
    status: async () => {
      const response = await request('status')
      if (response.status === 404) return undefined
      const status = decodeStatus(await json(response))
      // Reads never adopt or discard credentials. Explicit same-origin POSTs do that work.
      return status.status === 'pending' || status.credentialRejected === true
        ? decodeStatus(await json(await request('redeem', {})))
        : status
    },
    start: async (deviceLabel) => decodeStarted(await json(await request('start', { deviceLabel }))),
    forget: async () => {
      const response = await request('forget', {})
      if (!response.ok) throw new PairingHttpError(response.status)
    },
  }
}
