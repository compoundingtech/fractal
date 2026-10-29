import type { CollectionSocketFactory } from '@fractal/client'
import { expect, it, vi } from 'vitest'

import { browserSocket, traceQuerySocket, wrapTraceFetch } from './trace.ts'

const TRACEPARENT = '00-0123456789abcdef0123456789abcdef-0123456789abcdef-01'

const recorded = (traceContext: () => { traceparent: string; tracestate?: string } | undefined) => {
  const seen: Array<{ url: string; traceparent: string | null; tracestate: string | null }> = []
  const fetchImpl = wrapTraceFetch({
    baseUrl: 'https://gateway.test/base',
    traceContext,
    fetchImpl: async (input, init) => {
      const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined))
      seen.push({
        url: input instanceof Request ? input.url : String(input),
        traceparent: headers.get('traceparent'),
        tracestate: headers.get('tracestate'),
      })
      return new Response('{}')
    },
  })
  return { seen, fetchImpl }
}

it('adds the active context to gateway-origin requests only', async () => {
  const { seen, fetchImpl } = recorded(() => ({ traceparent: TRACEPARENT, tracestate: 'wf=1' }))
  await fetchImpl('https://gateway.test/v1/client/resources')
  await fetchImpl(new URL('/v1/attachments', 'https://gateway.test'))
  await fetchImpl(new Request('https://gateway.test/v1/x'))
  await fetchImpl('https://elsewhere.test/v1/client/resources')
  expect(seen.map(({ traceparent, tracestate }) => [traceparent, tracestate])).toEqual([
    [TRACEPARENT, 'wf=1'],
    [TRACEPARENT, 'wf=1'],
    [TRACEPARENT, 'wf=1'],
    [null, null],
  ])
})

it('reads the context per request, keeps a caller traceparent, and drops invalid contexts', async () => {
  let active: { traceparent: string; tracestate?: string } | undefined
  const { seen, fetchImpl } = recorded(() => active)
  await fetchImpl('https://gateway.test/a')
  active = { traceparent: TRACEPARENT }
  await fetchImpl('https://gateway.test/b')
  await fetchImpl('https://gateway.test/c', {
    headers: { traceparent: '00-11111111111111111111111111111111-1111111111111111-01' },
  })
  active = { traceparent: '00-00000000000000000000000000000000-0123456789abcdef-01', tracestate: 'wf=1' }
  await fetchImpl('https://gateway.test/d')
  expect(seen.map(({ traceparent, tracestate }) => [traceparent, tracestate])).toEqual([
    [null, null],
    [TRACEPARENT, null],
    ['00-11111111111111111111111111111111-1111111111111111-01', null],
    [null, null],
  ])
})

it('closes a native browser socket synchronously without the prohibited reserved 1001 code', () => {
  const close = vi.fn((code?: number) => {
    if (code === 1001) throw new DOMException('reserved close code', 'InvalidAccessError')
  })
  vi.stubGlobal('WebSocket', class { close = close })
  try {
    const socket = browserSocket('ws://gateway.test/collections', [], {})
    socket.close(1001)
    expect(close).toHaveBeenCalledExactlyOnceWith()
  } finally {
    vi.unstubAllGlobals()
  }
})

it('propagates valid upgrade context through the browser URL and preserves factory headers', () => {
  const socket = { onopen: null, onmessage: null, onclose: null, onerror: null, send() {}, close() {} }
  const inner = vi.fn<CollectionSocketFactory>(() => socket)
  const factory = traceQuerySocket(inner)
  const headers = { traceparent: TRACEPARENT, tracestate: 'wf=1' }
  expect(factory('wss://gateway.test/collections?existing=1', ['collections'], headers)).toBe(socket)
  const [url, protocols, forwarded] = inner.mock.calls[0]!
  const traced = new URL(url)
  expect(traced.searchParams.get('existing')).toBe('1')
  expect(traced.searchParams.get('traceparent')).toBe(TRACEPARENT)
  expect(traced.searchParams.get('tracestate')).toBe('wf=1')
  expect(protocols).toEqual(['collections'])
  expect(forwarded).toBe(headers)
})

it('does not add query context for invalid upgrade traceparents', () => {
  const inner = vi.fn<CollectionSocketFactory>()
  const headers = { traceparent: 'invalid', tracestate: 'wf=1' }
  traceQuerySocket(inner)('wss://gateway.test/collections', [], headers)
  expect(inner).toHaveBeenCalledExactlyOnceWith('wss://gateway.test/collections', [], headers)
})
