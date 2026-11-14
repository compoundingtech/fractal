import { describe, expect, it, vi } from 'vitest'
import { getDebug } from './measurement/index.ts'
import { instrumentFetch, readRequestLog, subscribeRequestLog } from './transport.ts'

const parent = '00-0123456789abcdef0123456789abcdef-0123456789abcdef-01'
const child = '00-0123456789abcdef0123456789abcdef-abcdef0123456789-01'
const origin = 'https://example.test'
const harness = () => {
  const requests: Headers[] = []
  const fetchImpl: typeof fetch = async (input, init) => {
    requests.push(new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined)))
    return new Response(undefined, { status: 204 })
  }
  return { requests, fetch: instrumentFetch({ fetchImpl, origin, traceContext: () => ({ traceparent: parent, tracestate: 'vendor=value' }) }) }
}

describe('same-origin UX context propagation', () => {
  it('adds W3C context while preserving request headers and caller init', async () => {
    const h = harness()
    const init = { headers: { accept: 'application/json' } }
    await h.fetch(`${origin}/v1/client/agents`, init)
    expect(h.requests[0]?.get('traceparent')).toBe(parent)
    expect(h.requests[0]?.get('tracestate')).toBe('vendor=value')
    expect(h.requests[0]?.get('accept')).toBe('application/json')
    expect(init).toEqual({ headers: { accept: 'application/json' } })
  })
  it('keeps SDK child context instead of overwriting it with the UX root', async () => {
    const h = harness()
    await h.fetch(new Request(`${origin}/v1/client/capabilities`, { headers: { traceparent: child } }))
    expect(h.requests[0]?.get('traceparent')).toBe(child)
    expect(h.requests[0]?.get('tracestate')).toBeNull()
  })
  it('does not leak context across origins', async () => {
    const h = harness()
    await h.fetch('https://other.test/v1/client/agents')
    expect(h.requests[0]?.has('traceparent')).toBe(false)
    expect(h.requests[0]?.has('tracestate')).toBe(false)
  })
})

describe('latency instrumentation', () => {
  const instrumented = (fetchImpl: typeof fetch) => instrumentFetch({ fetchImpl, origin })

  it('has no latency sample until a response arrives', async () => {
    const fetch = instrumented(async () => new Response(undefined, { status: 204 }))
    expect(getDebug('Wf.httpInstrumented')).toBe(1)
    expect(getDebug('Wf.httpSamples')).toBe(0)
    await fetch(`${origin}/a`)
    expect(getDebug('Wf.httpSamples')).toBe(1)
  })

  it('counts a failure as an error but not as latency, and a cancellation as neither', async () => {
    const failures = [new TypeError('network'), new DOMException('stop', 'AbortError')]
    const fetch = instrumented(async () => { throw failures.shift() })
    await expect(fetch(`${origin}/a`)).rejects.toThrow('network')
    await expect(fetch(`${origin}/a`)).rejects.toThrow('stop')
    expect(getDebug('Wf.httpSamples')).toBe(0)
    expect(getDebug('Wf.httpInFlight')).toBe(0)
    expect(getDebug('Wf.httpErrors')).toBe(1)
    expect(readRequestLog().errors).toMatchObject([{ method: 'GET', path: '/a', outcome: 'network' }])
  })

  it('names the route and status behind an error, without the query string', async () => {
    const fetch = instrumented(async () => new Response(undefined, { status: 404 }))
    await fetch(`${origin}/v1/client/usage/quota?token=secret`)
    expect(readRequestLog().errors).toMatchObject([{ path: '/v1/client/usage/quota', outcome: 404 }])
    expect(readRequestLog().slowest).toMatchObject([{ path: '/v1/client/usage/quota', outcome: 404 }])
  })
})

describe('request log publication', () => {
  const instrumented = (fetchImpl: typeof fetch) => instrumentFetch({ fetchImpl, origin })

  it('notifies on each change, keeps the snapshot stable between them, and stops on unsubscribe', async () => {
    const fetch = instrumented(async () => new Response(undefined, { status: 500 }))
    let notified = 0
    const unsubscribe = subscribeRequestLog(() => { notified++ })
    const before = readRequestLog()
    expect(readRequestLog()).toBe(before)

    await fetch(`${origin}/a`)
    // One response records an error and a slowest entry, each its own published change.
    expect(notified).toBe(2)
    const after = readRequestLog()
    expect(after).not.toBe(before)
    expect(after.errors).toMatchObject([{ path: '/a', outcome: 500 }])
    expect(before.errors).toEqual([])

    unsubscribe()
    await fetch(`${origin}/b`)
    expect(notified).toBe(2)
    expect(readRequestLog().errors).toHaveLength(2)
  })

  it('does not republish a response too quick to enter a full slowest list', async () => {
    let durationMs = 1000
    let now = 0
    const clock = vi.spyOn(performance, 'now').mockImplementation(() => now)
    const fetch = instrumented(async () => {
      now += durationMs
      return new Response(undefined, { status: 204 })
    })
    for (let index = 0; index < 8; index++) await fetch(`${origin}/slow`)
    const full = readRequestLog()
    let notified = 0
    const unsubscribe = subscribeRequestLog(() => { notified++ })
    durationMs = 1
    await fetch(`${origin}/quick`)
    expect(notified).toBe(0)
    expect(readRequestLog()).toBe(full)
    unsubscribe()
    clock.mockRestore()
  })

  it('publishes the reset when a new source instruments fetch', async () => {
    await instrumented(async () => new Response(undefined, { status: 500 }))(`${origin}/a`)
    let notified = 0
    const unsubscribe = subscribeRequestLog(() => { notified++ })
    instrumented(async () => new Response(undefined, { status: 204 }))
    expect(notified).toBe(1)
    expect(readRequestLog()).toEqual({ errors: [], slowest: [] })
    unsubscribe()
  })
})
