import { ClientError, FractalClient, type ClientOptions } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Cause, Effect, Option } from 'effect'
import * as AsyncResult from 'effect/reactivity/AsyncResult'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { clientReadAuthorization, unavailable, waiting, type Feed } from './source.ts'

/** Member-owned native reads; no client-generated tones, forecasts or history samples. */
export interface ObjectivesSource {
  readonly list: Atom.Atom<Feed<Native.SloList>>
  readonly accounts: Atom.Atom<Feed<Native.UsagePeriod>>
  readonly history: (id: string) => Atom.Atom<Feed<Native.SloHistory>>
  readonly refresh: () => void
}

/** Objectives, budgets and their histories read from the gateway as retained feeds. */
export const gatewayObjectives = ({ options, discovery, registry, now }: {
  readonly options: ClientOptions
  readonly discovery: FractalClient
  readonly registry: AtomRegistry.AtomRegistry
  readonly now: () => number
}): ObjectivesSource => {
  // Atom runs the read and interrupts it (aborting the request) on unmount or refresh. Its state ends
  // with the atom's node, so the last reading is retained here: a remounted pane or a failed re-read
  // shows it as stale instead of waiting.
  const reader = <T>(read: (client: FractalClient) => Promise<T>): Atom.Atom<Feed<T>> => {
    let retained: { readonly value: T; readonly readAt: number } | undefined
    const stale = (error?: { readonly reason: 'unsupported' | 'failed'; readonly detail: string }): Feed<T> =>
      retained === undefined ? waiting
        : { _tag: 'Observed', value: retained.value, readAt: retained.readAt, freshness: 'stale', ...(error === undefined ? {} : { error }) }
    return Atom.make(Effect.tryPromise({
      try: async signal => {
        const client = new FractalClient({ ...options, fetchImpl: (input, init) => {
          signal.throwIfAborted()
          return (options.fetchImpl ?? globalThis.fetch)(input, { ...init, signal })
        } })
        client.discover = () => discovery.discover()
        return { value: await read(client), readAt: now() }
      },
      catch: error => unavailable({
        reason: clientReadAuthorization(error) ?? (error instanceof ClientError && (error.status === 404 || error.status === 501) ? 'unsupported' : 'failed'),
        detail: 'Could not read objectives and budgets.',
      }),
    })).pipe(Atom.map(AsyncResult.match({
      onInitial: () => stale(),
      onSuccess: ({ value: reading, waiting: refreshing }): Feed<T> => {
        retained = reading
        return refreshing === true ? stale() : { _tag: 'Observed', value: reading.value, readAt: reading.readAt, freshness: 'live' }
      },
      onFailure: ({ cause }): Feed<T> => {
        const failure = Option.getOrElse(Cause.findErrorOption(cause), () => unavailable({ reason: 'failed', detail: 'Could not read objectives and budgets.' }))
        if (failure._tag !== 'Unavailable') return failure
        if (failure.reason === 'ungranted' || failure.reason === 'unauthorized') {
          retained = undefined
          return failure
        }
        return retained === undefined ? failure : stale({ reason: failure.reason, detail: failure.detail })
      },
    })))
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
