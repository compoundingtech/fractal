import { describe, expect, it } from 'vitest'
import { Deferred, Effect, Layer, Option, Queue, Schema, Stream } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'
import * as TestClock from 'effect/testing/TestClock'
import { itEffect } from './testing.ts'
import { currentSpace, emptyWorkspace, legacyDecoder, makeWorkspaceSchema, panesOf, tabsOf, transition, type Layout, type SpaceBody, type WorkspaceConfig, type WorkspaceState } from './mod.ts'
import { makeWorkspaceStore, memoryPersistence, stableKey, WorkspacePersistence, type SpaceEvent, type SpaceSync } from './store.ts'

const View = Schema.Struct({ scrollTop: Schema.optional(Schema.Finite) })
type View = typeof View.Type
const schema = makeWorkspaceSchema(View)

const counter = (prefix: string): WorkspaceConfig => {
  let next = 0
  return { newId: () => `${prefix}-${next++}` }
}

const initial = () => emptyWorkspace<View>(counter('init'), { id: 'space', name: 'Main' })
const refs = (state: WorkspaceState<View>) => tabsOf(currentSpace(state).layout).map(tab => tab.ref)
const settle = TestClock.adjust('1 second')

interface FakeSync {
  readonly sync: SpaceSync
  readonly published: { space: string; body: SpaceBody; base: string | undefined; key: string }[]
  readonly removed: string[]
  readonly incoming: Queue.Queue<SpaceEvent>
  /** While set, each publish waits for it. */
  gate: Deferred.Deferred<void> | undefined
}

const fakeSync = Effect.fn(function* (syncsSizes = true) {
  const incoming = yield* Queue.unbounded<SpaceEvent>()
  const fake: FakeSync = {
    published: [],
    removed: [],
    incoming,
    gate: undefined,
    sync: undefined as unknown as SpaceSync,
  }
  let revision = 0
  ;(fake as { sync: SpaceSync }).sync = {
    syncsSizes,
    incoming: Stream.fromQueue(incoming),
    publish: (space, body, meta) => Effect.gen(function* () {
      fake.published.push({ space, body, ...meta })
      if (fake.gate !== undefined) yield* Deferred.await(fake.gate)
      revision += 1
      return { revision: `r${revision}` }
    }),
    remove: space => Effect.sync(() => { fake.removed.push(space) }),
  }
  return fake
})

describe('the workspace store', () => {
  itEffect('persists after a quiet period and reloads, repaired, in a new store', () => Effect.gen(function* () {
    const kv = yield* KeyValueStore.KeyValueStore
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, initial })
    store.dispatch({ _tag: 'Open', ref: 'doc:a' })
    store.dispatch({ _tag: 'Open', ref: 'doc:b' })
    expect(yield* kv.get('workspace')).toBeUndefined()
    yield* settle
    expect(yield* kv.get('workspace')).toBeDefined()
    const again = yield* makeWorkspaceStore({ config: counter('b'), schema })
    expect(refs(again.getState())).toEqual(['doc:a', 'doc:b'])
  }).pipe(Effect.provide(memoryPersistence())))

  itEffect('migrates a legacy document on load', () => Effect.gen(function* () {
    const legacy = [legacyDecoder(Schema.Struct({ open: Schema.Array(Schema.String) }), old =>
      old.open.reduce((state, ref) => transition(state, { _tag: 'Open', ref }, counter('m')), initial()))]
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, legacy })
    expect(refs(store.getState())).toEqual(['doc:x', 'doc:y'])
  }).pipe(Effect.provide(memoryPersistence(JSON.stringify({ open: ['doc:x', 'doc:y'] })))))

  itEffect('adopts another window\'s structure and keeps its own focus', () => Effect.gen(function* () {
    const changed = yield* Queue.unbounded<void>()
    const kv = yield* KeyValueStore.KeyValueStore
    const layer = Layer.succeed(WorkspacePersistence, WorkspacePersistence.of({ key: 'workspace', changed: Stream.fromQueue(changed) }))
    const mine = yield* makeWorkspaceStore({ config: counter('a'), schema, initial }).pipe(Effect.provide(layer))
    mine.dispatch({ _tag: 'Open', ref: 'doc:a' })
    yield* settle
    // Another window adds a tab and a second space, and writes.
    const other = transition(transition(Option.getOrThrow(Schema.decodeUnknownOption(schema)(JSON.parse((yield* kv.get('workspace'))!))), { _tag: 'Open', ref: 'doc:b' }, counter('o')), { _tag: 'SpaceNew', id: 'second', name: 'Second' }, counter('o2'))
    yield* kv.set('workspace', JSON.stringify(Schema.encodeSync(schema)(other)))
    yield* Queue.offer(changed, undefined)
    yield* settle
    expect(refs(mine.getState())).toEqual(['doc:a', 'doc:b'])
    expect(mine.getState().spaces.map(space => space.id)).toEqual(['space', 'second'])
    expect(mine.getState().current).toBe('space')
    // That window closes the second space.
    const closed = transition(other, { _tag: 'SpaceClose', space: 'second' }, counter('o3'))
    yield* kv.set('workspace', JSON.stringify(Schema.encodeSync(schema)(closed)))
    yield* Queue.offer(changed, undefined)
    yield* settle
    expect(mine.getState().spaces.map(space => space.id)).toEqual(['space'])
  }).pipe(Effect.provide(KeyValueStore.layerMemory)))
})

describe('syncing spaces with an external store', () => {
  itEffect('publishes structure, never local state, and does not echo what it adopts', () => Effect.gen(function* () {
    const fake = yield* fakeSync()
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, initial, sync: [fake.sync] })
    store.dispatch({ _tag: 'Open', ref: 'doc:a' })
    store.dispatch({ _tag: 'Open', ref: 'doc:b' })
    yield* settle
    expect(fake.published.map(row => row.body)).toEqual([{ name: 'Main', layout: { tabs: [{ ref: 'doc:a' }, { ref: 'doc:b' }] } }])
    expect(currentSpace(store.getState()).revision).toBe('r1')
    // Showing another tab and zooming are this client's alone.
    store.dispatch({ _tag: 'Cycle', by: 1 })
    store.dispatch({ _tag: 'SetView', tab: tabsOf(currentSpace(store.getState()).layout)[0]!.id, view: { scrollTop: 3 } })
    yield* settle
    expect(fake.published).toHaveLength(1)
    // Another writer's body is adopted and not sent back.
    yield* Queue.offer(fake.incoming, { _tag: 'Body', space: 'space', body: { name: 'Main', layout: { tabs: [{ ref: 'doc:b' }, { ref: 'doc:c' }] } }, revision: 'r9' })
    yield* settle
    expect(refs(store.getState())).toEqual(['doc:b', 'doc:c'])
    expect(currentSpace(store.getState()).revision).toBe('r9')
    expect(fake.published).toHaveLength(1)
    // The next local change builds on that revision.
    store.dispatch({ _tag: 'Open', ref: 'doc:d' })
    yield* settle
    expect(fake.published[1]).toMatchObject({ base: 'r9' })
  }).pipe(Effect.provide(memoryPersistence())))

  itEffect('holds incoming bodies while its own write is out, then applies only the latest', () => Effect.gen(function* () {
    const fake = yield* fakeSync()
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, initial, sync: [fake.sync] })
    const gate = yield* Deferred.make<void>()
    fake.gate = gate
    store.dispatch({ _tag: 'Open', ref: 'doc:a' })
    yield* settle
    expect(fake.published).toHaveLength(1)
    yield* Queue.offer(fake.incoming, { _tag: 'Body', space: 'space', body: { name: 'Main', layout: { tabs: [{ ref: 'old:1' }] } }, revision: 'old' })
    yield* Queue.offer(fake.incoming, { _tag: 'Body', space: 'space', body: { name: 'Main', layout: { tabs: [{ ref: 'doc:a' }, { ref: 'won:2' }] } }, revision: 'won' })
    yield* settle
    expect(refs(store.getState()), 'nothing rolls back while the write is out').toEqual(['doc:a'])
    fake.gate = undefined
    yield* Deferred.succeed(gate, undefined)
    yield* settle
    expect(refs(store.getState())).toEqual(['doc:a', 'won:2'])
    expect(currentSpace(store.getState()).revision).toBe('won')
    expect(fake.published).toHaveLength(1)
  }).pipe(Effect.provide(memoryPersistence())))

  itEffect('keeps local sizes when the store does not carry them', () => Effect.gen(function* () {
    const fake = yield* fakeSync(false)
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, initial, sync: [fake.sync] })
    store.dispatch({ _tag: 'Open', ref: 'doc:a' })
    store.dispatch({ _tag: 'Split', side: 'right', ref: 'doc:b' })
    const split = currentSpace(store.getState()).layout as Extract<Layout, { axis: string }>
    yield* settle
    store.dispatch({ _tag: 'Resize', split: split.id, sizes: [0.3, 0.7] })
    yield* settle
    expect(fake.published).toHaveLength(1)
    expect(JSON.stringify(fake.published[0]!.body)).not.toContain('sizes')
    yield* Queue.offer(fake.incoming, { _tag: 'Body', space: 'space', body: fake.published[0]!.body, revision: 'again' })
    yield* settle
    expect((currentSpace(store.getState()).layout as typeof split).sizes).toEqual([0.3, 0.7])
    expect(panesOf(currentSpace(store.getState()).layout)).toHaveLength(2)
  }).pipe(Effect.provide(memoryPersistence())))

  itEffect('creates spaces that arrive, and removes them both ways', () => Effect.gen(function* () {
    const fake = yield* fakeSync()
    const store = yield* makeWorkspaceStore({ config: counter('a'), schema, initial, sync: [fake.sync] })
    yield* Queue.offer(fake.incoming, { _tag: 'Body', space: 'tui', body: { name: 'Elsewhere', layout: { tabs: [] } }, revision: 'r1' })
    yield* settle
    expect(store.getState().spaces.map(space => space.name)).toEqual(['Main', 'Elsewhere'])
    store.dispatch({ _tag: 'SpaceClose', space: 'tui' })
    yield* settle
    expect(fake.removed).toEqual(['tui'])
    store.dispatch({ _tag: 'SpaceNew', id: 'mine', name: 'Mine' })
    yield* settle
    yield* Queue.offer(fake.incoming, { _tag: 'Removed', space: 'mine' })
    yield* settle
    expect(store.getState().spaces.map(space => space.id)).toEqual(['space'])
  }).pipe(Effect.provide(memoryPersistence())))

  it('derives the same idempotency key for the same body on the same base', () => {
    expect(stableKey('a\nr1\n{}')).toBe(stableKey('a\nr1\n{}'))
    expect(stableKey('a\nr1\n{}')).not.toBe(stableKey('a\nr2\n{}'))
  })
})
