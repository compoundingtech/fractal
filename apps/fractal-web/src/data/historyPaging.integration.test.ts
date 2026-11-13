import { markdownBlock } from '@fractal/client/markdown'
import type { CollectionCommand, CollectionFrame, CollectionSocket, Snapshot, TimelineEntry, TimelinePage } from '@fractal/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { liveSource } from './liveSource.ts'

const snapshot: Snapshot = {
  id: 'snapshot/1', created_at: '2026-10-03T00:00:00Z', host_id: 'host/example',
  projection_version: 'client-projection.v0', store_index: 1,
}
const ref = 'agent/example'
const sessionId = 'session/example'
const entry = (sequence: number, text = `Row ${sequence}`, revision = 1): TimelineEntry => ({
  id: `timeline-entry/${sequence}`, sequence, revision, type: 'content', role: 'assistant',
  final: true, timestamp: snapshot.created_at, body: { text: markdownBlock(text), media_type: 'text/markdown' },
})
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } })

/** Script the HTTP page independently of collection frames so races have explicit barriers. */
class Gateway {
  socket: CollectionSocket | undefined
  readonly commands: CollectionCommand[] = []
  readonly requests: { url: URL; limit: number; respond: (response: Response) => void }[] = []
  readonly cursorLimits = new Map<string, number>()

  readonly fetch: typeof fetch = async (input) => {
    const url = new URL(String(input))
    if (url.pathname === '/v1/client/sessions/example/timeline') {
      const cursor = url.searchParams.get('cursor') ?? ''
      const limit = this.cursorLimits.get(cursor)
      const requestedLimit = url.searchParams.get('limit')
      if (limit === undefined || (requestedLimit !== null && Number(requestedLimit) !== limit)) {
        return json({
          api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'page-cursor-expired',
          message: 'Cursor page size does not match', retryable: false, request_id: 'request/history', details: {},
        }, 410)
      }
      const { promise, resolve } = Promise.withResolvers<Response>()
      this.requests.push({ url, limit, respond: resolve })
      return promise
    }
    if (url.pathname !== '/v1/client/capabilities') throw new Error(`Unexpected request ${url}`)
    return json({ api_version: 'st3.client.v0', snapshot, value: {
      kind: 'capabilities', capabilities: [{ id: 'work.done', state: 'granted', version: 0 }],
      event_cursor: 'cursor/current',
      limits: { max_page_items: 200, max_event_items: 100, max_wait_ms: 1000, max_response_bytes: 65536 },
      schemas: ['client-v0'], session_actor: 'person/operator', transport: 'fabric-loopback',
    } })
  }

  readonly factory = () => {
    const socket: CollectionSocket = {
      onopen: null, onmessage: null, onclose: null, onerror: null,
      send: (text: string) => this.commands.push(JSON.parse(text)), close: () => {},
    }
    this.socket = socket
    queueMicrotask(() => socket.onopen?.())
    return socket
  }

  subscription() {
    return this.commands.findLast(
      (command) => command.kind === 'subscribe' && command.collection === 'conversation',
    )
  }

  frame(rows: TimelineEntry[], replace = false, cursor?: string, limit = 200) {
    if (cursor !== undefined) this.cursorLimits.set(cursor, limit)
    const subscription = this.subscription()
    if (subscription === undefined) throw new Error('No conversation subscription')
    const frame: CollectionFrame = {
      kind: 'conversation', id: subscription.id, collection: 'conversation', session_id: sessionId,
      items: rows, replace, has_more: cursor !== undefined,
      ...(cursor === undefined ? {} : { older_cursor: cursor }),
    }
    this.socket?.onmessage?.({ data: JSON.stringify(frame) })
  }

  page(index: number, items: TimelineEntry[], nextCursor?: string) {
    const limit = this.requests[index]!.limit
    if (nextCursor !== undefined) this.cursorLimits.set(nextCursor, limit)
    const value: TimelinePage = {
      kind: 'timeline-page', session_id: sessionId, items,
      page: { limit, has_more: nextCursor !== undefined, ...(nextCursor === undefined ? {} : { next_cursor: nextCursor }) },
    }
    this.requests[index]!.respond(json({ api_version: 'st3.client.v0', snapshot, value }))
  }

  fail(index: number) {
    this.requests[index]!.respond(json({
      api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'unavailable',
      message: 'History unavailable', retryable: true, request_id: 'request/history', details: {},
    }, 503))
  }
}

let frames = new Map<number, FrameRequestCallback>()
beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  frames = new Map()
  let nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = ++nextFrame
    frames.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
})
afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})
const drain = async () => {
  const { promise, resolve } = Promise.withResolvers<void>()
  setImmediate(resolve)
  await promise
  await vi.advanceTimersByTimeAsync(1)
  const pending = [...frames.values()]
  frames.clear()
  for (const callback of pending) callback(0)
}
const until = async (check: () => boolean) => {
  for (let round = 0; round < 400 && !check(); round += 1) await drain()
  if (!check()) throw new Error('condition not reached before the drain budget ran out')
}
const open = async (cursorLimit = 200) => {
  const gateway = new Gateway()
  const live = liveSource({ options: { baseUrl: 'http://gateway.test', maxFollows: 12, socket: gateway.factory, fetch: gateway.fetch } })
  const release = live.registry.mount(live.source.conversationInterest!(ref))
  const read = () => {
    const feed = live.registry.get(live.source.conversation(ref))
    if (feed._tag !== 'Observed') throw new Error(`Conversation is ${feed._tag}`)
    return feed.value
  }
  try {
    await until(() => gateway.subscription() !== undefined)
    gateway.frame([entry(5), entry(6)], true, 'cursor/older-1', cursorLimit)
    await until(() => live.registry.get(live.source.conversation(ref))._tag === 'Observed')
    return {
      gateway, live, read,
      ids: () => read().items.map((item) => item.id),
      load: () => live.source.loadOlder!(ref),
      close: async () => { release(); await live.dispose() },
    }
  } catch (error) {
    release()
    await live.dispose()
    throw error
  }
}

describe('conversation history paging through the SDK and liveSource', () => {
  it.each([200, 5])('inherits cursor-bound limit %i, coalesces clicks, and prepends sequential pages to completion', async (limit) => {
    const ctx = await open(limit)
    try {
      const first = ctx.load()
      const concurrent = ctx.load()
      await until(() => ctx.gateway.requests.length === 1 && ctx.read().older?._tag === 'Loading')
      expect(ctx.ids()).toEqual(['timeline-entry/5', 'timeline-entry/6'])
      expect(ctx.gateway.requests[0]!.url.searchParams.get('cursor')).toBe('cursor/older-1')
      expect(ctx.gateway.requests[0]!.url.searchParams.has('limit')).toBe(false)
      expect(ctx.gateway.requests[0]!.limit).toBe(limit)
      ctx.gateway.page(0, [entry(3), entry(4)], 'cursor/older-2')
      await Promise.all([first, concurrent])
      await until(() => ctx.read().items.length === 4)
      expect(ctx.gateway.requests).toHaveLength(1)
      expect(ctx.ids()).toEqual([3, 4, 5, 6].map((id) => `timeline-entry/${id}`))
      expect(ctx.read()).toMatchObject({ hasOlder: true, olderCursor: 'cursor/older-2' })
      expect(ctx.read().older).toBeUndefined()

      const second = ctx.load()
      await until(() => ctx.gateway.requests.length === 2)
      expect(ctx.gateway.requests[1]!.url.searchParams.get('cursor')).toBe('cursor/older-2')
      ctx.gateway.page(1, [entry(1), entry(2)])
      await second
      await until(() => ctx.read().items.length === 6)
      expect(ctx.ids()).toEqual([1, 2, 3, 4, 5, 6].map((id) => `timeline-entry/${id}`))
      expect(ctx.read().hasOlder).toBe(false)
      expect(ctx.read().olderCursor).toBeUndefined()
      expect(ctx.read().older).toBeUndefined()
      await ctx.load()
      expect(ctx.gateway.requests).toHaveLength(2)
    } finally { await ctx.close() }
  })

  it('keeps live arrivals and newer revisions while deduplicating an overlapping history page', async () => {
    const ctx = await open()
    try {
      const loading = ctx.load()
      await until(() => ctx.gateway.requests.length === 1)
      ctx.gateway.frame([entry(4, 'Live revision', 2), entry(7)])
      await until(() => ctx.ids().includes('timeline-entry/7'))
      ctx.gateway.page(0, [entry(3), entry(4, 'Stale history revision'), entry(5)])
      await loading
      await until(() => ctx.read().older === undefined)
      expect(ctx.ids()).toEqual([3, 4, 5, 6, 7].map((id) => `timeline-entry/${id}`))
      expect(ctx.read().items.find((item) => item.id === 'timeline-entry/4')).toMatchObject({ text: 'Live revision' })
      expect(ctx.read().items.find((item) => item.id === 'timeline-entry/7')).toMatchObject({ text: 'Row 7' })
    } finally { await ctx.close() }
  })

  it('retains the loaded window after failure and retries the same cursor only on explicit demand', async () => {
    const ctx = await open()
    try {
      const loading = ctx.load()
      await until(() => ctx.gateway.requests.length === 1)
      ctx.gateway.fail(0)
      await loading
      await until(() => ctx.read().older?._tag === 'Failed')
      expect(ctx.ids()).toEqual(['timeline-entry/5', 'timeline-entry/6'])
      expect(ctx.read()).toMatchObject({ hasOlder: true, olderCursor: 'cursor/older-1', older: { _tag: 'Failed', detail: expect.any(String) } })
      await vi.advanceTimersByTimeAsync(10_000)
      await drain()
      expect(ctx.gateway.requests).toHaveLength(1)

      const retry = ctx.load()
      await until(() => ctx.gateway.requests.length === 2 && ctx.read().older?._tag === 'Loading')
      expect(ctx.ids()).toEqual(['timeline-entry/5', 'timeline-entry/6'])
      expect(ctx.gateway.requests[1]!.url.searchParams.get('cursor')).toBe('cursor/older-1')
      ctx.gateway.page(1, [entry(3), entry(4)])
      await retry
      await until(() => ctx.read().items.length === 4)
      expect(ctx.ids()).toEqual([3, 4, 5, 6].map((id) => `timeline-entry/${id}`))
      expect(ctx.read().older).toBeUndefined()
    } finally { await ctx.close() }
  })

  it('releases an invalidated request without letting its finalizer clear a replacement request', async () => {
    const ctx = await open()
    try {
      const loading = ctx.load()
      await until(() => ctx.gateway.requests.length === 1)
      ctx.gateway.frame([entry(10)], true, 'cursor/replaced')
      await until(() => ctx.ids()[0] === 'timeline-entry/10')
      expect(ctx.read().older).toBeUndefined()
      const replacementLoad = ctx.load()
      const concurrent = ctx.load()
      await until(() => ctx.gateway.requests.length === 2 && ctx.read().older?._tag === 'Loading')
      expect(ctx.gateway.requests[1]!.url.searchParams.get('cursor')).toBe('cursor/replaced')
      ctx.gateway.page(0, [entry(3), entry(4)])
      await loading
      await drain()
      expect(ctx.ids()).toEqual(['timeline-entry/10'])
      expect(ctx.read()).toMatchObject({ hasOlder: true, olderCursor: 'cursor/replaced' })
      expect(ctx.read().older?._tag).toBe('Loading')
      const afterObsoleteCompletion = ctx.load()
      expect(ctx.gateway.requests).toHaveLength(2)
      ctx.gateway.page(1, [entry(9)])
      await Promise.all([replacementLoad, concurrent, afterObsoleteCompletion])
      await until(() => ctx.read().items.length === 2)
      expect(ctx.ids()).toEqual(['timeline-entry/9', 'timeline-entry/10'])
    } finally { await ctx.close() }
  })
})
