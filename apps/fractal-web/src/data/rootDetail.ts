import type { TraceFact } from '@fractal/ui/assistant-ui'
import type { DetailsSupplement, DetailsSupplementQuery } from '../extensions/contract.ts'
import type { Feed } from './source.ts'
import { Duration, Effect, Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'

/**
 * Descendant-inclusive spend of an agent's current native root, read from the web server's
 * root-detail relay (`server/rootDetail.mts`). A source declares the relay; without a declaration
 * the Details pane requests nothing.
 */
export type RootDetailWindow = '1d' | '7d'
export const rootDetailPath = '/v1/client/usage/root-detail'

export interface RootDetailSource {
  /** Absolute or same-origin URL of the relay route. */
  readonly url: string
  /** The source's credentialed transport; the relay verifies the same credential the gateway would see. */
  readonly fetch: typeof globalThis.fetch
  /** Reported window shown in Details. */
  readonly window: RootDetailWindow
  /** Refresh cadence while a Details pane shows the root. */
  readonly refreshMs: number
}

/** The relay at `${origin}${rootDetailPath}`; 7-day window and 30-second refresh unless declared otherwise. */
export const relayRootDetailSource = ({ origin = '', fetchImpl, window = '7d', refreshMs = 30_000 }: {
  readonly origin?: string
  readonly fetchImpl: typeof globalThis.fetch
  readonly window?: RootDetailWindow
  readonly refreshMs?: number
}): RootDetailSource => ({ url: `${origin}${rootDetailPath}`, fetch: fetchImpl, window, refreshMs })

const Text = Schema.NonEmptyString.check(Schema.isMaxLength(512))
const Amount = Schema.Finite.check(Schema.isGreaterThanOrEqualTo(0))
const Count = Schema.Natural.check(Schema.isLessThanOrEqualTo(Number.MAX_SAFE_INTEGER))
const optional = <S extends Schema.Top>(schema: S) => Schema.optionalKey(Schema.NullOr(schema))

/** The relay's 200 answer; it renders at most 20 descendants and nothing outside this allowlist. */
export const RootDetailObservation = Schema.Struct({
  agent: Text,
  incarnationId: Text,
  host: Text,
  nativeSessionId: Text,
  window: Schema.Literals(['1d', '7d']),
  inclusiveUsd: optional(Amount),
  childCount: optional(Count),
  children: Schema.optionalKey(Schema.Array(Schema.Struct({
    sessionId: Text,
    role: optional(Text),
    model: optional(Text),
    usd: Amount,
    requests: Count,
  })).check(Schema.isMaxLength(20))),
  attributedFraction: optional(Schema.Finite.check(Schema.isBetween({ minimum: 0, maximum: 1 }))),
  wallHours: optional(Amount),
  activeHours: optional(Amount),
  producerHost: optional(Text),
})
export type RootDetailObservation = typeof RootDetailObservation.Type

const unknown = <T,>(detail: string): TraceFact<T> => ({ _tag: 'Unknown', reason: 'not-covered', detail })
const known = <T,>(value: T): TraceFact<T> => ({ _tag: 'Known', value, source: 'series' })
const windowLabels: Record<RootDetailWindow, string> = { '1d': 'last 24 hours', '7d': 'last 7 days' }

/** Settled without a read: still acquiring, a harness the relay never covers, or no verified root. */
export const detailsRequestEligibility = (query: DetailsSupplementQuery): Feed<DetailsSupplement> | undefined => {
  // A known non-omp harness stays not-covered even while native facts acquire.
  if (query.nativeRootPending === true && (query.harness === undefined || query.harness === 'omp')) return { _tag: 'Waiting' }
  if (query.harness !== 'omp') return { _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-not-covered', detail: 'Descendant usage is not covered for this harness' }
  if (query.nativeSessionId === undefined || query.incarnationId === undefined || query.host === undefined)
    return { _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-not-attributed', detail: 'No verified current native root is available' }
  return undefined
}

/** Descendant-inclusive dollars for the reported window; never native main-session dollars, tokens or lifetime time. */
export const projectRootDetail = (query: DetailsSupplementQuery, observation: RootDetailObservation): Feed<DetailsSupplement> => {
  if (observation.agent !== query.agentRef || observation.nativeSessionId !== query.nativeSessionId || observation.incarnationId !== query.incarnationId || observation.host !== query.host)
    return { _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-not-attributed', detail: 'Root detail does not match the current native identity' }
  const window = known({ _tag: 'Reported' as const, label: `${windowLabels[observation.window]} (as reported; exact cut bounds and as-of unknown)` })
  const timeLabel = `usage ledger${observation.producerHost == null ? '; ledger host unreported' : `; reported ledger host ${observation.producerHost}`}; native owner ${observation.host}; reported ${observation.window}, cut bounds unknown`
  const durations = [
    ...(observation.wallHours == null ? [] : [{ label: `Root observed wall span (${timeLabel})`, value: `${observation.wallHours} h` }]),
    ...(observation.activeHours == null ? [] : [{ label: `Root active estimate (${timeLabel}; gaps ≤10 min)`, value: `${observation.activeHours} h` }]),
  ]
  return {
    _tag: 'Observed', freshness: 'live', coverage: { _tag: 'Partial' },
    value: {
      nativeSessionId: observation.nativeSessionId,
      totalUsd: observation.inclusiveUsd == null ? unknown('Root total was not reported') : known(observation.inclusiveUsd),
      usdWindow: window,
      // Window-wide monetary attribution, not completeness of this root.
      coverage: observation.attributedFraction == null ? { _tag: 'Unavailable', reason: 'not-attributed' } : { _tag: 'Partial', attributedFraction: observation.attributedFraction },
      subagents: observation.children === undefined ? unknown('Traced descendants were not reported') : known(observation.children.map((child) => ({
        sessionId: child.sessionId,
        parentSessionId: unknown<string>('Producer does not report descendant parent identity'),
        label: child.role == null || child.role === '.unknown' ? unknown<string>('Descendant role was not reported') : known(child.role),
        model: child.model == null || child.model === '.unknown' ? unknown<string>('Descendant model was not reported') : known(child.model),
        usd: child.usd,
        requests: child.requests,
        firstMs: unknown<number>('Producer does not report descendant first-request timestamps'),
        lastMs: unknown<number>('Producer does not report descendant last-request timestamps'),
      }))),
      subagentsWindow: window,
      subagentCount: observation.childCount == null ? unknown('Total traced descendant count was not reported') : known(observation.childCount),
      durations,
    },
  }
}

const Failure = Schema.Struct({ code: Schema.String, message: Schema.String })
/** A 404 the relay did not write: the server has no producer configured, so the route reached the gateway. */
const unconfigured: Feed<never> = { _tag: 'Unavailable', reason: 'unsupported', code: 'root-detail-unconfigured', detail: 'This server reports no descendant usage' }
const unreadable: Feed<never> = { _tag: 'Unavailable', reason: 'failed', detail: 'Root detail relay could not be read' }
const decodeJson = Schema.decodeUnknownSync(Schema.fromJsonString(Schema.Unknown))
const decodeFailure = Schema.decodeUnknownSync(Failure)
const decodeObservation = Schema.decodeUnknownSync(RootDetailObservation)

const readRootDetail = (source: RootDetailSource, query: DetailsSupplementQuery, now: () => number) => Effect.tryPromise({
  try: async (signal): Promise<Feed<DetailsSupplement>> => {
    const response = await source.fetch(`${source.url}?${new URLSearchParams({ agent: query.agentRef, window: source.window })}`, { credentials: 'same-origin', headers: { accept: 'application/json' }, signal })
    const text = await response.text()
    if (text.length > 256 * 1024) return { _tag: 'Unavailable', reason: 'failed', detail: 'Root detail answer exceeded its size limit' }
    if (response.status === 404) {
      const failure = Schema.decodeUnknownOption(Schema.fromJsonString(Failure))(text)
      if (failure._tag === 'None' || failure.value.code !== 'root-detail-not-found') return unconfigured
    }
    const payload = decodeJson(text)
    if (!response.ok) {
      const failure = decodeFailure(payload)
      return { _tag: 'Unavailable', reason: response.status === 403 ? 'ungranted' : response.status === 422 || response.status === 404 ? 'unsupported' : 'failed', code: failure.code, detail: failure.message }
    }
    const projected = projectRootDetail(query, decodeObservation(payload))
    return projected._tag === 'Observed' ? { ...projected, value: { ...projected.value, receivedAtMs: now() } } : projected
  },
  catch: () => unreadable,
}).pipe(
  // The relay verifies the root with three gateway reads before and after one producer read,
  // each bounded at 30 s; this allows all seven plus transport overhead.
  Effect.timeout('215 seconds'),
  Effect.catch(() => Effect.succeed<Feed<DetailsSupplement>>(unreadable)),
)

/** Visible demand owns refresh and cancellation; `remember` keeps the last feed across remounts. */
export const createDetailsSupplementAtom = ({ source, query, remember, isUnconfigured = () => false, now = Date.now }: {
  readonly source: RootDetailSource
  readonly query: DetailsSupplementQuery
  readonly remember?: (feed: Feed<DetailsSupplement>) => void
  /** Shared across a source's atoms: once one read finds no relay, none reads again. */
  readonly isUnconfigured?: () => boolean
  readonly now?: () => number
}): Atom.Atom<Feed<DetailsSupplement>> => {
  const ineligible = detailsRequestEligibility(query)
  if (ineligible !== undefined) return Atom.make(ineligible)
  let latest: Feed<DetailsSupplement> = { _tag: 'Waiting' }
  return Atom.make((get): Feed<DetailsSupplement> => {
    let active = true
    const fiber = Effect.runFork(Effect.gen(function* () {
      while (true) {
        const feed: Feed<DetailsSupplement> = isUnconfigured() ? unconfigured : yield* readRootDetail(source, query, now)
        yield* Effect.sync(() => {
          if (!active) return
          // A transient failure keeps this exact identity's last answer as stale; denial or mismatch replaces it.
          latest = feed._tag === 'Unavailable' && feed.reason === 'failed' && latest._tag === 'Observed'
            ? { ...latest, freshness: 'stale', error: { reason: 'failed', detail: feed.detail } }
            : feed
          remember?.(latest)
          get.setSelf(latest)
        })
        if (feed === unconfigured) return
        yield* Effect.sleep(Duration.millis(source.refreshMs))
      }
    }))
    get.addFinalizer(() => { active = false; fiber.interruptUnsafe() })
    return latest._tag === 'Observed' ? { ...latest, freshness: 'stale' } : latest
  }).pipe(Atom.autoDispose)
}

export type DetailsSupplementCache = (query: DetailsSupplementQuery) => Atom.Atom<Feed<DetailsSupplement>>

/** One cache per source: up to 64 roots; a paused native fence reads the matching entry but never mounts its read. */
export const createDetailsSupplementCache = (source: RootDetailSource, now: () => number = Date.now): DetailsSupplementCache => {
  const roots = new Map<string, { readonly atom: Atom.Atom<Feed<DetailsSupplement>>; feed: Feed<DetailsSupplement> }>()
  let absent = false
  const isUnconfigured = () => absent
  return (query) => {
    const ineligible = detailsRequestEligibility(query)
    if (ineligible !== undefined) return Atom.make(ineligible)
    if (absent) return Atom.make(unconfigured)
    const { canRefresh, nativeRootPending: _nativeRootPending, ...identity } = query
    const key = JSON.stringify(identity)
    let entry = roots.get(key)
    if (entry === undefined) {
      const created: { readonly atom: Atom.Atom<Feed<DetailsSupplement>>; feed: Feed<DetailsSupplement> } = {
        feed: { _tag: 'Waiting' },
        atom: createDetailsSupplementAtom({
          source, query: identity, now, isUnconfigured,
          remember: (feed) => {
            created.feed = feed
            if (feed === unconfigured) absent = true
          },
        }),
      }
      entry = created
      if (roots.size >= 64) roots.delete(roots.keys().next().value!)
      roots.set(key, entry)
    }
    if (canRefresh !== false) return entry.atom
    return Atom.make<Feed<DetailsSupplement>>(entry.feed._tag === 'Observed'
      ? { ...entry.feed, freshness: 'stale', error: { reason: 'failed', detail: 'Native root evidence is stale; descendant reads are paused.' } }
      : entry.feed._tag === 'Unavailable' ? entry.feed : { _tag: 'Unavailable', reason: 'failed', detail: 'Current native root facts are required before reading descendants.' })
  }
}
