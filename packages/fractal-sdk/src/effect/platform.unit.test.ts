import { it } from '@effect/vitest'
import { API_VERSION, type CollectionSocket, type CollectionSocketFactory } from '@fractal/client'
import * as Effect from 'effect/Effect'
import * as Fiber from 'effect/Fiber'
import * as Stream from 'effect/Stream'
import { describe, expect } from 'vitest'
import { makeAdmission } from './admission.ts'
import { collectionWindow, followKey, Fractal, FractalLive, type FollowDescriptor } from './mod.ts'

const snapshot = {
  id: 'snapshot/1', created_at: '2026-10-21T00:00:00Z', host_id: 'host/test',
  projection_version: 'client-projection.v0', store_index: 1,
}

const settle = Effect.promise(async () => {
  for (let turn = 0; turn < 20; turn += 1)
    await new Promise<void>((resolve) => setImmediate(resolve))
})

describe('host-independent SDK', () => {
  it.live('builds and follows without browser globals, sharing and releasing one authenticated socket', () => {
    const commands: { kind: string; id: string; collection?: string }[] = []
    const httpAuthorization: (string | null)[] = []
    const httpClients: (string | null)[] = []
    let socketCount = 0
    let closes = 0
    let socketAuthorization: string | undefined
    let socketClient: string | undefined
    const socket: CollectionSocket = {
      onopen: null, onmessage: null, onclose: null, onerror: null,
      send: (text) => { commands.push(JSON.parse(text)) },
      close: () => { closes += 1 },
    }
    const factory: CollectionSocketFactory = (_url, _protocols, headers) => {
      socketCount += 1
      socketAuthorization = headers.Authorization
      socketClient = headers['x-st3-client']
      queueMicrotask(() => socket.onopen?.())
      return socket
    }
    const layer = FractalLive({
      baseUrl: 'http://gateway.test', maxFollows: 4,
      credential: () => 'paired-test-credential', client: 'sdk-node-test', socket: factory,
      fetch: async (_input, init) => {
        httpAuthorization.push(new Headers(init?.headers).get('authorization'))
        httpClients.push(new Headers(init?.headers).get('x-st3-client'))
        return new Response(JSON.stringify({ api_version: API_VERSION, snapshot, value: { capabilities: [] } }))
      },
    })
    return Effect.gen(function* () {
      expect('window' in globalThis).toBe(false)
      expect('document' in globalThis).toBe(false)
      yield* Effect.gen(function* () {
        const sdk = yield* Fractal
        const first = yield* Effect.forkChild(sdk.watch(collectionWindow({ _tag: 'Window', collection: 'agents', limit: 50 })).pipe(Stream.runDrain))
        // A descriptor can choose its decoded value without another projection or socket.
        const descriptor: FollowDescriptor<{ readonly identity: string }> = {
          spec: { _tag: 'Window', collection: 'missions', limit: 25, descriptor: 'mission-identity' },
          decodeRow: (row) => ({ value: { identity: row.id }, undecodable: [] }),
        }
        const identities: string[] = []
        const second = yield* Effect.forkChild(sdk.watch(descriptor).pipe(Stream.runForEach((event) =>
          Effect.sync(() => {
            if (event._tag === 'Observed') identities.push(...event.value.items.map((item) => item.identity))
          }),
        )))
        const history: string[] = []
        const third = yield* Effect.forkChild(sdk.followThreadTimeline({ _tag: 'ThreadTimeline', thread: 'thread/test', limit: 40 })
          .pipe(Stream.runForEach(event => Effect.sync(() => {
            if (event._tag === 'Observed') history.push(...event.value.items.map(item => item.id))
          }))))
        const summaries: Array<{ readonly unread: boolean | undefined; readonly snapshotId: string }> = []
        const fourth = yield* Effect.forkChild(sdk.followThreadSummaries({ _tag: 'ThreadSummary', thread: 'thread/test', limit: 1 })
          .pipe(Stream.runForEach(event => Effect.sync(() => {
            if (event._tag === 'Observed') summaries.push({ unread: event.value.items[0]?.unread, snapshotId: event.value.snapshotId })
          }))))
        yield* settle
        expect(socketCount).toBe(1)
        expect(socketAuthorization).toBe('Bearer paired-test-credential')
        expect(httpAuthorization).toContain('Bearer paired-test-credential')
        expect(socketClient).toBe('sdk-node-test')
        expect(httpClients).toContain('sdk-node-test')
        expect(commands.filter((command) => command.kind === 'subscribe').map((command) => command.collection)).toEqual(['agents', 'missions', 'thread-timeline', 'threads'])
        expect(followKey(descriptor.spec)).not.toBe(followKey({ ...descriptor.spec, descriptor: 'another-decoder' }))
        const subscription = commands.find((command) => command.collection === 'missions')!
        socket.onmessage?.({ data: JSON.stringify({
          kind: 'snapshot', id: subscription.id, collection: 'missions', has_more: false,
          items: [{ id: 'mission/test', revision: '1' }], order: ['mission/test'], snapshot,
        }) })
        const thread = commands.find(command => command.collection === 'thread-timeline')!
        socket.onmessage?.({ data: JSON.stringify({
          kind: 'snapshot', id: thread.id, collection: 'thread-timeline', has_more: false,
          items: [{ kind: 'message', id: 'message/test', actor_id: 'agent/test', accepted_at_unix_ms: 5, content: 'Done', thread_quote_id: null }],
          order: ['message/test'], snapshot,
        }) })
        const exact = commands.find(command => command.collection === 'threads')!
        expect(exact).toMatchObject({ collection: 'threads', subject: 'thread/test', limit: 1 })
        expect(exact).not.toHaveProperty('thread_context')
        socket.onmessage?.({ data: JSON.stringify({
          kind: 'snapshot', id: exact.id, collection: 'threads', has_more: false,
          items: [{ id: 'thread/test', agent_id: 'agent/test', title: 'Test', title_revision: 'claim/title', revision: 'claim/title', state: 'open', unread: true }],
          order: ['thread/test'], snapshot,
        }) })
        // Ingest and Effect publication complete on separate scheduler turns.
        yield* Effect.promise(async () => {
          await expect.poll(() => identities, { timeout: 2_000 }).toEqual(['mission/test'])
          await expect.poll(() => history, { timeout: 2_000 }).toEqual(['message/test'])
          await expect.poll(() => summaries, { timeout: 2_000 }).toEqual([{ unread: true, snapshotId: snapshot.id }])
        })
        yield* Fiber.interrupt(first)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(1)
        yield* Fiber.interrupt(second)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(2)
        yield* Fiber.interrupt(third)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(3)
        yield* Fiber.interrupt(fourth)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(4)
      }).pipe(Effect.provide(layer), Effect.scoped)
      expect(closes).toBeGreaterThan(0)
    })
  })

  it.effect('neutral admission never treats a host visibility hint as permission to evict', () => Effect.sync(() => {
    const admission = makeAdmission<string>({ cap: 1 })
    expect(admission.open('first')).toEqual({ _tag: 'Admitted', evict: undefined })
    admission.setVisible({ key: 'first', visible: false })
    expect(admission.open('second')).toEqual({ _tag: 'Full' })
    admission.close('first')
    expect(admission.open('second')).toEqual({ _tag: 'Admitted', evict: undefined })
  }))
})

for (const granted of [true, false]) {
  it.live(`only requests Inbox thread hints when the daemon grants them (${granted})`, () => {
    const commands: unknown[] = []
    const socket: CollectionSocket = {
      onopen: null, onmessage: null, onclose: null, onerror: null,
      send: text => { commands.push(JSON.parse(text)) },
      close: () => {},
    }
    const layer = FractalLive({
      baseUrl: 'http://gateway.test', maxFollows: 1, client: 'sdk-node-test',
      socket: () => { queueMicrotask(() => socket.onopen?.()); return socket },
      fetch: async () => Response.json({ api_version: API_VERSION, snapshot, value: {
        capabilities: [{ id: 'collection.attention.thread-hints', version: 1, state: granted ? 'granted' : 'ungranted' }],
      } }),
    })
    return Effect.gen(function* () {
      const sdk = yield* Fractal
      yield* Effect.forkChild(sdk.followWindow({
        _tag: 'Window', collection: 'attention', limit: 100, attentionThreadHints: true,
      }).pipe(Stream.runDrain))
      yield* settle
      expect(commands).toHaveLength(1)
      expect(commands[0]).toMatchObject({ kind: 'subscribe', collection: 'attention', limit: 100 })
      if (granted) expect(commands[0]).toHaveProperty('attention_thread_hints', true)
      else expect(commands[0]).not.toHaveProperty('attention_thread_hints')
    }).pipe(Effect.provide(layer), Effect.scoped)
  })
}
