/** W3C propagation at the browser HTTP and WebSocket transport edges. */
import type { CollectionSocket, CollectionSocketFactory, TraceContext } from '@fractal/client'
import { isValidTraceparent, validTraceContext } from '@fractal/sdk/effect'

/**
 * A `fetch` that adds the callback's context to each request to `baseUrl`'s origin when it starts;
 * other origins never see it. Headers the caller already set win, so wrapping a fetch used by
 * `FractalClient` (which injects its own) is harmless.
 */
export const wrapTraceFetch = ({
  fetchImpl,
  traceContext,
  baseUrl,
}: {
  readonly fetchImpl: typeof fetch
  readonly traceContext: () => TraceContext | undefined
  readonly baseUrl: string
}): typeof fetch => {
  // A relative `baseUrl` (and relative request URLs) resolve against the page, as `fetch` does.
  const base = new URL(baseUrl, globalThis.location?.href)
  return (input, init) => {
    const href = input instanceof Request ? input.url : input instanceof URL ? input.href : input
    if (new URL(href, base).origin !== base.origin) return fetchImpl(input, init)
    const context = validTraceContext(traceContext)
    if (context === undefined) return fetchImpl(input, init)
    const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined))
    if (headers.has('traceparent')) return fetchImpl(input, init)
    headers.set('traceparent', context.traceparent)
    if (context.tracestate !== undefined) headers.set('tracestate', context.tracestate)
    return fetchImpl(input, { ...init, headers })
  }
}

/** A browser `WebSocket` behind the generated client's socket surface. */
export const browserSocket: CollectionSocketFactory = (url, protocols) => {
  const socket = new WebSocket(url, protocols)
  const wrapped: CollectionSocket = {
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
    send: (data) => socket.send(data),
    // Native browsers prohibit application close code 1001; no-argument close is synchronous.
    close: (code, reason) => code === 1001 ? socket.close() : socket.close(code, reason),
  }
  socket.onopen = () => wrapped.onopen?.()
  socket.onmessage = (event) => wrapped.onmessage?.({ data: event.data })
  socket.onclose = (event) => wrapped.onclose?.({ code: event.code, reason: event.reason })
  socket.onerror = (event) => wrapped.onerror?.(event)
  return wrapped
}

/** Carry the factory headers' trace context as upgrade URL query parameters. */
export const traceQuerySocket =
  (inner: CollectionSocketFactory): CollectionSocketFactory =>
  (url, protocols, headers) => {
    const traceparent = headers['traceparent']
    if (traceparent === undefined || !isValidTraceparent(traceparent)) return inner(url, protocols, headers)
    const traced = new URL(url)
    traced.searchParams.set('traceparent', traceparent)
    const tracestate = headers['tracestate']
    if (tracestate !== undefined) traced.searchParams.set('tracestate', tracestate)
    return inner(traced.toString(), protocols, headers)
  }
