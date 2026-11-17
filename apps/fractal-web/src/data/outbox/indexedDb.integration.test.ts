import 'fake-indexeddb/auto'
import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { it } from '@effect/vitest'
import { Effect } from 'effect'
import { afterEach, describe, expect, vi } from 'vitest'
import { attemptsStore, openIndexedDbOutbox, spoolAttachment, type IndexedDbOutbox } from './indexedDb.ts'
import { OutboxConflict, retentionAction, type OutboxRecord } from './repository.ts'

const scope = { account: 'account/a', deployment: 'deployment/a', member: 'member/a', profile: 'profile/a', kind: 'chat' } as const
const packet = {
  bytes: [123, 34, 120, 34, 58, 49, 125], fingerprint: { version: 1, sha256: 'body-sha256' },
  actionId: 'action/a', idempotencyKey: 'key/a', expectedIdentities: ['message/a'],
} as const
const attempt = (id = 'attempt/a'): OutboxRecord => ({
  schemaVersion: 1, localAttemptId: id, scope, intent: { bytes: [0, 127, 255], attachments: [] },
  state: { status: 'pending', phase: 'preparing' }, mayHaveDispatched: false, revision: 0,
  createdAt: 1, updatedAt: 1, timelineObserved: false,
})
const ready = (record: OutboxRecord): OutboxRecord => ({ ...record, revision: record.revision + 1, preparedPacket: packet, state: { status: 'pending', phase: 'ready' } })
const connections: IndexedDbOutbox[] = []
const open = Effect.fn('test.open')(function* (factory: IDBFactory, now = () => 100) {
  const repo = yield* openIndexedDbOutbox({ indexedDB: factory, now })
  connections.push(repo)
  return repo
})
afterEach(() => { for (const connection of connections.splice(0)) connection.close(); vi.restoreAllMocks() })

const writeRaw = (factory: IDBFactory, values: readonly unknown[]): Promise<void> => {
  const { promise, resolve, reject } = Promise.withResolvers<void>()
  const request = factory.open('fractal-web-outbox-v1', 1)
  request.onerror = () => reject(request.error)
  request.onsuccess = () => {
    const db = request.result
    const tx = db.transaction(attemptsStore, 'readwrite')
    tx.oncomplete = () => { db.close(); resolve() }
    tx.onabort = () => { db.close(); reject(tx.error) }
    for (const value of values) tx.objectStore(attemptsStore).put(value)
  }
  return promise
}

describe('durable IndexedDB outbox', () => {
  it.effect('acknowledges only after transaction completion, never request success', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const writer = yield* open(factory)
    const reader = yield* open(factory)
    let acknowledged = false
    let requestSucceeded = false
    let completed = false
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) {
        request.addEventListener('success', () => {
          requestSucceeded = true
          expect(acknowledged).toBe(false)
        })
        this.transaction.addEventListener('complete', () => { completed = true })
      }
      return request
    })
    expect(yield* reader.read('attempt/a')).toBeUndefined()
    yield* writer.commit(attempt())
    acknowledged = true
    expect(requestSucceeded).toBe(true)
    expect(completed).toBe(true)
    expect(yield* reader.read('attempt/a')).toEqual(attempt())
  }))

  it.effect('an aborted commit leaves neither accepted intent nor attachment bytes', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const repo = yield* open(factory)
    const image = yield* spoolAttachment(new Uint8Array([1, 2, 3]))
    const record = { ...attempt(), intent: { bytes: [1], attachments: [{ hash: image.hash, name: 'image.png', mediaType: 'image/png' }] } }
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) request.addEventListener('success', () => this.transaction.abort())
      return request
    })
    expect(yield* repo.commit(record, [image]).pipe(Effect.flip)).toMatchObject({ _tag: 'OutboxStorageError' })
    expect(yield* repo.read(record.localAttemptId)).toBeUndefined()
    expect(yield* repo.readSpool(record.localAttemptId, image.hash)).toBeUndefined()
  }))

  it.effect('stale revision CAS loses with a typed conflict', () => Effect.gen(function* () {
    const repo = yield* open(new IDBFactory())
    const record = yield* repo.commit(attempt())
    yield* repo.compareAndSwap(0, ready(record))
    expect(yield* repo.compareAndSwap(0, ready(record)).pipe(Effect.flip)).toEqual(new OutboxConflict({ localAttemptId: record.localAttemptId, expectedRevision: 0, actualRevision: 1 }))
  }))

  it.effect('two independent connections race for one dispatch permit; exactly one wins', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const first = yield* open(factory)
    const second = yield* open(factory)
    const record = yield* first.commit(attempt())
    yield* first.compareAndSwap(0, ready(record))
    const results = yield* Effect.all([first, second].map((repo) => repo.acquireDispatchPermit(record.localAttemptId, 1).pipe(Effect.match({
      onSuccess: (value) => ({ won: true as const, value }), onFailure: (value) => ({ won: false as const, value }),
    }))), { concurrency: 'unbounded' })
    expect(results.filter((result) => result.won)).toHaveLength(1)
    expect(results.find((result) => !result.won)?.value).toMatchObject({ _tag: 'OutboxConflict', expectedRevision: 1, actualRevision: 2 })
    expect(yield* first.read(record.localAttemptId)).toMatchObject({ mayHaveDispatched: true, state: { phase: 'dispatched' } })
  }))

  it.effect('close and reopen preserve exact record, unsigned packet and image bytes', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const repo = yield* open(factory)
    const image = yield* spoolAttachment(new Uint8Array([0, 255, 1, 128]))
    const record = { ...attempt(), intent: { bytes: [255, 0], attachments: [{ hash: image.hash, name: 'image.png', mediaType: 'image/png' }] } }
    yield* repo.commit(record, [image])
    const prepared = yield* repo.compareAndSwap(0, ready(record))
    repo.close()
    const reopened = yield* open(factory)
    expect(yield* reopened.read(record.localAttemptId)).toEqual(prepared)
    expect(yield* reopened.readSpool(record.localAttemptId, image.hash)).toEqual(image.bytes)
    expect((yield* reopened.read(record.localAttemptId))?.preparedPacket?.bytes).toEqual(packet.bytes)
  }))

  it.effect('startup restores dispatched nonterminal as unknown but preserves never-dispatched pending', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const repo = yield* open(factory)
    const dispatched = yield* repo.commit(attempt('dispatched'))
    yield* repo.compareAndSwap(0, ready(dispatched))
    yield* repo.acquireDispatchPermit('dispatched', 1)
    yield* repo.commit(attempt('preparing'))
    const prepared = yield* repo.commit(attempt('ready'))
    yield* repo.compareAndSwap(0, ready(prepared))
    repo.close()
    const reopened = yield* open(factory)
    expect(yield* reopened.read('dispatched')).toMatchObject({ state: { status: 'unknown' }, mayHaveDispatched: true, revision: 3 })
    expect(yield* reopened.read('preparing')).toMatchObject({ state: { status: 'pending', phase: 'preparing' }, revision: 0 })
    expect(yield* reopened.read('ready')).toMatchObject({ state: { status: 'pending', phase: 'ready' }, revision: 1 })
    expect(yield* reopened.acquireDispatchPermit('dispatched', 3).pipe(Effect.flip)).toMatchObject({ code: 'invalid-record' })
  }))

  it.effect('corrupt and future-version records are kept in quarantine, never listed or dispatchable', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const repo = yield* open(factory)
    const future = { ...attempt('future'), schemaVersion: 999 }
    const corrupt = { localAttemptId: 'corrupt', schemaVersion: 1, scope }
    yield* Effect.promise(() => writeRaw(factory, [future, corrupt]))
    repo.close()
    const reopened = yield* open(factory)
    expect(yield* reopened.list(scope)).toEqual([])
    const quarantine = yield* reopened.listQuarantined()
    expect(quarantine).toHaveLength(2)
    expect(quarantine.find((item) => item.key === 'future')?.raw).toEqual(future)
    expect(yield* reopened.read('future').pipe(Effect.flip)).toMatchObject({ code: 'quarantined' })
    expect(yield* reopened.acquireDispatchPermit('future', 0).pipe(Effect.flip)).toMatchObject({ code: 'quarantined' })
    yield* reopened.cleanupTerminal()
    expect(yield* reopened.listQuarantined()).toEqual(quarantine)
  }))

  it.effect('quota failure refuses new acceptance without evicting existing unresolved data or memory fallback', () => Effect.gen(function* () {
    const factory = new IDBFactory()
    const repo = yield* open(factory)
    yield* repo.commit(attempt('unresolved'))
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      if (this.name === attemptsStore) throw new DOMException('Quota exhausted', 'QuotaExceededError')
      return add.call(this, value, key)
    })
    expect(yield* repo.commit(attempt('new')).pipe(Effect.flip)).toMatchObject({ _tag: 'OutboxStorageError', code: 'quota' })
    expect(yield* repo.read('new')).toBeUndefined()
    expect(yield* repo.read('unresolved')).toEqual(attempt('unresolved'))
    repo.close()
    const reopened = yield* open(factory)
    expect(yield* reopened.read('new')).toBeUndefined()
    expect(yield* reopened.list(scope)).toHaveLength(1)
  }))

  it.effect('disabled and denied storage surface typed errors without a memory repository', () => Effect.gen(function* () {
    for (const [name, code] of [['InvalidStateError', 'disabled'], ['SecurityError', 'denied']] as const) {
      const result = yield* openIndexedDbOutbox({ indexedDB: { open: () => { throw new DOMException('Unavailable', name) } } }).pipe(Effect.flip)
      expect(result).toMatchObject({ _tag: 'OutboxStorageError', code })
    }
  }))

  it.effect('requests persistence and reports granted, denied and unsupported honestly', () => Effect.gen(function* () {
    for (const granted of [true, false]) {
      const persist = vi.fn(async () => granted)
      const repo = yield* openIndexedDbOutbox({ indexedDB: new IDBFactory(), storage: { persist } })
      connections.push(repo)
      expect(persist).toHaveBeenCalledTimes(1)
      expect(repo.persistence).toBe(granted ? 'granted' : 'denied')
    }
    expect((yield* open(new IDBFactory())).persistence).toBe('unsupported')
    expect(yield* openIndexedDbOutbox({ indexedDB: new IDBFactory(), storage: { persist: async () => { throw new DOMException('Denied', 'NotAllowedError') } } }).pipe(Effect.flip)).toMatchObject({ code: 'denied' })
  }))

  it.effect('scope isolation excludes another account, member, deployment, profile or action kind', () => Effect.gen(function* () {
    const repo = yield* open(new IDBFactory())
    yield* repo.commit(attempt())
    for (const other of [
      { ...scope, account: 'other' }, { ...scope, member: 'other' }, { ...scope, deployment: 'other' },
      { ...scope, profile: 'other' }, { ...scope, kind: 'thread-start' as const },
    ]) expect(yield* repo.list(other)).toEqual([])
  }))

  it.effect('retention never expires unresolved; terminal content lasts 7d and tombstones another 30d', () => Effect.gen(function* () {
    const day = 86_400_000
    let now = 100
    const repo = yield* open(new IDBFactory(), () => now)
    const image = yield* spoolAttachment(new Uint8Array([3, 4]))
    const record = { ...attempt('terminal'), intent: { bytes: [1], attachments: [{ hash: image.hash, name: 'image.png', mediaType: 'image/png' }] } }
    yield* repo.commit(record, [image])
    yield* repo.commit(attempt('unresolved'))
    const finished: OutboxRecord = { ...record, revision: 1, state: { status: 'not-applied' }, terminalAt: now }
    yield* repo.compareAndSwap(0, finished)
    now += 7 * day - 1
    yield* repo.cleanupTerminal()
    expect((yield* repo.read('terminal'))?.intent).toBeDefined()
    expect(yield* repo.readSpool('terminal', image.hash)).toEqual(image.bytes)
    now += 1
    yield* repo.cleanupTerminal()
    const tombstone = yield* repo.read('terminal')
    expect(tombstone).toMatchObject({ tombstoneAt: now, revision: 2, state: { status: 'not-applied' } })
    expect(tombstone?.intent).toBeUndefined()
    expect(yield* repo.readSpool('terminal', image.hash)).toBeUndefined()
    now += 30 * day - 1
    yield* repo.cleanupTerminal()
    expect(yield* repo.read('terminal')).toBeDefined()
    now += 1
    yield* repo.cleanupTerminal()
    expect(yield* repo.read('terminal')).toBeUndefined()
    expect(yield* repo.read('unresolved')).toEqual(attempt('unresolved'))
    expect(retentionAction({ ...attempt(), state: { status: 'unknown' }, mayHaveDispatched: true }, Number.MAX_SAFE_INTEGER)).toBe('keep')
    expect(retentionAction({ ...finished, state: { status: 'delivered' }, timelineObserved: false }, now)).toBe('keep')
  }))

  it.effect('explicit dismissal removes unknown content but never claims cancellation or nonapplication', () => Effect.gen(function* () {
    const repo = yield* open(new IDBFactory())
    const record = yield* repo.commit(attempt())
    yield* repo.compareAndSwap(0, ready(record))
    const dispatched = yield* repo.acquireDispatchPermit(record.localAttemptId, 1)
    yield* repo.compareAndSwap(2, { ...dispatched, revision: 3, state: { status: 'unknown' }, dismissed: { at: 100 } })
    yield* repo.cleanupTerminal()
    expect(yield* repo.read(record.localAttemptId)).toMatchObject({ state: { status: 'unknown' }, mayHaveDispatched: true, dismissed: { at: 100 }, tombstoneAt: 100 })
    expect((yield* repo.read(record.localAttemptId))?.intent).toBeUndefined()
  }))

  it.effect('rejects rewriting intent, prepared bytes or dispatch history', () => Effect.gen(function* () {
    const repo = yield* open(new IDBFactory())
    const record = yield* repo.commit(attempt())
    expect(yield* repo.compareAndSwap(0, { ...ready(record), intent: { bytes: [9], attachments: [] } }).pipe(Effect.flip)).toMatchObject({ code: 'invalid-record' })
    yield* repo.compareAndSwap(0, ready(record))
    expect(yield* repo.compareAndSwap(1, { ...ready(record), revision: 2, preparedPacket: { ...packet, bytes: [9] } }).pipe(Effect.flip)).toMatchObject({ code: 'invalid-record' })
    const dispatched = yield* repo.acquireDispatchPermit(record.localAttemptId, 1)
    expect(yield* repo.compareAndSwap(2, { ...dispatched, revision: 3, state: { status: 'pending', phase: 'ready' }, mayHaveDispatched: false }).pipe(Effect.flip)).toMatchObject({ code: 'invalid-record' })
  }))
})
