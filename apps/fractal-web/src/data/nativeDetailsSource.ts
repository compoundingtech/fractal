import { ClientError, FractalClient, type ClientOptions } from '@fractal/client'
import * as Native from '@fractal/client/schema'
import { Effect, Fiber, Schedule, Stream } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import type * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { observed, unavailable, waiting, type Feed } from './source.ts'
import { projectNativeDeclarations, type NativeDeclarations } from './nativeDeclarations.ts'

export interface NativeDetailsSource {
  /**
   * The bounded spend period, read once per mounted Details agent: no timer re-reads it, hidden
   * Details holds no demand, and each open or agent switch starts one fresh read.
   */
  readonly usagePeriod: (agentRef: string) => Atom.Atom<Feed<Native.UsagePeriod>>
  /** Latest redacted declaration; revision keys the agent observation, not the independent declaration history. */
  readonly rawDeclarations: (query: { readonly agentRef: string; readonly revision: string }) => Atom.Atom<Feed<Native.AgentDeclaration>>
  /** Public-safe categories mapped from the same raw getter, never a second read. */
  readonly declarations: (query: { readonly agentRef: string; readonly revision: string }) => Atom.Atom<Feed<NativeDeclarations>>
  /** Refreshes the getter after read or projection failure so the new value is re-projected; refused reads stay refused. */
  readonly retry: (atom: Atom.Atom<Feed<unknown>>) => void
  /** One requested re-read of a settled getter, healthy or failed; refused and unsupported reads stay as they are. */
  readonly refresh: (atom: Atom.Atom<Feed<unknown>>) => void
}
const failure = (error: unknown): Feed<never> => unavailable({
  reason: error instanceof ClientError && error.status === 403 ? 'ungranted'
    : error instanceof ClientError && (error.status === 404 || error.status === 501) ? 'unsupported' : 'failed',
  detail: 'These session details could not be loaded.',
})

/** Reuse the source registry, SDK codecs and getter lifecycle used by SubjectReads. */
export const gatewayNativeDetails = ({ options, discovery, registry, now }: {
  readonly options: ClientOptions
  readonly discovery: FractalClient
  readonly registry: AtomRegistry.AtomRegistry
  readonly now: () => number
}): NativeDetailsSource => {
  /** `polled` re-reads every 15 seconds while mounted; otherwise the getter reads once per mount or refresh. */
  const reader = <T>(read: (client: FractalClient) => Promise<T>, { polled }: { readonly polled: boolean }): Atom.Atom<Feed<T>> => {
    let retained: T | undefined
    return Atom.make((get): Feed<T> => {
      const readOnce = Effect.tryPromise({
        try: async (signal) => {
          const client = new FractalClient({ ...options, fetchImpl: (input, init) => {
            signal.throwIfAborted()
            return (options.fetchImpl ?? globalThis.fetch)(input, { ...init, signal })
          } })
          // Keep capability discovery shared with all other native readers.
          client.discover = () => discovery.discover()
          const value = await read(client)
          signal.throwIfAborted()
          return value
        },
        catch: failure,
      }).pipe(Effect.match({
        onSuccess: (value): Feed<T> => { retained = value; return observed({ value }) },
        onFailure: (error): Feed<T> => {
          if (error._tag === 'Unavailable' && error.reason === 'ungranted') retained = undefined
          return retained === undefined || error._tag !== 'Unavailable'
            ? error : { _tag: 'Observed', value: retained, freshness: 'stale', error: { reason: error.reason, detail: error.detail } }
        },
      }))
      const fiber = Effect.runFork((polled ? Stream.fromEffectSchedule(readOnce, Schedule.spaced('15 seconds')) : Stream.fromEffect(readOnce))
        .pipe(Stream.runForEach(value => Effect.sync(() => get.setSelf(value)))))
      get.addFinalizer(() => Effect.runFork(Fiber.interrupt(fiber)))
      return retained === undefined ? waiting : observed({ value: retained, freshness: 'stale' })
    })
  }
  // Keyed by the visible agent so each Details open or agent switch reads once; the period is not polled.
  const usagePeriod = Atom.family((_agentRef: string) => reader(async client => {
    const until_ms = now()
    const response = await client.usagePeriod({ since_ms: Math.max(0, until_ms - 604_800_000), until_ms })
    return Native.decodeUnknownSync(Native.UsagePeriod)(response.value)
  }, { polled: false }))
  const rawDeclarations = Atom.family((key: string) => reader(async client => {
    const [agentRef, revision] = key.split('\n')
    if (agentRef === undefined || revision === undefined) throw new TypeError('Invalid declaration query')
    const response = await client.agentDeclarationGet(agentRef, undefined, false)
    const declaration = Native.decodeUnknownSync(Native.AgentDeclaration)(response.value)
    if (declaration.id !== agentRef) throw new TypeError('Mismatched declaration observation')
    return declaration
  }, { polled: true }))
  const sourceOf = new WeakMap<Atom.Atom<Feed<unknown>>, Atom.Atom<Feed<unknown>>>()
  const declarations = Atom.family((key: string) => {
    const raw = rawDeclarations(key)
    const projected = Atom.map(raw, (feed): Feed<NativeDeclarations> => {
      if (feed._tag !== 'Observed') return feed
      const value = projectNativeDeclarations({ declaration: feed.value })
      return value === undefined ? unavailable({ reason: 'failed', detail: 'The agent’s settings could not be read.' }) : { ...feed, value }
    })
    sourceOf.set(projected, raw)
    return projected
  })
  const settled = (atom: Atom.Atom<Feed<unknown>>): boolean => {
    const state = registry.get(atom)
    return (state._tag === 'Unavailable' && state.reason === 'failed') ||
      (state._tag === 'Observed' && (state.freshness === 'live' || state.error?.reason === 'failed'))
  }
  return { usagePeriod,
    rawDeclarations: ({ agentRef, revision }) => rawDeclarations(`${agentRef}\n${revision}`),
    declarations: ({ agentRef, revision }) => declarations(`${agentRef}\n${revision}`),
    retry: atom => {
      const target = sourceOf.get(atom) ?? atom
      // The public projection may reject a successful raw read; retryability belongs to the requested feed.
      const state = registry.get(atom)
      if ((state._tag === 'Unavailable' && state.reason === 'failed') ||
        (state._tag === 'Observed' && state.error?.reason === 'failed')) registry.refresh(target)
    },
    // An in-flight read (waiting, or retained without an error) is not restarted.
    refresh: atom => { if (settled(atom)) registry.refresh(sourceOf.get(atom) ?? atom) },
  }
}
