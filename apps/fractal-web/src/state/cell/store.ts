import { Context, Effect, Layer, Option, Queue, Schema, Stream } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'

import { CellEnvironment, type StorageChange } from './environment.ts'

/** Pre-paint fields: a flat projection the inline boot script reads without knowing any schema. */
export type BootFields = Readonly<Record<string, string | number | boolean>>

/**
 * - `boot`: shared across windows, and projected into `wf.boot@1` for the first frame.
 * - `shared`: shared across windows of the origin.
 * - `window`: this window only (sessionStorage); survives reload, not a new window.
 * - `sensitive`: shared, person-scoped, bounded, and removed by `clearScope('person', …)`.
 */
export type Tier = 'boot' | 'shared' | 'window' | 'sensitive'

/** Everything the store needs about one stored value, with its storage key already resolved. */
export interface CellBindingSpec<A, I> {
  readonly key: string
  readonly schema: Schema.Codec<A, I>
  readonly version: number
  readonly migrate?: ((value: unknown, fromVersion: number) => unknown) | undefined
  readonly default: A
  readonly tier: Tier
  readonly boot?: ((value: A) => BootFields) | undefined
  /** Keep only the `max` most recently written keys under `prefix`; also the eviction pool under quota. */
  readonly retain?: { readonly prefix: string; readonly max: number } | undefined
  /** Supplies a first value when nothing is stored, e.g. from a retired storage format. Written once. */
  readonly seed?: (() => A | undefined) | undefined
}

export interface CellBinding<A> {
  readonly read: () => A
  readonly write: (value: A) => void
  readonly subscribe: (listener: () => void) => () => void
}

export type CellIssue =
  | { readonly _tag: 'Corrupt'; readonly key: string }
  | { readonly _tag: 'FutureVersion'; readonly key: string; readonly version: number }
  | { readonly _tag: 'WriteFailed'; readonly key: string; readonly cause: unknown }

/** Shared keys live under `wf.c/`, window keys under `wf.w/`, so scans and clears never see foreign keys. */
export const localRoot = 'wf.c/'
export const sessionRoot = 'wf.w/'
/** The projection the inline boot script reads. Its fields are a stable contract with `index.html`. */
export const bootKey = 'wf.boot@1'
/** A revoke in one window: others drop unsaved values under the prefix instead of writing them back. */
export const clearKey = 'wf.clear@1'

const Envelope = Schema.fromJsonString(Schema.Struct({ v: Schema.Int, t: Schema.Finite, w: Schema.String, d: Schema.Unknown }))
type Envelope = typeof Envelope.Type
const decodeEnvelope = Schema.decodeUnknownOption(Envelope)
const encodeEnvelope = Schema.encodeSync(Envelope)
const ClearMarker = Schema.fromJsonString(Schema.Struct({ prefix: Schema.String, t: Schema.Finite, w: Schema.String }))
const decodeClear = Schema.decodeUnknownOption(ClearMarker)
const BootJson = Schema.fromJsonString(Schema.Record(Schema.String, Schema.Union([Schema.String, Schema.Finite, Schema.Boolean])))
const decodeBoot = Schema.decodeUnknownOption(BootJson)

interface Entry {
  envelope: Envelope
  dirty: boolean
  /** Not yet in storage: its first write may push a bounded family over `retain.max`. */
  fresh: boolean
  readonly area: 'local' | 'session'
  readonly retain: CellBindingSpec<unknown, unknown>['retain']
}

export interface CellStoreShape {
  readonly bind: <A, I>(spec: CellBindingSpec<A, I>) => CellBinding<A>
  /** Writes every pending value now. Runs on its own after `debounceMs` and on pagehide. */
  readonly flush: () => void
  /** Removes stored and live values under a key prefix in every window: the revoke path. */
  readonly clearPrefix: (prefix: string) => void
  readonly issues: () => readonly CellIssue[]
}

export class CellStore extends Context.Service<CellStore, CellStoreShape>()('fractal-web/state/CellStore') {}

const run = <A>(effect: Effect.Effect<A, KeyValueStore.KeyValueStoreError>) => Effect.runSync(Effect.result(effect))

/** Builds the store over a `CellEnvironment`. Reads are synchronous so restored state paints on the first frame. */
export const make = ({ debounceMs = 150, writer = globalThis.crypto.randomUUID() }: {
  readonly debounceMs?: number
  readonly writer?: string
} = {}) => Effect.gen(function* () {
  const environment = yield* CellEnvironment
  const local = KeyValueStore.prefix(environment.local, localRoot)
  const session = KeyValueStore.prefix(environment.session, sessionRoot)
  const areas = { local, session } as const
  const issues: CellIssue[] = []
  const live = new Map<string, Entry>()
  const listeners = new Map<string, Set<() => void>>()
  const bindings = new Map<string, CellBinding<unknown>>()
  let clock = 0
  const tick = () => (clock = Math.max(Date.now(), clock + 1))
  const notify = (key: string) => listeners.get(key)?.forEach((listener) => listener())
  const issue = (found: CellIssue) => {
    issues.push(found)
    if (issues.length > 100) issues.shift()
  }

  // The projection holds the fields of every boot cell this window has read, merged over what is stored.
  const storedBoot = (): Record<string, string | number | boolean> => {
    const raw = run(environment.local.get(bootKey))
    if (raw._tag === 'Failure' || raw.success === undefined) return {}
    const decoded = decodeBoot(raw.success)
    return Option.isSome(decoded) ? { ...decoded.value } : {}
  }
  const projection: Record<string, string | number | boolean> = storedBoot()
  let projectionDirty = false
  const project = (fields: BootFields) => {
    for (const [field, value] of Object.entries(fields)) {
      if (projection[field] === value) continue
      projection[field] = value
      projectionDirty = true
    }
  }

  // A queue, not a ref: writes made before the flushing fiber first runs must still schedule a flush.
  const pending = yield* Queue.unbounded<void>()
  const schedule = () => { Queue.offerUnsafe(pending, undefined) }

  const familyAges = (prefix: string) => {
    const found: { readonly key: string; readonly t: number; readonly area: 'local' | 'session' }[] = []
    for (const area of ['local', 'session'] as const) {
      const keys = run(areas[area].keys)
      if (keys._tag === 'Failure') continue
      for (const key of keys.success) {
        if (!key.startsWith(prefix)) continue
        const raw = run(areas[area].get(key))
        const envelope = raw._tag === 'Success' ? decodeEnvelope(raw.success) : Option.none()
        found.push({ key, area, t: Option.isSome(envelope) ? envelope.value.t : 0 })
      }
    }
    return found.sort((left, right) => right.t - left.t)
  }
  const remove = (key: string, area: 'local' | 'session') => {
    run(areas[area].remove(key))
    if (live.delete(key)) notify(key)
  }
  const trim = ({ prefix, max }: { readonly prefix: string; readonly max: number }) => {
    for (const stale of familyAges(prefix).slice(max)) remove(stale.key, stale.area)
  }
  /** Quota: give up the oldest other member of the value's family, then retry; never another family's data. */
  const evictOldest = (key: string, entry: Entry) => {
    if (entry.retain === undefined) return false
    const oldest = familyAges(entry.retain.prefix).filter((member) => member.key !== key).at(-1)
    if (oldest === undefined) return false
    remove(oldest.key, oldest.area)
    return true
  }
  const newer = (left: Envelope, right: Envelope) => left.t > right.t || (left.t === right.t && left.w > right.w)
  const save = (key: string, entry: Entry) => {
    // Storage events arrive asynchronously: another window's later write may not have reached us yet.
    const raw = run(areas[entry.area].get(key))
    const stored = raw._tag === 'Success' && raw.success !== undefined ? decodeEnvelope(raw.success) : Option.none()
    if (Option.isSome(stored) && newer(stored.value, entry.envelope)) {
      entry.envelope = stored.value
      entry.dirty = false
      entry.fresh = false
      clock = Math.max(clock, stored.value.t)
      notify(key)
      return
    }
    const encoded = encodeEnvelope(entry.envelope)
    for (;;) {
      const written = run(areas[entry.area].set(key, encoded))
      if (written._tag === 'Success') {
        entry.dirty = false
        if (entry.fresh && entry.retain !== undefined) trim(entry.retain)
        entry.fresh = false
        return
      }
      if (!evictOldest(key, entry)) {
        // The live value stays; the next flush retries.
        issue({ _tag: 'WriteFailed', key, cause: written.failure })
        return
      }
    }
  }
  const flush = () => {
    for (const [key, entry] of live) if (entry.dirty) save(key, entry)
    if (!projectionDirty) return
    // Merge: another window may own fields this one never read.
    const written = run(environment.local.set(bootKey, JSON.stringify({ ...storedBoot(), ...projection })))
    if (written._tag === 'Success') projectionDirty = false
    else issue({ _tag: 'WriteFailed', key: bootKey, cause: written.failure })
  }

  const dropPrefix = (prefix: string) => {
    for (const key of [...live.keys()]) {
      if (!key.startsWith(prefix)) continue
      live.delete(key)
      notify(key)
    }
  }
  const receive = ({ key, newValue }: StorageChange) => {
    if (key === null) {
      // Another window cleared the whole area.
      for (const [liveKey, entry] of [...live]) {
        if (entry.area !== 'local') continue
        live.delete(liveKey)
        notify(liveKey)
      }
      return
    }
    if (key === clearKey) {
      const marker = newValue === null ? Option.none() : decodeClear(newValue)
      if (Option.isSome(marker)) dropPrefix(marker.value.prefix)
      return
    }
    if (!key.startsWith(localRoot)) return
    const cellKey = key.slice(localRoot.length)
    const entry = live.get(cellKey)
    if (newValue === null) {
      if (entry !== undefined && entry.area === 'local') { live.delete(cellKey); notify(cellKey) }
      return
    }
    const remote = decodeEnvelope(newValue)
    if (Option.isNone(remote)) return
    clock = Math.max(clock, remote.value.t)
    // A window still showing the default loads the stored value on its next read.
    if (entry === undefined) return notify(cellKey)
    if (!newer(remote.value, entry.envelope)) return
    entry.envelope = remote.value
    entry.dirty = false
    entry.fresh = false
    notify(cellKey)
  }

  yield* Stream.fromQueue(pending).pipe(
    Stream.debounce(`${debounceMs} millis`),
    Stream.runForEach(() => Effect.sync(flush)),
    Effect.forkScoped,
  )
  yield* environment.pageHide.pipe(Stream.runForEach(() => Effect.sync(flush)), Effect.forkScoped)
  yield* environment.changes.pipe(Stream.runForEach((change) => Effect.sync(() => receive(change))), Effect.forkScoped)
  yield* Effect.addFinalizer(() => Effect.sync(flush))

  const bind = <A, I>(spec: CellBindingSpec<A, I>): CellBinding<A> => {
    const existing = bindings.get(spec.key)
    if (existing !== undefined) return existing as CellBinding<A>
    const area = spec.tier === 'window' ? 'session' : 'local'
    const store = areas[area]
    const key = spec.key
    const decode = Schema.decodeUnknownSync(spec.schema)
    const encode = Schema.encodeSync(spec.schema)
    const load = (): Envelope | undefined => {
      const raw = run(store.get(key))
      if (raw._tag === 'Failure' || raw.success === undefined) return undefined
      const envelope = decodeEnvelope(raw.success)
      if (Option.isNone(envelope)) { issue({ _tag: 'Corrupt', key }); return undefined }
      return envelope.value
    }
    const write = (value: A) => {
      const previous = live.get(key)
      live.set(key, {
        envelope: { v: spec.version, t: tick(), w: writer, d: encode(value) },
        dirty: true,
        fresh: previous === undefined ? load() === undefined : previous.fresh,
        area,
        retain: spec.retain,
      })
      if (spec.boot !== undefined) project(spec.boot(value))
      notify(key)
      schedule()
    }
    const read = (): A => {
      let envelope = live.get(key)?.envelope
      if (envelope === undefined) {
        envelope = load()
        if (envelope === undefined) {
          const seeded = spec.seed?.()
          if (seeded !== undefined) { write(seeded); return seeded }
          if (spec.boot !== undefined) project(spec.boot(spec.default))
          return spec.default
        }
        live.set(key, { envelope, dirty: false, fresh: false, area, retain: spec.retain })
      }
      try {
        let value: A
        if (envelope.v === spec.version) value = decode(envelope.d)
        // A newer client wrote this: show the default and leave its value alone.
        else if (envelope.v > spec.version) { issue({ _tag: 'FutureVersion', key, version: envelope.v }); return spec.default }
        else if (spec.migrate === undefined) throw new Error(`No migration for ${key} from version ${envelope.v}`)
        else { value = decode(spec.migrate(envelope.d, envelope.v)); write(value) }
        if (spec.boot !== undefined) project(spec.boot(value))
        return value
      } catch {
        issue({ _tag: 'Corrupt', key })
        return spec.default
      }
    }
    const binding: CellBinding<A> = {
      read,
      write,
      subscribe: (listener) => {
        const set = listeners.get(key) ?? new Set()
        listeners.set(key, set)
        set.add(listener)
        return () => { set.delete(listener) }
      },
    }
    bindings.set(key, binding as CellBinding<unknown>)
    return binding
  }

  return CellStore.of({
    bind,
    flush,
    issues: () => issues,
    clearPrefix: (prefix) => {
      run(local.clearPrefix(prefix))
      run(session.clearPrefix(prefix))
      dropPrefix(prefix)
      // Storage events report each removed key, but not values another window has not saved yet.
      run(environment.local.set(clearKey, JSON.stringify({ prefix, t: tick(), w: writer })))
    },
  })
})

export const layer = (options?: Parameters<typeof make>[0]): Layer.Layer<CellStore, never, CellEnvironment> =>
  Layer.effect(CellStore, make(options))
