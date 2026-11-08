import { Context, Effect, Exit, Layer, Option, Ref, Schedule, Schema, Scope, Stream, SubscriptionRef } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'
import { decodeWorkspace, type LegacyDecoder } from './codec.ts'
import { emptyWorkspace, spaceBody, transition, type Command, type WorkspaceConfig, type WorkspaceState } from './model.ts'
import { canonicalBody, type SpaceBody } from './wire.ts'

/** Where the document lives in the `KeyValueStore`, and word that another writer (a second window) changed it. */
export class WorkspacePersistence extends Context.Service<WorkspacePersistence, {
  readonly key: string
  readonly changed: Stream.Stream<void>
}>()('@overeng/workspace/WorkspacePersistence') {}

export type WorkspacePersistenceLayer = Layer.Layer<WorkspacePersistence | KeyValueStore.KeyValueStore>

/** In memory, optionally seeded with a stored document's text. */
export const memoryPersistence = (initial?: string, key = 'workspace'): WorkspacePersistenceLayer => Layer.mergeAll(
  Layer.succeed(WorkspacePersistence, WorkspacePersistence.of({ key, changed: Stream.empty })),
  initial === undefined
    ? KeyValueStore.layerMemory
    : Layer.effectDiscard(KeyValueStore.KeyValueStore.use(store => Effect.orDie(store.set(key, initial)))).pipe(Layer.provideMerge(KeyValueStore.layerMemory)),
)

/**
 * `localStorage`, with other windows' writes arriving through the `storage` event. Storage can refuse
 * even to be read in privacy modes; the workspace then stays in memory rather than breaking the host.
 */
export const localStoragePersistence = (key: string): WorkspacePersistenceLayer => Layer.mergeAll(
  Layer.sync(WorkspacePersistence, () => WorkspacePersistence.of({
    key,
    changed: typeof window === 'undefined'
      ? Stream.empty
      : Stream.fromEventListener<StorageEvent>(window, 'storage').pipe(Stream.filter(event => event.key === key), Stream.as(undefined)),
  })),
  Layer.unwrap(Effect.sync(() => {
    try {
      const storage = globalThis.localStorage
      storage.getItem(key)
      return KeyValueStore.layerStorage(() => storage)
    } catch {
      return KeyValueStore.layerMemory
    }
  })),
)

/** Something another writer did to one space. */
export type SpaceEvent =
  | { readonly _tag: 'Body'; readonly space: string; readonly body: SpaceBody; readonly revision?: string | undefined }
  | { readonly _tag: 'Removed'; readonly space: string }

/**
 * An external store that holds spaces as whole bodies and follows them live, such as a server-side
 * layout store. The workspace store publishes a space's body when its structure changes here, and
 * adopts bodies that arrive. Focus, zoom, shown tabs and every other local state never leave.
 */
export interface SpaceSync {
  /** Whether bodies carry split sizes. Without, sizes stay local and incoming bodies never reset them. */
  readonly syncsSizes: boolean
  readonly incoming: Stream.Stream<SpaceEvent>
  /**
   * Writes a whole body. `base` is the revision it was made on; `key` is stable across retries of the
   * same body on the same base, for the store's idempotency. Succeeds with the new revision, if any.
   */
  readonly publish: (space: string, body: SpaceBody, meta: { readonly base: string | undefined; readonly key: string }) => Effect.Effect<{ readonly revision?: string | undefined } | void, unknown>
  /** A space closed here. Absent: closing stays local. */
  readonly remove?: ((space: string, meta: { readonly base: string | undefined }) => Effect.Effect<void, unknown>) | undefined
}

export interface WorkspaceStore<V> {
  /** Synchronous, so an activation commits within the event that caused it. */
  readonly dispatch: (command: Command<V>) => void
  readonly getState: () => WorkspaceState<V>
  readonly subscribe: (listener: () => void) => () => void
  /** Writes pending state now, as the page goes away. */
  readonly flush: () => void
}

export interface WorkspaceStoreOptions<V> {
  readonly config: WorkspaceConfig
  readonly schema: Schema.Codec<WorkspaceState<V>>
  readonly legacy?: readonly LegacyDecoder<V>[] | undefined
  readonly sync?: readonly SpaceSync[] | undefined
  /** The document when nothing is stored. */
  readonly initial?: (() => WorkspaceState<V>) | undefined
  readonly debounceMs?: number | undefined
}

/** A short stable hash (FNV-1a, 64 bit) for idempotency keys. */
export const stableKey = (text: string): string => {
  let hash = 0xcbf29ce484222325n
  for (let index = 0; index < text.length; index += 1) {
    hash ^= BigInt(text.charCodeAt(index))
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn
  }
  return hash.toString(16).padStart(16, '0')
}

interface SyncTrack {
  /** Canonical bodies last published and last adopted, per space: neither is echoed back. */
  readonly sent: Map<string, string>
  readonly received: Map<string, string>
  readonly pending: Set<string>
  /** The latest event that arrived while the space's publish was outstanding. */
  readonly held: Map<string, SpaceEvent>
  /** Spaces this sync knows of, so that closing one here can remove it there. */
  readonly known: Set<string>
  /**
   * Spaces whose structure a command here changed since they last arrived. Only these are written:
   * adopting a body (another writer's, another window's, the stored document's) never writes it back,
   * even where adoption normalised it, so a client never rewrites what it was only shown.
   */
  readonly dirty: Set<string>
  /** Spaces a command here closed: only these are removed there. */
  readonly closed: Set<string>
}

/** Whether a command is this client's own: everything but what arrives from elsewhere. */
const isLocal = <V>(command: Command<V>) => command._tag !== 'Receive' && command._tag !== 'SetRevision'

/**
 * The Effect program that owns the workspace for as long as its scope. A debounced fiber persists and
 * publishes; changes from other windows and external stores arrive as `Receive`. React needs listeners
 * told within `dispatch`, which a `SubscriptionRef` stream cannot do, so they are called directly.
 */
export const makeWorkspaceStore = Effect.fn('makeWorkspaceStore')(function* <V>(options: WorkspaceStoreOptions<V>) {
  const { key, changed } = yield* WorkspacePersistence
  const kv = yield* KeyValueStore.KeyValueStore
  const { config, schema } = options
  const encode = Schema.encodeSync(schema)
  const load = kv.get(key).pipe(
    Effect.map(text => {
      if (text === undefined) return Option.none<WorkspaceState<V>>()
      try {
        return decodeWorkspace(JSON.parse(text) as unknown, { schema, config, legacy: options.legacy })
      } catch {
        return Option.none<WorkspaceState<V>>()
      }
    }),
    Effect.orElseSucceed(() => Option.none<WorkspaceState<V>>()),
  )
  const stored = yield* load
  const initial = Option.getOrElse(stored, () => options.initial?.() ?? emptyWorkspace<V>(config))
  const state = yield* SubscriptionRef.make(initial)
  const written = yield* Ref.make(Option.map(stored, document => JSON.stringify(encode(document))))
  // Spaces the stored document named when this window last read it: a later read without one means
  // another window closed it, not that this window has not written its new one yet.
  let remoteSpaces = new Set(Option.match(stored, { onNone: () => [], onSome: document => document.spaces.map(space => space.id) }))
  const listeners = new Set<() => void>()
  /** Told of each change a command here makes, before listeners, so syncs can mark what to write. */
  const localChanges = new Set<(before: WorkspaceState<V>, after: WorkspaceState<V>) => void>()

  const dispatch = (command: Command<V>) => {
    const current = SubscriptionRef.getUnsafe(state)
    const next = transition(current, command, config)
    if (next === current) return
    Effect.runSync(SubscriptionRef.set(state, next))
    if (isLocal(command)) for (const mark of localChanges) mark(current, next)
    for (const listener of listeners) listener()
  }

  const persist = Effect.gen(function* () {
    const text = JSON.stringify(encode(yield* SubscriptionRef.get(state)))
    const last = yield* Ref.get(written)
    if (Option.isSome(last) && last.value === text) return
    yield* Ref.set(written, Option.some(text))
    // Quota or privacy mode: the workspace stays in memory.
    yield* kv.set(key, text).pipe(Effect.ignore)
  })

  const fromOtherWindow = Effect.gen(function* () {
    const remote = yield* load
    if (Option.isNone(remote)) return
    yield* Ref.set(written, Option.some(JSON.stringify(encode(remote.value))))
    const ids = new Set(remote.value.spaces.map(space => space.id))
    for (const space of remote.value.spaces) {
      dispatch({ _tag: 'Receive', space: space.id, body: spaceBody(space, { sizes: true }), revision: space.revision })
    }
    for (const id of remoteSpaces) if (!ids.has(id)) dispatch({ _tag: 'Receive', space: id, removed: true })
    remoteSpaces = ids
  })

  yield* SubscriptionRef.changes(state).pipe(
    Stream.debounce(options.debounceMs ?? 200),
    Stream.runForEach(() => persist),
    Effect.forkScoped({ startImmediately: true }),
  )
  yield* changed.pipe(Stream.runForEach(() => fromOtherWindow), Effect.forkScoped({ startImmediately: true }))

  const scope = yield* Effect.scope
  for (const sync of options.sync ?? []) {
    const track: SyncTrack = { sent: new Map(), received: new Map(), pending: new Set(), held: new Map(), known: new Set(), dirty: new Set(), closed: new Set() }
    const structure = (space: WorkspaceState<V>['spaces'][number]) => canonicalBody(spaceBody(space, { sizes: sync.syncsSizes }))
    localChanges.add((before, after) => {
      const previous = new Map(before.spaces.map(space => [space.id, space] as const))
      for (const space of after.spaces) {
        const was = previous.get(space.id)
        previous.delete(space.id)
        if (was === space || (was !== undefined && was.name === space.name && was.layout === space.layout)) continue
        if (was === undefined || structure(was) !== structure(space)) {
          track.dirty.add(space.id)
          track.closed.delete(space.id)
        }
      }
      for (const id of previous.keys()) {
        track.dirty.delete(id)
        track.closed.add(id)
      }
    })
    const bodyOf = (space: string) => {
      const row = SubscriptionRef.getUnsafe(state).spaces.find(candidate => candidate.id === space)
      return row === undefined ? undefined : { body: spaceBody(row, { sizes: sync.syncsSizes }), revision: row.revision }
    }
    const apply = (event: SpaceEvent) => {
      if (event._tag === 'Removed') {
        track.dirty.delete(event.space)
        track.known.delete(event.space)
        track.sent.delete(event.space)
        track.received.delete(event.space)
        track.closed.delete(event.space)
        dispatch({ _tag: 'Receive', space: event.space, removed: true })
        return
      }
      track.known.add(event.space)
      const text = canonicalBody(event.body)
      track.received.set(event.space, text)
      // Our own write coming back: only its revision is news.
      if (track.sent.get(event.space) === text && event.revision !== undefined) {
        dispatch({ _tag: 'SetRevision', space: event.space, revision: event.revision })
        return
      }
      // Another writer's body wins over what this client had not written yet: last writer wins.
      track.dirty.delete(event.space)
      dispatch({ _tag: 'Receive', space: event.space, body: event.body, revision: event.revision, sizes: sync.syncsSizes })
    }
    const publishSpace = (space: string): Effect.Effect<void> => Effect.suspend(() => {
      if (track.pending.has(space) || !track.dirty.has(space)) return Effect.void
      const current = bodyOf(space)
      if (current === undefined) return Effect.void
      const text = canonicalBody(current.body)
      if (track.sent.get(space) === text || track.received.get(space) === text) return Effect.void
      track.pending.add(space)
      const meta = { base: current.revision, key: stableKey(`${space}\n${current.revision ?? ''}\n${text}`) }
      return sync.publish(space, current.body, meta).pipe(
        Effect.retry({ schedule: Schedule.exponential('200 millis'), times: 3 }),
        Effect.tap(result => Effect.sync(() => {
          track.sent.set(space, text)
          track.known.add(space)
          // Written as it stands: nothing is left to write until a command here changes it again.
          const now = bodyOf(space)
          if (now !== undefined && canonicalBody(now.body) === text) track.dirty.delete(space)
          const revision = result === undefined ? undefined : result.revision
          if (revision !== undefined) dispatch({ _tag: 'SetRevision', space, revision })
        })),
        Effect.ignore,
        Effect.ensuring(Effect.sync(() => {
          track.pending.delete(space)
          const held = track.held.get(space)
          track.held.delete(space)
          if (held !== undefined) apply(held)
        })),
        // What changed here while the write was out goes next.
        Effect.andThen(Effect.suspend(() => publishSpace(space))),
      )
    })
    const removeSpace = (space: string) => {
      track.closed.delete(space)
      track.known.delete(space)
      track.sent.delete(space)
      track.received.delete(space)
      return sync.remove === undefined ? Effect.void : sync.remove(space, { base: undefined }).pipe(Effect.ignore)
    }
    let previous = new Set(SubscriptionRef.getUnsafe(state).spaces.map(space => space.id))
    const reconcile = Effect.suspend(() => {
      const ids = new Set(SubscriptionRef.getUnsafe(state).spaces.map(space => space.id))
      const effects: Effect.Effect<void>[] = []
      for (const id of previous) if (!ids.has(id) && track.known.has(id) && track.closed.has(id)) effects.push(removeSpace(id))
      previous = ids
      for (const id of ids) effects.push(Effect.forkIn(publishSpace(id), scope).pipe(Effect.asVoid))
      return Effect.all(effects, { discard: true })
    })
    yield* sync.incoming.pipe(
      Stream.runForEach(event => Effect.sync(() => {
        if (track.pending.has(event.space)) track.held.set(event.space, event)
        else apply(event)
      })),
      Effect.forkScoped({ startImmediately: true }),
    )
    yield* SubscriptionRef.changes(state).pipe(
      Stream.debounce(options.debounceMs ?? 200),
      Stream.runForEach(() => reconcile),
      Effect.forkScoped({ startImmediately: true }),
    )
  }

  return {
    dispatch,
    getState: () => SubscriptionRef.getUnsafe(state),
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    flush: () => Effect.runSync(persist),
  } satisfies WorkspaceStore<V>
})

/** A store over its persistence, created synchronously so the first React render already has the workspace. */
export const createWorkspaceStore = <V>(layer: WorkspacePersistenceLayer, options: WorkspaceStoreOptions<V>): WorkspaceStore<V> & { readonly dispose: () => void } => {
  const scope = Effect.runSync(Scope.make())
  const store = Effect.runSync(makeWorkspaceStore(options).pipe(Scope.provide(scope), Effect.provide(layer)))
  return { ...store, dispose: () => { Effect.runFork(Scope.close(scope, Exit.void)) } }
}
