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
    let socketCount = 0
    let closes = 0
    let socketAuthorization: string | undefined
    const socket: CollectionSocket = {
      onopen: null, onmessage: null, onclose: null, onerror: null,
      send: (text) => { commands.push(JSON.parse(text)) },
      close: () => { closes += 1 },
    }
    const factory: CollectionSocketFactory = (_url, _protocols, headers) => {
      socketCount += 1
      socketAuthorization = headers.Authorization
      queueMicrotask(() => socket.onopen?.())
      return socket
    }
    const layer = FractalLive({
      baseUrl: 'http://gateway.test', maxFollows: 2,
      credential: () => 'paired-test-credential', client: 'sdk-node-test', socket: factory,
      fetch: async (_input, init) => {
        httpAuthorization.push(new Headers(init?.headers).get('authorization'))
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
        yield* settle
        expect(socketCount).toBe(1)
        expect(socketAuthorization).toBe('Bearer paired-test-credential')
        expect(httpAuthorization).toContain('Bearer paired-test-credential')
        expect(commands.filter((command) => command.kind === 'subscribe').map((command) => command.collection)).toEqual(['agents', 'missions'])
        expect(followKey(descriptor.spec)).not.toBe(followKey({ ...descriptor.spec, descriptor: 'another-decoder' }))
        const subscription = commands.find((command) => command.collection === 'missions')!
        socket.onmessage?.({ data: JSON.stringify({
          kind: 'snapshot', id: subscription.id, collection: 'missions', has_more: false,
          items: [{ id: 'mission/test', revision: '1' }], order: ['mission/test'], snapshot,
        }) })
        // Ingest and Effect publication complete on separate scheduler turns.
        yield* Effect.promise(async () => {
          await expect.poll(() => identities, { timeout: 2_000 }).toEqual(['mission/test'])
        })
        yield* Fiber.interrupt(first)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(1)
        yield* Fiber.interrupt(second)
        yield* settle
        expect(commands.filter((command) => command.kind === 'unsubscribe')).toHaveLength(2)
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
