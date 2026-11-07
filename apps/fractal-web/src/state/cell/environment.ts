import * as BrowserKeyValueStore from '@effect/platform-browser/BrowserKeyValueStore'
import * as BrowserStream from '@effect/platform-browser/BrowserStream'
import { Context, Effect, Layer, Stream } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'

/** A write another window made to the shared area. `key: null` is a whole-area clear. */
export interface StorageChange {
  readonly key: string | null
  readonly newValue: string | null
}

/**
 * Where UI state cells live. `local` is shared by every window of the origin, `session` belongs to
 * this window only. `changes` carries other windows' writes to `local`; `pageHide` asks for a flush.
 */
export class CellEnvironment extends Context.Service<CellEnvironment, {
  readonly local: KeyValueStore.KeyValueStore
  readonly session: KeyValueStore.KeyValueStore
  readonly changes: Stream.Stream<StorageChange>
  readonly pageHide: Stream.Stream<unknown>
  /** Reads a key some retired storage format wrote outside the cell roots, for one-time seeding. */
  readonly retired: (key: string) => string | undefined
}>()('fractal-web/state/CellEnvironment') {}

const service = (layer: Layer.Layer<KeyValueStore.KeyValueStore>) => Effect.service(KeyValueStore.KeyValueStore).pipe(Effect.provide(layer))

/** Browser storage. Storage events never fire in the writing window, so `changes` is foreign writes only. */
export const layerBrowser: Layer.Layer<CellEnvironment> = Layer.effect(CellEnvironment, Effect.gen(function* () {
  const local = yield* service(BrowserKeyValueStore.layerLocalStorage)
  const session = yield* service(BrowserKeyValueStore.layerSessionStorage)
  return {
    local,
    session,
    changes: BrowserStream.fromEventListenerWindow('storage').pipe(
      Stream.filter((event) => event.storageArea === window.localStorage),
      Stream.map((event) => ({ key: event.key, newValue: event.newValue })),
    ),
    pageHide: BrowserStream.fromEventListenerWindow('pagehide'),
    retired: (key) => {
      try { return window.localStorage.getItem(key) ?? undefined } catch { return undefined }
    },
  }
}))

/** One window that remembers nothing past its lifetime: embeds, stories and storage-less browsers. */
export const layerMemory: Layer.Layer<CellEnvironment> = Layer.effect(CellEnvironment, Effect.gen(function* () {
  const local = yield* service(KeyValueStore.layerMemory)
  const session = yield* service(KeyValueStore.layerMemory)
  return { local, session, changes: Stream.never, pageHide: Stream.never, retired: () => undefined }
}))

/** True when this document can use localStorage; privacy modes and sandboxed frames can refuse it. */
export const browserStorageAvailable = (): boolean => {
  if (typeof window === 'undefined') return false
  try {
    const probe = 'wf.probe'
    window.localStorage.setItem(probe, probe)
    window.localStorage.removeItem(probe)
    window.sessionStorage.getItem(probe)
    return true
  } catch {
    return false
  }
}
