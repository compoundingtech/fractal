import type { ClientOptions } from '@fractal/client'
import { Effect } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it } from 'vitest'

import recording from './subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { gatewaySubjectReads } from './subjectReadPort.ts'

const envelope = (value: unknown) => ({
  api_version: 'st3.client.v0',
  snapshot: { id: 'snapshot/test', created_at: '1970-01-01T00:00:03Z' },
  value,
})
const canonical = { mission: 'mission/example', revision: 'rev-1', published_at: 3, text: 'mission "example" {}\n', legacy: false }
const publication = { kind: 'publication-definition', subject: canonical.mission, revision: canonical.revision, token: 'token-1', declaration: { gates: [{ environment: { TOKEN: 'secret' } }] } }

const gateway = (declaration: () => Response) => {
  const requests: Array<{ url: URL; signal: AbortSignal | null | undefined }> = []
  const registry = AtomRegistry.make()
  const options: ClientOptions = {
    baseUrl: 'http://gateway.invalid',
    fetchImpl: async (input, init) => {
      const url = new URL(String(input))
      if (url.pathname.endsWith('/capabilities')) return Response.json(recording.capabilities)
      requests.push({ url, signal: init?.signal })
      if (url.pathname.endsWith('/mission-declaration')) return declaration()
      if (url.pathname.endsWith('/publication-definition')) return Response.json(envelope(publication))
      throw new Error(`Unexpected request: ${url}`)
    },
  }
  return { requests, registry, read: gatewaySubjectReads({ options, registry }).missionDefinition.read(canonical.mission) }
}

it('reads served canonical KDL verbatim without requesting the structured fallback', async () => {
  const { read, requests, registry } = gateway(() => Response.json(envelope(canonical)))
  try {
    expect(await Effect.runPromise(read)).toEqual(canonical)
    expect(requests.map(({ url }) => url.pathname)).toEqual(['/v1/client/mission-declaration'])
    expect(requests[0]!.url.searchParams.get('mission')).toBe(canonical.mission)
    expect(requests[0]!.signal).toBeInstanceOf(AbortSignal)
  } finally { registry.dispose() }
})

it('falls back only for the old router empty non-JSON 404 and redacts before publishing', async () => {
  // web.rs static_asset returns bare StatusCode::NOT_FOUND for an unmatched /v1/ route.
  const { read, requests, registry } = gateway(() => new Response(null, { status: 404 }))
  try {
    const value = await Effect.runPromise(read)
    expect(value).toEqual({ ...publication, declaration: { gates: [{ environment: { TOKEN: '<redacted>' } }] } })
    expect(requests.map(({ url }) => url.pathname)).toEqual(['/v1/client/mission-declaration', '/v1/client/publication-definition'])
    expect(requests[1]!.url.searchParams.get('subject')).toBe(canonical.mission)
    expect(requests[1]!.signal).toBe(requests[0]!.signal)
  } finally { registry.dispose() }
})

it('surfaces canonical mission-not-found instead of disguising it as old-daemon fallback', async () => {
  const message = 'the mission revision is not published'
  const { read, requests, registry } = gateway(() => Response.json({
    api_version: 'st3.client.v0', error_version: 'st3.client.error.v0',
    code: 'not-found', message, retryable: false, request_id: 'request/missing', details: {},
  }, { status: 404 }))
  try {
    expect(await Effect.runPromise(Effect.flip(read))).toMatchObject({ reason: 'failed', detail: message })
    expect(requests.map(({ url }) => url.pathname)).toEqual(['/v1/client/mission-declaration'])
  } finally { registry.dispose() }
})
