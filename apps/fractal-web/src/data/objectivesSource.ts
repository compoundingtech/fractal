import { ClientError, FractalClient, type ClientOptions } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Effect, Fiber } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { unavailable, waiting, type Feed } from './source.ts'

/** Member-owned native reads; no client-generated tones, forecasts or history samples. */
export interface ObjectivesSource {
  readonly list: Atom.Atom<Feed<Native.SloList>>
  readonly accounts: Atom.Atom<Feed<Native.UsagePeriod>>
  readonly history: (id: string) => Atom.Atom<Feed<Native.SloHistory>>
  readonly refresh: () => void
}

export const gatewayObjectives = ({ options, discovery, registry, now }: {
  readonly options: ClientOptions
  readonly discovery: FractalClient
  readonly registry: AtomRegistry.AtomRegistry
  readonly now: () => number
}): ObjectivesSource => {
  const reader = <T>(read: (client: FractalClient) => Promise<T>): Atom.Atom<Feed<T>> => {
    let previous: Feed<T> = waiting
    return Atom.make((get): Feed<T> => {
      const fiber = Effect.runFork(Effect.tryPromise({
        try: async signal => {
          const client = new FractalClient({ ...options, fetchImpl: (input, init) => {
            signal.throwIfAborted()
            return (options.fetchImpl ?? globalThis.fetch)(input, { ...init, signal })
          } })
          client.discover = () => discovery.discover()
          const value = await read(client)
          signal.throwIfAborted()
          return value
        },
        catch: error => unavailable({
          reason: error instanceof ClientError && error.status === 403 ? 'ungranted'
            : error instanceof ClientError && (error.status === 404 || error.status === 501) ? 'unsupported' : 'failed',
          detail: 'Could not read objectives and budgets.',
        }),
      }).pipe(Effect.match({
        onSuccess: (value): Feed<T> => ({ _tag: 'Observed', value, freshness: 'live', readAt: now() }),
        onFailure: (failure): Feed<T> => previous._tag === 'Observed' && failure._tag === 'Unavailable' && failure.reason !== 'ungranted'
          ? { ...previous, freshness: 'stale', error: { reason: failure.reason, detail: failure.detail } } : failure,
      }), Effect.tap(value => Effect.sync(() => { previous = value; get.setSelf(value) }))))
      get.addFinalizer(() => Effect.runFork(Fiber.interrupt(fiber)))
      return previous._tag === 'Observed' ? { ...previous, freshness: 'stale' } : waiting
    })
  }
  const list = reader(async client => Native.decodeUnknownSync(Native.SloList)((await client.sloList()).value))
  const accounts = reader(async client => {
    const until_ms = now()
    return Native.decodeUnknownSync(Native.UsagePeriod)((await client.usagePeriod({ since_ms: Math.max(0, until_ms - 86_400_000), until_ms })).value)
  })
  const charts = new Map<string, Atom.Atom<Feed<Native.SloHistory>>>()
  const history = (id: string) => {
    let atom = charts.get(id)
    if (atom === undefined) {
      atom = reader(async client => {
        const until_ms = now()
        const result = await client.sloHistory(id, { since_ms: Math.max(0, until_ms - 86_400_000), until_ms, steps: 48 })
        const value = Native.decodeUnknownSync(Native.SloHistory)(result.value)
        if (value.id !== id) throw new TypeError('Mismatched objective history')
        return value
      })
      charts.set(id, atom)
    }
    return atom
  }
  return { list, accounts, history, refresh: () => {
    registry.refresh(list)
    registry.refresh(accounts)
    for (const atom of charts.values()) registry.refresh(atom)
  } }
}
