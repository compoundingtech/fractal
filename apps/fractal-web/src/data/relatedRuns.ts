import { ClientError, FractalClient, type ClientOptions } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Effect, Fiber } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { unavailable, waiting, type Feed } from './source.ts'

/** Demand-owned GETs; the source's frame commits invalidate all mounted agent reads. */
export const relatedRunsFeed = ({ options, discovery, registry, epoch = Atom.make(0), retain, refusal }: {
  readonly options: ClientOptions
  readonly discovery: FractalClient
  readonly registry: AtomRegistry.AtomRegistry
  readonly epoch?: Atom.Atom<number>
  readonly retain?: (get: Atom.AtomContext) => void
  readonly refusal?: Atom.Atom<string | undefined>
}): ((ref: string) => Atom.Atom<Feed<Native.AgentRelatedRuns>>) => Atom.family((ref: string) => {
  let latest: Feed<Native.AgentRelatedRuns> = waiting
  return Atom.make((get): Feed<Native.AgentRelatedRuns> => {
    const revision = get(epoch)
    const denied = refusal === undefined ? undefined : get(refusal)
    retain?.(get)
    let active = true
    if (denied !== undefined) {
      latest = unavailable({ reason: 'ungranted', detail: denied })
      return latest
    }
    if (latest._tag === 'Observed') latest = { ...latest, freshness: 'stale', error: undefined }
    const fiber = Effect.runFork(Effect.gen(function* () {
      // Multiple invalidations in one frame share one read, without a timer or poller.
      yield* Effect.yieldNow
      const result = yield* Effect.tryPromise({
        try: async (signal) => {
          const client = new FractalClient({ ...options, fetchImpl: (input, init) => {
            signal.throwIfAborted()
            return (options.fetchImpl ?? globalThis.fetch)(input, { ...init, signal })
          } })
          client.discover = () => discovery.discover()
          const response = await client.agentRelatedRunsGet(ref)
          signal.throwIfAborted()
          return Native.decodeUnknownSync(Native.AgentRelatedRuns)(response.value)
        },
        catch: (cause) => ({
          reason: cause instanceof ClientError && cause.status === 403 ? 'ungranted' as const
            : cause instanceof ClientError && (cause.status === 501 || cause.response.code === 'unsupported-capability') ? 'unsupported' as const : 'failed' as const,
          detail: cause instanceof Error ? cause.message : String(cause),
        }),
      }).pipe(Effect.result)
      if (!active || registry.get(epoch) !== revision) return
      if (result._tag === 'Success') latest = { _tag: 'Observed', value: result.success, freshness: 'live' }
      else latest = latest._tag === 'Observed' && result.failure.reason !== 'ungranted'
        ? { ...latest, freshness: 'stale', error: result.failure }
        : unavailable(result.failure)
      get.setSelf(latest)
    }))
    get.addFinalizer(() => { active = false; Effect.runFork(Fiber.interrupt(fiber)) })
    return latest
  })
})
