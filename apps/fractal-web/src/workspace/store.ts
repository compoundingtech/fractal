import * as BrowserKeyValueStore from '@effect/platform-browser/BrowserKeyValueStore'
import * as BrowserStream from '@effect/platform-browser/BrowserStream'
import { Context, Effect, Exit, Layer, Option, Ref, Schema, Scope, Stream, SubscriptionRef } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'
import { emptyWorkspace, repairWorkspace, transition, WorkspaceState, type Command, type TransitionContext } from './model.ts'

/** Where the document lives in the `KeyValueStore`, and word that another writer (a second window, later a glass follow) changed it. */
export class WorkspaceStorage extends Context.Service<WorkspaceStorage, {
  readonly key: string
  readonly changed: Stream.Stream<void>
}>()('fractal/WorkspaceStorage') {}

export type WorkspaceStorageLayer = Layer.Layer<WorkspaceStorage | KeyValueStore.KeyValueStore>

const memoryKey = 'workspace'

export const memoryStorage = (initial?: string): WorkspaceStorageLayer => Layer.mergeAll(
  Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({ key: memoryKey, changed: Stream.empty })),
  initial === undefined
    ? KeyValueStore.layerMemory
    : Layer.effectDiscard(KeyValueStore.KeyValueStore.use(store => Effect.orDie(store.set(memoryKey, initial)))).pipe(Layer.provideMerge(KeyValueStore.layerMemory)),
)

/**
 * `localStorage` can refuse even to be read in privacy modes, which is a defect of the layer rather than a
 * store error, and a browser storage failure must not break the shell: the workspace then stays in memory.
 */
export const localStorageLayer = (key: string): WorkspaceStorageLayer => Layer.mergeAll(
  Layer.sync(WorkspaceStorage, () => WorkspaceStorage.of({
    key,
    changed: BrowserStream.fromEventListenerWindow('storage').pipe(Stream.filter(event => event.key === key), Stream.as(undefined)),
  })),
  BrowserKeyValueStore.layerLocalStorage.pipe(Layer.catchCause(() => KeyValueStore.layerMemory)),
)

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

const sameDocument = Schema.toEquivalence(WorkspaceState)

/**
 * The Effect program that owns the machine, for as long as its scope. A debounced fiber persists; a change
 * from another writer arrives as `Receive`, the same adoption a followed glass would go through. React needs
 * its listeners told within `dispatch`, which a `SubscriptionRef` stream cannot do, so they are called directly.
 */
export const makeWorkspaceStore = Effect.fn('makeWorkspaceStore')(function* (options?: { readonly debounceMs?: number; readonly context?: TransitionContext }) {
  const { key, changed } = yield* WorkspaceStorage
  const documents = KeyValueStore.toSchemaStore(yield* KeyValueStore.KeyValueStore, WorkspaceState)
  const context = options?.context ?? defaultContext
  const load = documents.get(key).pipe(Effect.orElseSucceed(() => Option.none<WorkspaceState>()))
  const stored = yield* load
  const state = yield* SubscriptionRef.make(Option.match(stored, { onNone: () => emptyWorkspace, onSome: document => repairWorkspace(document, context) }))
  // An untouched new window has no document to publish.
  const written = yield* Ref.make(Option.some(Option.getOrElse(stored, () => emptyWorkspace)))
  const listeners = new Set<() => void>()

  const dispatch = (command: Command) => {
    const current = SubscriptionRef.getUnsafe(state)
    const next = transition(current, command, context)
    if (next === current) return
    Effect.runSync(SubscriptionRef.set(state, next))
    for (const listener of listeners) listener()
  }
  const persist = Effect.gen(function* () {
    const document = yield* SubscriptionRef.get(state)
    const last = yield* Ref.get(written)
    if (Option.isSome(last) && sameDocument(last.value, document)) return
    yield* Ref.set(written, Option.some(document))
    // Quota or privacy mode: the tabs stay in memory.
    yield* documents.set(key, document).pipe(Effect.ignore)
  })
  yield* SubscriptionRef.changes(state).pipe(
    Stream.debounce(options?.debounceMs ?? 200),
    Stream.runForEach(() => persist),
    Effect.forkScoped({ startImmediately: true }),
  )
  yield* changed.pipe(
    Stream.runForEach(() => Effect.flatMap(load, Option.match({
      onNone: () => Effect.void,
      onSome: remote => Ref.set(written, Option.some(remote)).pipe(Effect.andThen(Effect.sync(() => dispatch({ _tag: 'Receive', layout: remote.layout })))),
    }))),
    Effect.forkScoped({ startImmediately: true }),
  )
  return {
    dispatch,
    getState: () => SubscriptionRef.getUnsafe(state),
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    flush: () => Effect.runSync(persist),
  } satisfies Omit<WorkspaceStore, 'dispose'>
})

/** A store over its storage layer, created synchronously so the first React render already has the tabs. */
export const createWorkspaceStore = (layer: WorkspaceStorageLayer, options?: Parameters<typeof makeWorkspaceStore>[0]): WorkspaceStore => {
  const scope = Effect.runSync(Scope.make())
  const store = Effect.runSync(makeWorkspaceStore(options).pipe(Scope.provide(scope), Effect.provide(layer)))
  return { ...store, dispose: () => { Effect.runFork(Scope.close(scope, Exit.void)) } }
}
