import type { CollectionSocket, CollectionCommand, CollectionFrame, TimelineEntry } from '@smalltalk/st3-client'
import { decodeConversationChunk } from '@st3/sdk/effect'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import { expect, it } from 'vitest'
import { liveSource } from './liveSource.ts'

const snapshot = { id: 'snapshot/history', created_at: '2026-01-15T12:00:00Z', host_id: 'host/example', projection_version: 'client-projection.v0', store_index: 1 }
const entry = (sequence: number, text: string): TimelineEntry => ({ id: `timeline-entry/${sequence}`, sequence, revision: 1, type: 'content', role: 'assistant', final: true, timestamp: snapshot.created_at, body: { text, media_type: 'text/markdown' } })

it.each(['omitted', 'null'] as const)('uses native session/cursors and terminal %s cursors, keeps concurrent output, and retries without losing loaded data', async terminalCursor => {
  let socket: CollectionSocket | undefined
  const commands: CollectionCommand[] = []
  const requests: URL[] = []
  let settle: (() => void) | undefined
  let fail = false
  const fetcher: typeof fetch = async input => {
    const url = new URL(String(input))
    if (url.pathname === '/v1/client/capabilities') return Response.json({ api_version: 'st3.client.v0', snapshot, value: { kind: 'capabilities', capabilities: [], event_cursor: 'cursor/current', limits: { max_page_items: 100, max_event_items: 100, max_wait_ms: 1000, max_response_bytes: 65536 }, schemas: ['client-v0'], session_actor: 'person/operator', transport: 'fabric-loopback' } })
    requests.push(url)
    if (url.searchParams.has('cursor')) {
      await new Promise<void>(resolve => { settle = resolve })
      if (fail) return Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'unavailable', message: 'History service unavailable', retryable: true, request_id: 'request/history', details: {} }, { status: 503 })
    }
    return Response.json({ api_version: 'st3.client.v0', snapshot, value: { kind: 'timeline-page', session_id: `session/${url.pathname.split('/').at(-2)}`, items: url.searchParams.has('cursor') ? [entry(1, 'Earlier result'), entry(2, 'Boundary result')] : [entry(2, 'Boundary result'), entry(3, 'Recent result')], page: { limit: 100, has_more: !url.searchParams.has('cursor'), ...(!url.searchParams.has('cursor') ? { next_cursor: 'cursor/older' } : terminalCursor === 'null' ? { next_cursor: null } : {}) } } })
  }
  const live = liveSource({ options: { baseUrl: 'http://gateway.example', maxFollows: 4, fetch: fetcher, socket: () => {
    socket = { onopen: null, onmessage: null, onclose: null, onerror: null, close() {}, send(data) { commands.push(JSON.parse(String(data)) as CollectionCommand) } }
    queueMicrotask(() => socket?.onopen?.())
    return socket
  } } })
  const ref = 'agent/history'
  const atom = live.source.conversation(ref)
  const release = live.registry.mount(live.source.conversationInterest!(ref))
  const frame = (items: TimelineEntry[], replace = false, session = 'session/history'): CollectionFrame => ({ kind: 'conversation', id: commands.findLast(command => command.kind === 'subscribe' && command.collection === 'conversation')!.id, collection: 'conversation', session_id: session, replace, has_more: true, items })
  const send = (value: CollectionFrame) => socket!.onmessage!({ data: JSON.stringify(value) })
  const page = () => { const feed = live.registry.get(atom); if (feed._tag !== 'Observed') throw new Error('Expected observed history'); return feed.value }
  try {
    await expect.poll(() => commands.some(command => command.kind === 'subscribe' && command.collection === 'conversation')).toBe(true)
    send(frame([entry(2, 'Boundary result'), entry(3, 'Recent result')], true))
    await expect.poll(() => live.registry.get(atom)._tag).toBe('Observed')
    const loading = live.source.loadEarlierConversation!(ref)
    await expect.poll(() => requests.length).toBe(2)
    expect(page().historyLoad?._tag).toBe('Loading')
    await live.source.loadEarlierConversation!(ref)
    expect(requests).toHaveLength(2)
    send(frame([entry(4, 'Concurrent live output')]))
    await expect.poll(() => page().items.length).toBe(3)
    fail = true
    settle!()
    await loading
    expect(page().historyLoad?._tag).toBe('Failed')
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/2', 'timeline-entry/3', 'timeline-entry/4'])
    fail = false
    const retry = live.source.loadEarlierConversation!(ref)
    await expect.poll(() => requests.length).toBe(4)
    settle!()
    await retry
    expect(page().hasOlder).toBe(false)
    expect(page().historyLoad).toBeUndefined()
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/1', 'timeline-entry/2', 'timeline-entry/3', 'timeline-entry/4'])
    expect(requests.every(url => url.pathname === '/v1/client/sessions/history/timeline')).toBe(true)
    expect(requests[1]!.searchParams.get('cursor')).toBe('cursor/older')
    send(frame([entry(2, 'Boundary result'), entry(3, 'Recent result')], true, 'session/second'))
    await expect.poll(() => page().hasOlder).toBe(true)
    const obsolete = live.source.loadEarlierConversation!(ref)
    await expect.poll(() => requests.length).toBe(6)
    send(frame([entry(10, 'New session window')], true, 'session/third'))
    await expect.poll(() => page().items[0]?.id).toBe('timeline-entry/10')
    settle!()
    await obsolete
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/10'])
    expect(page().historyLoad).toBeUndefined()
  } finally { release(); await live.dispose() }
})

it('keeps the proven earlier prefix and an in-flight cursor when a same-session live window is replaced', async () => {
  let socket: CollectionSocket | undefined
  let settle: (() => void) | undefined
  const commands: CollectionCommand[] = []
  const requests: URL[] = []
  const fetcher: typeof fetch = async input => {
    const url = new URL(String(input))
    requests.push(url)
    const cursor = url.searchParams.get('cursor')
    if (cursor === 'cursor/oldest') await new Promise<void>(resolve => { settle = resolve })
    return Response.json({ api_version: 'st3.client.v0', snapshot, value: {
      kind: 'timeline-page', session_id: 'session/history',
      items: cursor === 'cursor/oldest' ? [entry(0, 'Conversation start'), entry(1, 'Earlier result')]
        : cursor === 'cursor/older' ? [entry(1, 'Earlier result'), entry(2, 'Boundary result')]
          : [entry(2, 'Boundary result'), entry(3, 'Recent result')],
      page: { limit: 50, has_more: cursor !== 'cursor/oldest', ...(cursor === 'cursor/oldest' ? {} : { next_cursor: cursor === 'cursor/older' ? 'cursor/oldest' : 'cursor/older' }) },
    } })
  }
  const live = liveSource({ options: { baseUrl: 'http://gateway.example', maxFollows: 4, fetch: fetcher, socket: () => {
    socket = { onopen: null, onmessage: null, onclose: null, onerror: null, close() {}, send(data) { commands.push(JSON.parse(String(data)) as CollectionCommand) } }
    queueMicrotask(() => socket?.onopen?.())
    return socket
  } } })
  const ref = 'agent/history'
  const atom = live.source.conversation(ref)
  const release = live.registry.mount(live.source.conversationInterest!(ref))
  const frame = (items: TimelineEntry[], hasMore = true): Extract<CollectionFrame, { kind: 'conversation' }> => ({ kind: 'conversation', id: commands.findLast(command => command.kind === 'subscribe' && command.collection === 'conversation')!.id, collection: 'conversation', session_id: 'session/history', replace: true, has_more: hasMore, items })
  const send = (value: CollectionFrame) => socket!.onmessage!({ data: JSON.stringify(value) })
  const page = () => { const feed = live.registry.get(atom); if (feed._tag !== 'Observed') throw new Error('Expected observed history'); return feed.value }
  try {
    await expect.poll(() => commands.some(command => command.kind === 'subscribe' && command.collection === 'conversation')).toBe(true)
    send(frame([entry(2, 'Boundary result'), entry(3, 'Recent result')]))
    await expect.poll(() => live.registry.get(atom)._tag).toBe('Observed')
    await live.source.loadEarlierConversation!(ref)
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/1', 'timeline-entry/2', 'timeline-entry/3'])
    const earlier = page().items[0]
    const loading = live.source.loadEarlierConversation!(ref)
    await expect.poll(() => requests.filter(url => url.pathname.endsWith('/timeline')).map(url => url.searchParams.get('cursor'))).toContain('cursor/oldest')
    const replacement = frame([entry(3, 'Revised live result'), entry(4, 'New live result')])
    send(replacement)
    await expect.poll(() => page().items.at(-1)?.id).toBe('timeline-entry/4')
    expect(page().items[0]).toBe(earlier)
    expect(page().historyLoad?._tag).toBe('Loading')
    settle!()
    await loading
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/0', 'timeline-entry/1', 'timeline-entry/2', 'timeline-entry/3', 'timeline-entry/4'])
    expect(page().hasOlder).toBe(false)
    send(frame([entry(3, 'Revised live result'), entry(5, 'New authoritative tail')]))
    await expect.poll(() => page().items.at(-1)?.id).toBe('timeline-entry/5')
    expect(page().items.map(item => item.id)).toEqual(['timeline-entry/0', 'timeline-entry/1', 'timeline-entry/2', 'timeline-entry/3', 'timeline-entry/5'])
    expect(page().hasOlder).toBe(false)

    // Failing control: the old whole-history replacement loses the very same prefix.
    const control = new LiveTimeline()
    control.apply(decodeConversationChunk(frame([entry(3, 'Recent result')])))
    control.prepend(decodeConversationChunk(frame([entry(1, 'Earlier result'), entry(2, 'Boundary result')])).entries, true)
    control.apply(decodeConversationChunk(replacement))
    const retainsEarlier = (items: readonly { readonly id: string }[]) => items.some(item => item.id === 'timeline-entry/1')
    expect(retainsEarlier(page().items)).toBe(true)
    expect(retainsEarlier(control.project().items)).toBe(false)

    // A complete window is authoritative; an unrelated window cannot prove prefix order.
    send(frame([entry(9, 'Complete replacement')], false))
    await expect.poll(() => page().items.map(item => item.id)).toEqual(['timeline-entry/9'])
    expect(page().hasOlder).toBe(false)
  } finally { release(); await live.dispose() }
})
