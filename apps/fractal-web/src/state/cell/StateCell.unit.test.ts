// @vitest-environment node
import { Effect, ManagedRuntime, Schema } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, describe, expect, it } from 'vitest'

import * as StateCell from './StateCell.ts'
import * as Store from './store.ts'
import { memoryOrigin } from './testing.ts'

const settle = (ms = 30) => new Promise((resolve) => setTimeout(resolve, ms))

const DockV1 = Schema.Struct({ open: Schema.Boolean, width: Schema.Finite })
const Dock = Schema.Struct({ ...DockV1.fields, tab: Schema.Literals(['details', 'changes']) })
const dockSpec = {
  scope: 'global',
  key: 'test.dock',
  version: 2,
  schema: Dock,
  default: { open: false, width: 440, tab: 'details' },
  migrate: (value: unknown, from: number) => from === 1 ? { ...Schema.decodeUnknownSync(DockV1)(value), tab: 'details' } : value,
  tier: 'boot',
  boot: ({ open, width }: typeof Dock.Type) => ({ 'dock.open': open, 'dock.width': width }),
} as const
const dock = StateCell.make(dockSpec)
const folds = StateCell.make({ scope: 'tab', key: 'test.folds', version: 1, schema: Schema.Record(Schema.String, Schema.Boolean), default: {}, tier: 'window' })
const Draft = Schema.Struct({ text: Schema.String, updatedAt: Schema.Int })
const draft = StateCell.make({ scope: 'person', key: 'test.draft', version: 1, schema: Draft, default: { text: '', updatedAt: 0 }, tier: 'sensitive', retain: 3 })

/** A store per window, the way the app has one per document. */
type Environment = Parameters<typeof StateCell.layer>[0]
const open = (environment: Environment) => {
  const runtime = ManagedRuntime.make(StateCell.layer(environment, { debounceMs: 5 }))
  const store = runtime.runSync(Effect.service(Store.CellStore))
  return { store, dispose: () => runtime.dispose() }
}
const binding = <A, I>(store: Store.CellStoreShape, spec: { readonly key: string; readonly schema: Schema.Codec<A, I>; readonly default: A; readonly version: number }, extra: Partial<Store.CellBindingSpec<A, I>> = {}) =>
  store.bind({ tier: 'shared', ...spec, ...extra })

let uninstall: (() => Promise<void>) | undefined
const install = (environment: Environment) => {
  void uninstall?.()
  uninstall = StateCell.install(StateCell.layer(environment, { debounceMs: 5 }))
}
afterEach(async () => { await uninstall?.(); uninstall = undefined })

describe('StateCell', () => {
  it('restores global, window and person cells after a reload', () => {
    const origin = memoryOrigin()
    const first = origin.window()
    install(first.environment)
    const registry = AtomRegistry.make()
    registry.set(dock, { open: true, width: 512, tab: 'changes' })
    registry.set(folds('tab-1'), { turn: true })
    registry.set(draft('ada', 'gw/c1'), { text: 'hello', updatedAt: 1 })
    StateCell.flush()

    install(origin.window({ session: first.session }).environment)
    const fresh = AtomRegistry.make()
    expect(fresh.get(dock)).toEqual({ open: true, width: 512, tab: 'changes' })
    expect(fresh.get(folds('tab-1'))).toEqual({ turn: true })
    expect(fresh.get(draft('ada', 'gw/c1')).text).toBe('hello')
    // Window state stays with its window; drafts and the dock are shared.
    install(origin.window().environment)
    const other = AtomRegistry.make()
    expect(other.get(folds('tab-1'))).toEqual({})
    expect(other.get(draft('ada', 'gw/c1')).text).toBe('hello')
  })

  it('writes once per burst, after the debounce, and on pagehide', async () => {
    const origin = memoryOrigin()
    const window = origin.window()
    install(window.environment)
    const registry = AtomRegistry.make()
    const key = `${Store.localRoot}test.dock`
    for (let width = 300; width < 340; width += 1) registry.set(dock, { open: true, width, tab: 'details' })
    expect(origin.local.get(key)).toBeUndefined()
    await settle()
    expect(origin.local.get(key)).toContain('"width":339')
    registry.set(dock, { open: false, width: 339, tab: 'details' })
    window.hide()
    await settle(0)
    expect(origin.local.get(key)).toContain('"open":false')
  })

  it('projects boot fields, recomputed from stored values on the next load', () => {
    const origin = memoryOrigin()
    const first = origin.window()
    install(first.environment)
    AtomRegistry.make().set(dock, { open: true, width: 480, tab: 'changes' })
    StateCell.flush()
    expect(JSON.parse(origin.local.get(StateCell.bootKey)!)).toEqual({ 'dock.open': true, 'dock.width': 480 })
    // A lost projection is rebuilt from the cell the next time a window reads it.
    origin.local.delete(StateCell.bootKey)
    install(origin.window().environment)
    AtomRegistry.make().get(dock)
    StateCell.flush()
    expect(JSON.parse(origin.local.get(StateCell.bootKey)!)).toEqual({ 'dock.open': true, 'dock.width': 480 })
  })

  it('migrates an older version and leaves a newer one for its client', () => {
    const origin = memoryOrigin()
    origin.local.set(`${Store.localRoot}test.dock`, JSON.stringify({ v: 1, t: 1, w: 'old', d: { open: true, width: 333 } }))
    install(origin.window().environment)
    expect(AtomRegistry.make().get(dock)).toEqual({ open: true, width: 333, tab: 'details' })

    const future = memoryOrigin()
    const stored = JSON.stringify({ v: 9, t: 1, w: 'future', d: { shape: 'new' } })
    future.local.set(`${Store.localRoot}test.dock`, stored)
    install(future.window().environment)
    expect(AtomRegistry.make().get(dock).width).toBe(440)
    StateCell.flush()
    expect(future.local.get(`${Store.localRoot}test.dock`)).toBe(stored)
    expect(StateCell.issues()).toContainEqual({ _tag: 'FutureVersion', key: 'test.dock', version: 9 })
  })

  it('shows the default for corrupt values without throwing', () => {
    const origin = memoryOrigin()
    origin.local.set(`${Store.localRoot}test.dock`, '{bad')
    install(origin.window().environment)
    expect(AtomRegistry.make().get(dock).width).toBe(440)
  })

  it('seeds once from a retired format when nothing is stored', () => {
    const origin = memoryOrigin()
    install(origin.window().environment)
    let calls = 0
    const legacy = StateCell.make({ scope: 'global', key: 'test.legacy', version: 1, schema: Schema.Number, default: 0, seed: () => { calls += 1; return 7 } })
    expect(AtomRegistry.make().get(legacy)).toBe(7)
    StateCell.flush()
    install(origin.window().environment)
    expect(AtomRegistry.make().get(legacy)).toBe(7)
    expect(calls).toBe(1)
  })

  it('keeps the newest drafts per person and evicts the oldest under quota', () => {
    const origin = memoryOrigin({ quota: 600 })
    install(origin.window().environment)
    const registry = AtomRegistry.make()
    for (let index = 0; index < 5; index += 1) {
      registry.set(draft('ada', `c${index}`), { text: 't', updatedAt: index })
      StateCell.flush()
    }
    const drafts = () => [...origin.local.keys()].filter((key) => key.includes('test.draft/')).sort()
    expect(drafts()).toEqual(['wf.c/p:ada/test.draft/c2', 'wf.c/p:ada/test.draft/c3', 'wf.c/p:ada/test.draft/c4'])
    // Too big for the space left: older drafts give way; the newest edit is kept.
    registry.set(draft('ada', 'c5'), { text: 'x'.repeat(350), updatedAt: 5 })
    StateCell.flush()
    expect(drafts()).toContain('wf.c/p:ada/test.draft/c5')
    expect(drafts().length).toBeLessThan(3)
    expect(registry.get(draft('ada', 'c5')).text.length).toBe(350)
  })

  it('keeps the live value and reports when nothing can make room', () => {
    install(memoryOrigin({ quota: 100 }).window().environment)
    const registry = AtomRegistry.make()
    registry.set(dock, { open: true, width: 1, tab: 'details' })
    registry.set(draft('ada', 'big'), { text: 'x'.repeat(500), updatedAt: 1 })
    StateCell.flush()
    expect(registry.get(draft('ada', 'big')).text.length).toBe(500)
    expect(StateCell.issues().some((issue) => issue._tag === 'WriteFailed')).toBe(true)
  })
})

describe('across windows', () => {
  const dockBinding = (store: Store.CellStoreShape) => binding(store, { ...dockSpec }, { tier: 'boot', migrate: dockSpec.migrate, boot: dockSpec.boot })

  it('a second window follows a change and keeps its own other cells', async () => {
    const origin = memoryOrigin()
    const a = open(origin.window().environment), b = open(origin.window().environment)
    const dockA = dockBinding(a.store), dockB = dockBinding(b.store)
    let seen = 0
    dockB.subscribe(() => { seen += 1 })
    expect(dockB.read().open).toBe(false)
    dockA.write({ open: true, width: 500, tab: 'details' })
    a.store.flush()
    await settle()
    expect(dockB.read()).toEqual({ open: true, width: 500, tab: 'details' })
    expect(seen).toBeGreaterThan(0)
    await a.dispose(); await b.dispose()
  })

  it('the later write wins when two windows edit the same cell', async () => {
    const origin = memoryOrigin()
    const a = open(origin.window().environment), b = open(origin.window().environment)
    const dockA = dockBinding(a.store), dockB = dockBinding(b.store)
    dockA.write({ open: true, width: 300, tab: 'details' })
    await settle(1)
    dockB.write({ open: true, width: 600, tab: 'details' })
    b.store.flush(); a.store.flush()
    await settle()
    expect(dockA.read().width).toBe(600)
    expect(JSON.parse(origin.local.get(`${Store.localRoot}test.dock`)!).d.width).toBe(600)
    await a.dispose(); await b.dispose()
  })

  it('clearScope removes a person everywhere, and an unsaved edit in another window does not come back', async () => {
    const origin = memoryOrigin()
    const a = open(origin.window().environment), b = open(origin.window().environment)
    const spec = { key: 'p:ada/test.draft/c', schema: Draft, default: { text: '', updatedAt: 0 }, version: 1 }
    const draftA = binding(a.store, spec, { tier: 'sensitive' }), draftB = binding(b.store, spec, { tier: 'sensitive' })
    draftA.write({ text: 'secret', updatedAt: 1 })
    a.store.flush()
    await settle()
    // Window B edits and has not flushed when the revoke lands in window A.
    draftB.write({ text: 'secret more', updatedAt: 2 })
    a.store.clearPrefix(StateCell.scopePrefix('person', 'ada'))
    await settle()
    b.store.flush()
    expect([...origin.local.values()].some((value) => value.includes('secret'))).toBe(false)
    expect(draftA.read().text).toBe('')
    expect(draftB.read().text).toBe('')
    await a.dispose(); await b.dispose()
  })
})
