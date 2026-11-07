import { type Cause, Effect, Layer, Queue, Stream } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'

import { CellEnvironment, type StorageChange } from './environment.ts'

const memoryArea = ({ values, quota, onChange }: {
  readonly values: Map<string, string>
  readonly quota: number
  readonly onChange: (change: StorageChange) => void
}) => KeyValueStore.makeStringOnly({
  get: (key) => Effect.sync(() => values.get(key)),
  set: (key, value) => Effect.suspend(() => {
    let used = key.length + value.length
    for (const [other, stored] of values) if (other !== key) used += other.length + stored.length
    if (used > quota) {
      return Effect.fail(new KeyValueStore.KeyValueStoreError({ key, method: 'set', message: 'QuotaExceededError' }))
    }
    if (values.get(key) === value) return Effect.void
    values.set(key, value)
    onChange({ key, newValue: value })
    return Effect.void
  }),
  remove: (key) => Effect.sync(() => {
    if (values.delete(key)) onChange({ key, newValue: null })
  }),
  clear: Effect.sync(() => {
    values.clear()
    onChange({ key: null, newValue: null })
  }),
  size: Effect.sync(() => values.size),
  keys: Effect.sync(() => [...values.keys()]),
})

/**
 * Same-origin windows over one memory `localStorage`, each with its own `sessionStorage`. Like the
 * browser, a write reaches every other window as a change, never the writer. Sizes count characters.
 */
export const memoryOrigin = ({ quota = Number.POSITIVE_INFINITY }: { readonly quota?: number } = {}) => {
  const local = new Map<string, string>()
  const windows = new Set<{ readonly deliver: (change: StorageChange) => void }>()
  /** Pass a previous window's `session` to model a reload of that window. */
  const window = ({ session = new Map<string, string>() }: { readonly session?: Map<string, string> } = {}) => {
    const received: StorageChange[] = []
    let queue: Queue.Queue<StorageChange, Cause.Done> | undefined
    const self = { deliver: (change: StorageChange) => { if (queue === undefined) received.push(change); else Queue.offerUnsafe(queue, change) } }
    windows.add(self)
    const hide = { listeners: new Set<() => void>() }
    const environment = Layer.succeed(CellEnvironment, {
      local: memoryArea({ values: local, quota, onChange: (change) => { for (const other of windows) if (other !== self) other.deliver(change) } }),
      session: memoryArea({ values: session, quota, onChange: () => {} }),
      changes: Stream.callback<StorageChange>((events) => Effect.sync(() => {
        queue = events
        for (const change of received.splice(0)) Queue.offerUnsafe(events, change)
      })),
      retired: (key: string) => local.get(key),
      pageHide: Stream.callback<void>((events) => Effect.sync(() => {
        const listener = () => { Queue.offerUnsafe(events, undefined) }
        hide.listeners.add(listener)
      })),
    })
    return {
      environment,
      session,
      hide: () => { for (const listener of hide.listeners) listener() },
      close: () => { windows.delete(self) },
    }
  }
  return { local, window }
}
