import { it } from '@effect/vitest'
import type { Capabilities, EnvelopeOf } from '@fractal/client'
import { Deferred, Effect } from 'effect'
import { describe, expect } from 'vitest'

import { Fractal, FractalLive } from './mod.ts'

const response: EnvelopeOf<Capabilities> = {
  api_version: 'st3.client.v0', request_id: 'request/discovery',
  snapshot: { id: 'snapshot/discovery', host_id: 'host/example', store_index: 1,
    projection_version: 'client-projection.v0', created_at: '2026-11-17T00:00:00Z' },
  value: { kind: 'capabilities', capabilities: [], limits: { max_page_items: 100, max_event_items: 100, max_wait_ms: 1000, max_response_bytes: 1048576 },
    machine_version: 'test', session_actor: 'person/example', transport: 'local-web',
    event_cursor: 'cursor/discovery', oldest_event_cursor: 'cursor/discovery', schemas: [],
    issuer: 'person/example', issuer_key: 'issuer-key/example' },
}

describe('channel discovery observers', () => {
  for (const fails of [false, true]) {
    it.live(`forwards the existing probe ${fails ? 'failure' : 'canonical envelope'} without an additional read`, () => Effect.gen(function* () {
      const observed = yield* Deferred.make<EnvelopeOf<Capabilities> | undefined>()
      let reads = 0
      yield* Effect.gen(function* () {
        yield* Fractal
        expect(yield* Deferred.await(observed)).toEqual(fails ? undefined : response)
        expect(reads).toBe(1)
      }).pipe(Effect.provide(FractalLive({
        baseUrl: 'http://gateway.test', maxFollows: 2,
        fetch: async () => { reads += 1; if (fails) throw new TypeError('Offline'); return Response.json(response) },
        socket: () => ({ onopen: null, onmessage: null, onerror: null, onclose: null, send: () => {}, close: () => {} }),
        onCapabilities: envelope => { Effect.runFork(Deferred.succeed(observed, envelope)) },
        onProbeFailure: () => { Effect.runFork(Deferred.succeed(observed, undefined)) },
      })))
    }))

    it.live(`isolates a throwing observer from ${fails ? 'backoff and failed admission' : 'socket opening and successful admission'}`, () => Effect.gen(function* () {
      const progressed = yield* Deferred.make<void>()
      let reads = 0
      let sockets = 0
      yield* Effect.gen(function* () {
        const sdk = yield* Fractal
        yield* Deferred.await(progressed)
        const admission = yield* Effect.result(sdk.readAgents)
        expect(admission._tag).toBe(fails ? 'Failure' : 'Success')
        expect(reads).toBe(1)
        expect(sockets).toBe(fails ? 0 : 1)
      }).pipe(Effect.provide(FractalLive({
        baseUrl: 'http://gateway.test', maxFollows: 2, conversationSlots: 'advertised',
        fetch: async input => {
          if (String(input).includes('/capabilities')) {
            reads += 1
            if (fails) throw new TypeError('Offline')
            return Response.json(response)
          }
          return Response.json({ ...response, value: { items: [], page: { has_more: false } } })
        },
        socket: () => {
          sockets += 1
          Effect.runFork(Deferred.succeed(progressed, undefined))
          return { onopen: null, onmessage: null, onerror: null, onclose: null, send: () => {}, close: () => {} }
        },
        onCapabilities: () => { throw new Error('Observer failed') },
        onProbeFailure: () => { throw new Error('Observer failed') },
        onDiagnostics: event => {
          if (event._tag === 'Connection' && event.state._tag === 'Reconnecting') {
            expect(event.state.nextAt).toBeDefined()
            Effect.runFork(Deferred.succeed(progressed, undefined))
          }
        },
      })))
    }))
  }
})
