import { Context, Effect, Fiber, Layer, Option, Ref, Schema, Stream, SubscriptionRef } from 'effect'
import { emptyWorkspace, repairWorkspace, transition, WorkspaceState, type Command, type TransitionContext } from './model.ts'

const WorkspaceJson = Schema.fromJsonString(WorkspaceState)
const decodeDocument = Schema.decodeUnknownOption(WorkspaceJson)
const encodeDocument = Schema.encodeSync(WorkspaceJson)

/** Device-local persistence. Another writer of the document (a second window, later a glass follow) arrives through `subscribe`. */
export class WorkspaceStorage extends Context.Service<WorkspaceStorage, {
  readonly load: Effect.Effect<string | undefined>
  readonly save: (document: string) => Effect.Effect<void>
  readonly subscribe: (listener: (document: string) => void) => () => void
}>()('fractal/WorkspaceStorage') {}

export const memoryStorage = (initial?: string) => Layer.sync(WorkspaceStorage, () => {
  let document = initial
  return WorkspaceStorage.of({
    load: Effect.sync(() => document),
    save: next => Effect.sync(() => { document = next }),
    subscribe: () => () => {},
  })
})

/** A browser storage failure degrades to an in-memory workspace; it never breaks the shell. */
export const localStorageLayer = (key: string) => Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({
  load: Effect.sync(() => {
    try { return window.localStorage.getItem(key) ?? undefined } catch { return undefined }
  }),
  save: document => Effect.sync(() => {
    try { window.localStorage.setItem(key, document) } catch { /* Quota or privacy mode: the tabs stay in memory. */ }
  }),
  subscribe: listener => {
    const onStorage = (event: StorageEvent) => { if (event.key === key && event.newValue !== null) listener(event.newValue) }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  },
}))

export interface WorkspaceStore {
  /** Synchronous, so an activation commits in the event that caused it. */
  readonly dispatch: (command: Command) => void
  readonly getState: () => WorkspaceState
  readonly subscribe: (listener: () => void) => () => void
  /** Writes pending state now, as the page goes away. */
  readonly flush: () => void
  readonly dispose: () => void
}

const defaultContext: TransitionContext = { newId: () => globalThis.crypto.randomUUID() }

/**
 * The Effect program that owns the machine. A debounced fiber persists; a change from another writer
 * arrives as `Receive`, the same adoption a followed glass would go through.
 */
export const makeWorkspaceStore = (options?: { readonly debounceMs?: number; readonly context?: TransitionContext }) => Effect.gen(function* () {
  const storage = yield* WorkspaceStorage
  const context = options?.context ?? defaultContext
  const stored = yield* storage.load
  const decoded = stored === undefined ? Option.none<WorkspaceState>() : decodeDocument(stored)
  const initial = Option.isSome(decoded) ? repairWorkspace(decoded.value, context) : emptyWorkspace
  const changes = yield* SubscriptionRef.make<WorkspaceState>(initial)
  const written = yield* Ref.make(stored)
  const listeners = new Set<() => void>()
  let snapshot = initial

  const dispatch = (command: Command) => {
    const next = transition(snapshot, command, context)
    if (next === snapshot) return
    snapshot = next
    Effect.runSync(SubscriptionRef.set(changes, next))
    for (const listener of listeners) listener()
  }
  const persist = Effect.gen(function* () {
    const document = encodeDocument(snapshot)
    if ((yield* Ref.get(written)) === document) return
    yield* Ref.set(written, document)
    yield* storage.save(document)
  })
  const persisting = yield* SubscriptionRef.changes(changes).pipe(
    Stream.debounce(options?.debounceMs ?? 200),
    Stream.runForEach(() => persist),
    Effect.forkDetach,
  )
  const unsubscribe = storage.subscribe(raw => {
    const remote = decodeDocument(raw)
    if (Option.isSome(remote)) {
      Effect.runSync(Ref.set(written, raw))
      dispatch({ _tag: 'Receive', layout: remote.value.layout })
    }
  })
  return {
    dispatch,
    getState: () => snapshot,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    flush: () => Effect.runSync(persist),
    dispose: () => {
      unsubscribe()
      Effect.runFork(Fiber.interrupt(persisting))
    },
  } satisfies WorkspaceStore
})

/** A store over its storage layer, created synchronously so the first React render already has the tabs. */
export const createWorkspaceStore = (layer: Layer.Layer<WorkspaceStorage>, options?: Parameters<typeof makeWorkspaceStore>[0]): WorkspaceStore =>
  Effect.runSync(makeWorkspaceStore(options).pipe(Effect.provide(layer)))
