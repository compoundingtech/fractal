import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { gatewayThreads } from './threadSource.ts'

const base = fixture.capabilities
const capabilities = () => Response.json({ ...base, value: { ...base.value, capabilities: [
  ...base.value.capabilities,
  { id: 'threads-v1', version: 3, state: 'granted' },
  { id: 'thread-summary-feed', version: 1, state: 'granted' },
] } })
const page = (next_cursor: string | null = null) => ({ items: [], boundary_items: [], relations: [], source_episodes: [], complete: true, next_cursor, source_support: 'person-step' })

it('uses the generated authorized history operation and follows an empty page cursor', async () => {
  const reads: { url: URL; authorization: string | null }[] = []
  const token = 'paired-secret'
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', credential: async () => token, fetchImpl: async (input, init) => {
    const url = new URL(String(input))
    if (url.pathname.endsWith('/capabilities')) return capabilities()
    reads.push({ url, authorization: new Headers(init?.headers).get('Authorization') })
    return Response.json({ ...base, value: page(url.searchParams.has('cursor') ? null : 'opaque/+?&') })
  } }), registry)
  const atom = source.requestHistory!('thread/history-exact')
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed', value: { items: [], complete: true } }))
    expect(reads.map(read => read.url.pathname)).toEqual(['/v1/client/threads/history-exact/request-history', '/v1/client/threads/history-exact/request-history'])
    expect(reads[1]?.url.searchParams.get('cursor')).toBe('opaque/+?&')
    expect(reads.every(read => read.authorization === `Bearer ${token}`)).toBe(true)
  } finally { stop(); registry.dispose() }
})

it('clears old history immediately on refresh and does not retain it after current authorization denial', async () => {
  let denied = false
  const registry = AtomRegistry.make()
  const source = gatewayThreads(new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async input => {
    if (String(input).endsWith('/capabilities')) return capabilities()
    if (!denied) return Response.json({ ...base, value: page() })
    return Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/denied', code: 'forbidden', message: 'Read denied.', retryable: false, details: {} }, { status: 403 })
  } }), registry)
  const atom = source.requestHistory!('thread/history-exact')
  const stop = registry.mount(atom)
  try {
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Observed' }))
    denied = true
    source.reloadRequestHistory!('thread/history-exact')
    await vi.waitFor(() => expect(registry.get(atom)).toMatchObject({ _tag: 'Unavailable' }))
    expect(registry.get(atom)).not.toHaveProperty('value')
  } finally { stop(); registry.dispose() }
})
