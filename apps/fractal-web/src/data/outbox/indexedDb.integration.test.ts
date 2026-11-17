import 'fake-indexeddb/auto'
import { createOutboxRecord, observeOutbox, prepareMessageSendPacket, prepareOutboxRecord, type OutboxRecord, type OutboxScope, type PreparedPacket } from '@fractal/client'
import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { Effect, Schema } from 'effect'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { attemptsStore, defaultDatabaseName, openIndexedDbOutbox, spoolAttachment, type IndexedDbOutbox } from './indexedDb.ts'
import { OutboxConflict, retentionAction } from './repository.ts'

const scope = { principalId: 'person/a', deploymentId: 'deployment/a', memberId: 'member/a' } as const
const packet = prepareMessageSendPacket({
  id: 'action/a', idempotency_key: 'key/original-attempt-a', fence: { snapshot_id: 'snapshot/a', subject_revisions: {} },
  parameters: { to: 'agent/a', content: 'Exact café body', session_id: 'session/original', tags: ['tag/a'], in_reply_to: 'message/original' },
})
const expectedIdentities = ['message/a']
const attempt = (id = 'attempt/a') => createOutboxRecord(id, { text: 'Exact café body', attachments: [] })
const ready = (record: OutboxRecord<PreparedPacket>) => {
  const proposal = prepareOutboxRecord(record, packet, expectedIdentities)
  if (proposal === undefined) throw new TypeError('Fixture must be preparing.')
  return proposal
}
const connections: IndexedDbOutbox[] = []
const open = async (factory: IDBFactory, now = () => 100, owner: OutboxScope = scope): Promise<IndexedDbOutbox> => {
  const repo = await Effect.runPromise(openIndexedDbOutbox({ scope: owner, indexedDB: factory, now }))
  connections.push(repo)
  return repo
}
const unknownAttempt = async (repo: IndexedDbOutbox) => {
  const record = attempt()
  expect(await repo.commit(record)).toBe(true)
  expect(await repo.compareAndSwap(0, ready(record))).toBe(true)
  const dispatched = await repo.acquireDispatchPermit(record.id, 1)
  const unknown = observeOutbox(dispatched, { kind: 'failure', applied: 'unknown' })
  expect(await repo.compareAndSwap(2, unknown)).toBe(true)
  return unknown
}
/** Raw access is test-only: also proves old/corrupt rows are retained, not silently dropped. */
const raw = (factory: IDBFactory, write: readonly unknown[] = []): Promise<readonly unknown[]> => {
  const { promise, resolve, reject } = Promise.withResolvers<readonly unknown[]>()
  const request = factory.open(defaultDatabaseName, 1)
  request.onerror = () => reject(request.error)
  request.onsuccess = () => {
    const db = request.result
    const tx = db.transaction(attemptsStore, write.length === 0 ? 'readonly' : 'readwrite')
    let result: readonly unknown[] = []
    tx.oncomplete = () => { db.close(); resolve(result) }
    tx.onabort = () => { db.close(); reject(tx.error) }
    for (const value of write) tx.objectStore(attemptsStore).put(value)
    const rows = tx.objectStore(attemptsStore).getAll()
    rows.onsuccess = () => { result = rows.result }
  }
  return promise
}
const imageAttempt = async (id = 'attempt/a') => {
  const image = await Effect.runPromise(spoolAttachment(new Uint8Array([0, 255, 1, 128])))
  const record = createOutboxRecord(id, { text: 'image', attachments: [{ contentHash: image.hash, name: 'image.png', mediaType: 'image/png', reference: `spool:${image.hash}` }] })
  return { image, record }
}
afterEach(() => { for (const connection of connections.splice(0)) connection.close(); vi.restoreAllMocks() })

describe('durable core IndexedDB outbox', () => {
  it('acknowledges only after transaction completion, never request success', async () => {
    const factory = new IDBFactory()
    const writer = await open(factory)
    const reader = await open(factory)
    let acknowledged = false
    let requestSucceeded = false
    let completed = false
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) {
        request.addEventListener('success', () => { requestSucceeded = true; expect(acknowledged).toBe(false) })
        this.transaction.addEventListener('complete', () => { completed = true })
      }
      return request
    })
    expect(await reader.read('attempt/a')).toBeUndefined()
    expect(await writer.commit(attempt())).toBe(true)
    acknowledged = true
    expect(requestSucceeded).toBe(true)
    expect(completed).toBe(true)
    expect(await reader.read('attempt/a')).toEqual(attempt())
    expect(await writer.commit(attempt())).toBe(false)
  })

  it('aborted acceptance leaves neither intent, full request metadata nor attachment bytes', async () => {
    const repo = await open(new IDBFactory())
    const { image, record } = await imageAttempt()
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === attemptsStore) request.addEventListener('success', () => this.transaction.abort())
      return request
    })
    await expect(repo.accept(record, [image], new Uint8Array([9]))).rejects.toMatchObject({ _tag: 'OutboxStorageError' })
    expect(await repo.read(record.id)).toBeUndefined()
    expect(await repo.readIntentBytes(record.id)).toBeUndefined()
    expect(await repo.readSpool(record.id, image.hash)).toBeUndefined()
  })

  it('requires every hashed attachment atomically and rejects incorrect hashes', async () => {
    const repo = await open(new IDBFactory())
    const { image, record } = await imageAttempt()
    await expect(repo.commit(record)).rejects.toMatchObject({ code: 'invalid-record' })
    await expect(repo.accept(record, [{ ...image, bytes: new Uint8Array([9]) }])).rejects.toMatchObject({ code: 'invalid-record' })
    await expect(repo.accept(attempt(), [image])).rejects.toMatchObject({ code: 'invalid-record' })
    expect(await repo.list()).toEqual([])
  })

  it('stale core CAS returns false; two connections race for exactly one typed dispatch permit', async () => {
    const factory = new IDBFactory()
    const first = await open(factory)
    const second = await open(factory)
    const record = attempt()
    await first.commit(record)
    expect(await first.compareAndSwap(0, ready(record))).toBe(true)
    expect(await first.compareAndSwap(0, ready(record))).toBe(false)
    const results = await Promise.allSettled([first, second].map((repo) => repo.acquireDispatchPermit(record.id, 1)))
    expect(results.filter((result) => result.status === 'fulfilled')).toHaveLength(1)
    const loser = results.find((result) => result.status === 'rejected')
    expect(loser?.status === 'rejected' ? loser.reason : undefined).toEqual(new OutboxConflict({ localAttemptId: record.id, expectedRevision: 1, actualRevision: 2 }))
    expect(await first.read(record.id)).toMatchObject({ mayHaveDispatched: true, state: 'pending', phase: 'ready' })
  })

  it('reopen preserves exact core record, unsigned JSON body, image and immutable full intent bytes', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const { image, record } = await imageAttempt()
    const intentBytes = new Uint8Array([255, 0, 127])
    expect(await repo.accept(record, [image], intentBytes)).toBe(true)
    intentBytes.fill(0)
    const prepared = ready(record)
    await repo.compareAndSwap(0, prepared)
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.read(record.id)).toEqual(prepared)
    expect(await reopened.readSpool(record.id, image.hash)).toEqual(image.bytes)
    expect(await reopened.readIntentBytes(record.id)).toEqual(new Uint8Array([255, 0, 127]))
    const networkBody = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown))(packet)
    expect(new TextDecoder().decode(await reopened.readPacketBytes(record.id))).toBe(networkBody)
    expect(Object.hasOwn(packet.parameters, 'signature')).toBe(false)
    expect(await reopened.accept(record, [image], new Uint8Array([9]))).toBe(false)
    expect(await reopened.readIntentBytes(record.id)).toEqual(new Uint8Array([255, 0, 127]))
  })

  it('startup recovers dispatched as unknown and preserves never-dispatched preparing/ready', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    for (const id of ['dispatched', 'preparing', 'ready']) await repo.commit(attempt(id))
    await repo.compareAndSwap(0, ready(attempt('dispatched')))
    await repo.acquireDispatchPermit('dispatched', 1)
    await repo.compareAndSwap(0, ready(attempt('ready')))
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.read('dispatched')).toMatchObject({ state: 'unknown', ambiguous: true, mayHaveDispatched: true, revision: 3 })
    expect(await reopened.read('preparing')).toEqual(attempt('preparing'))
    expect(await reopened.read('ready')).toEqual(ready(attempt('ready')))
    await expect(reopened.acquireDispatchPermit('dispatched', 3)).rejects.toBeInstanceOf(OutboxConflict)
  })

  it('unknown settles via exact expected timeline presence and remains delivered after restart', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const unknown = await unknownAttempt(repo)
    const delivered = observeOutbox(unknown, { kind: 'observed', identity: 'message/a', source: 'timeline' })
    expect(await repo.compareAndSwap(3, delivered)).toBe(true)
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.read(unknown.id)).toEqual(delivered)
    expect(await raw(factory)).toEqual([expect.objectContaining({ terminalAt: 100, timelineObserved: true })])
  })

  it('unknown never returns to pending or not-applied, nor delivers without matching evidence', async () => {
    const repo = await open(new IDBFactory())
    const unknown = await unknownAttempt(repo)
    if (!('packet' in unknown)) throw new TypeError('Expected prepared fixture.')
    const { state: _state, ...history } = unknown
    const proposals: readonly unknown[] = [
      { ...history, revision: 4, state: 'pending', phase: 'ready' },
      { ...history, revision: 4, state: 'not-applied', proof: { applied: 'none' } },
      { ...history, revision: 4, state: 'delivered' },
      { ...history, revision: 4, state: 'delivered', evidence: { kind: 'timeline', identity: 'message/other', source: 'timeline' } },
      { ...history, revision: 4, state: 'delivered', evidence: { kind: 'timeline', identity: 'message/a', source: 'absence' } },
    ]
    for (const proposal of proposals) {
      // Deliberately cross the untrusted boundary with malformed caller proposals.
      await expect(repo.compareAndSwap(3, proposal as OutboxRecord<PreparedPacket>)).rejects.toBeDefined()
    }
    expect(observeOutbox(unknown, { kind: 'failure', applied: 'none' })).toBe(unknown)
    expect(await repo.read(unknown.id)).toEqual(unknown)
  })

  it('persists core completed receipt evidence without expiring delivery before timeline observation', async () => {
    const factory = new IDBFactory()
    let now = 100
    const repo = await open(factory, () => now)
    const unknown = await unknownAttempt(repo)
    // The caller's receipt validator owns correlation; storage validates its wire shape.
    const delivered = observeOutbox(unknown, { kind: 'delivered', receipt: {
      kind: 'action-result', action_id: packet.id, operation_id: 'operation/a',
      affected_ids: ['message/a'], snapshot_id: 'snapshot/receipt', status: 'completed',
    } })
    expect(await repo.compareAndSwap(unknown.revision, delivered)).toBe(true)
    now += 100 * 86_400_000
    await repo.cleanupTerminal()
    expect(await repo.read(unknown.id)).toEqual(delivered)
    repo.close()
    const reopened = await open(factory, () => now)
    expect(await reopened.read(unknown.id)).toEqual(delivered)
    expect(await raw(factory)).toEqual([expect.objectContaining({ terminalAt: 100, timelineObserved: false })])
  })

  it('terminal evidence cannot introduce its own unpersisted or replacement packet', async () => {
    const repo = await open(new IDBFactory())
    const record = attempt()
    await repo.commit(record)
    const delivered = observeOutbox(ready(record), { kind: 'observed', identity: 'message/a', source: 'timeline' })
    if (!('packet' in delivered)) throw new Error('Expected prepared evidence')
    await expect(repo.compareAndSwap(0, { ...delivered, revision: 1 })).rejects.toMatchObject({ code: 'invalid-record' })
    expect(await repo.read(record.id)).toEqual(record)
    await repo.compareAndSwap(0, ready(record))
    await expect(repo.compareAndSwap(1, { ...delivered, packet: { ...packet, idempotency_key: 'key/different-attempt-a' } })).rejects.toMatchObject({ code: 'invalid-record' })
  })

  it('corrupt/future/old mirror rows stay quarantined, retained and never dispatchable', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const future = { localAttemptId: 'future', scope, schemaVersion: 999, secret: 'same-owner-data' }
    const corrupt = { localAttemptId: 'corrupt', scope, schemaVersion: 2 }
    const legacy = { localAttemptId: 'legacy', scope: { account: 'old-account' }, schemaVersion: 1, intent: { bytes: [9] } }
    await raw(factory, [future, corrupt, legacy])
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.list()).toEqual([])
    expect(await reopened.listQuarantined()).toEqual(expect.arrayContaining([
      { key: 'future', reason: 'corrupt-or-unsupported', raw: future },
      { key: 'corrupt', reason: 'corrupt-or-unsupported', raw: corrupt },
      { key: 'legacy', reason: 'corrupt-or-unsupported' },
    ]))
    await expect(reopened.read('future')).rejects.toMatchObject({ code: 'quarantined' })
    await expect(reopened.acquireDispatchPermit('future', 0)).rejects.toMatchObject({ code: 'quarantined' })
    await reopened.cleanupTerminal()
    expect(await raw(factory)).toHaveLength(3)
  })

  it('quota failures refuse acceptance without evicting unresolved data or memory fallback', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    await repo.commit(attempt('unresolved'))
    const add = IDBObjectStore.prototype.add
    vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      if (this.name === attemptsStore) throw new DOMException('Quota exhausted', 'QuotaExceededError')
      return add.call(this, value, key)
    })
    await expect(repo.commit(attempt('new'))).rejects.toMatchObject({ _tag: 'OutboxStorageError', code: 'quota' })
    expect(await repo.read('new')).toBeUndefined()
    repo.close()
    const reopened = await open(factory)
    expect(await reopened.read('new')).toBeUndefined()
    expect(await reopened.list()).toEqual([attempt('unresolved')])
  })

  it('disabled/denied storage reports typed errors and persistence is reported honestly', async () => {
    for (const [name, code] of [['InvalidStateError', 'disabled'], ['SecurityError', 'denied']] as const) {
      const failure = await Effect.runPromise(Effect.flip(openIndexedDbOutbox({ scope, indexedDB: { open: () => { throw new DOMException('Unavailable', name) } } })))
      expect(failure).toMatchObject({ _tag: 'OutboxStorageError', code })
    }
    for (const granted of [true, false]) {
      const persist = vi.fn(async () => granted)
      const repo = await Effect.runPromise(openIndexedDbOutbox({ scope, indexedDB: new IDBFactory(), storage: { persist } }))
      connections.push(repo)
      expect(persist).toHaveBeenCalledTimes(1)
      expect(repo.persistence).toBe(granted ? 'granted' : 'denied')
    }
    expect((await open(new IDBFactory())).persistence).toBe('unsupported')
    expect(await Effect.runPromise(Effect.flip(openIndexedDbOutbox({ scope, indexedDB: new IDBFactory(), storage: { persist: async () => { throw new DOMException('Denied', 'NotAllowedError') } } })))).toMatchObject({ code: 'denied' })
  })

  it('fixed scope protects every read, list, spool, packet, intent, quarantine and cleanup path', async () => {
    const factory = new IDBFactory()
    const owner = await open(factory)
    const { image, record } = await imageAttempt()
    await owner.accept(record, [image], new Uint8Array([9]))
    await owner.compareAndSwap(0, ready(record))
    await raw(factory, [{ localAttemptId: 'owner-corrupt', scope, schemaVersion: 999, secret: 'private' }, { localAttemptId: 'unknown-owner', secret: 'private' }])
    for (const otherScope of [{ ...scope, principalId: 'other' }, { ...scope, deploymentId: 'other' }, { ...scope, memberId: 'other' }]) {
      const other = await open(factory, () => Number.MAX_SAFE_INTEGER, otherScope)
      expect(await other.list()).toEqual([])
      expect(await other.read(record.id)).toBeUndefined()
      expect(await other.readSpool(record.id, image.hash)).toBeUndefined()
      expect(await other.readPacketBytes(record.id)).toBeUndefined()
      expect(await other.readIntentBytes(record.id)).toBeUndefined()
      expect(await other.listQuarantined()).toEqual([{ key: 'unknown-owner', reason: 'corrupt-or-unsupported' }])
      expect(await other.compareAndSwap(1, observeOutbox(ready(record), { kind: 'observed', identity: 'message/a', source: 'timeline' }))).toBe(false)
      await expect(other.acquireDispatchPermit(record.id, 1)).rejects.toBeInstanceOf(OutboxConflict)
      await other.dismiss(record.id)
      await other.cleanupTerminal()
    }
    expect(await owner.readSpool(record.id, image.hash)).toEqual(image.bytes)
    expect(await owner.read(record.id)).toEqual(ready(record))
  })

  it('terminal content lasts 7d, tombstones another 30d; unresolved and receipt-only delivery never expire', async () => {
    const day = 86_400_000
    let now = 100
    const factory = new IDBFactory()
    const repo = await open(factory, () => now)
    const { image, record } = await imageAttempt('terminal')
    await repo.accept(record, [image], new Uint8Array([9]))
    await repo.commit(attempt('unresolved'))
    await repo.compareAndSwap(0, ready(record))
    const dispatched = await repo.acquireDispatchPermit(record.id, 1)
    const finished = observeOutbox(dispatched, { kind: 'failure', applied: 'none' })
    await repo.compareAndSwap(2, finished)
    now += 7 * day - 1
    await repo.cleanupTerminal()
    expect(await repo.read(record.id)).toEqual(finished)
    expect(await repo.readSpool(record.id, image.hash)).toEqual(image.bytes)
    now += 1
    await repo.cleanupTerminal()
    expect(await repo.read(record.id)).toBeUndefined()
    expect(await repo.readIntentBytes(record.id)).toBeUndefined()
    expect(await repo.readPacketBytes(record.id)).toBeUndefined()
    expect(await repo.readSpool(record.id, image.hash)).toBeUndefined()
    expect(await raw(factory)).toEqual(expect.arrayContaining([expect.objectContaining({ localAttemptId: record.id, tombstoneAt: now, terminalAt: 100 })]))
    now += 30 * day - 1
    await repo.cleanupTerminal()
    expect(await raw(factory)).toHaveLength(2)
    now += 1
    await repo.cleanupTerminal()
    expect(await raw(factory)).toHaveLength(1)
    expect(await repo.read('unresolved')).toEqual(attempt('unresolved'))
    expect(retentionAction({ timelineObserved: false }, Number.MAX_SAFE_INTEGER)).toBe('keep')
    expect(retentionAction({ terminalAt: 100, timelineObserved: false }, Number.MAX_SAFE_INTEGER)).toBe('keep')
  })

  it('explicit dismissal strips unknown content without claiming cancellation or nonapplication', async () => {
    const factory = new IDBFactory()
    const repo = await open(factory)
    const unknown = await unknownAttempt(repo)
    await repo.dismiss(unknown.id)
    await expect(repo.acquireDispatchPermit(unknown.id, unknown.revision)).rejects.toBeInstanceOf(OutboxConflict)
    await repo.cleanupTerminal()
    expect(await repo.read(unknown.id)).toBeUndefined()
    expect(await raw(factory)).toEqual([expect.objectContaining({ localAttemptId: unknown.id, dismissed: { at: 100 }, tombstoneAt: 100, tombstoneHistory: { state: 'unknown', revision: 3, mayHaveDispatched: true, ambiguous: true } })])
    expect(await raw(factory)).toEqual([expect.not.objectContaining({ record: expect.anything() })])
  })

  it('core authorization rejects rewriting intent, expected identities, prepared bytes or dispatch history', async () => {
    const repo = await open(new IDBFactory())
    const record = attempt()
    await repo.commit(record)
    await expect(repo.compareAndSwap(0, { ...ready(record), intent: { text: 'changed', attachments: [] } })).rejects.toMatchObject({ code: 'invalid-record' })
    await repo.compareAndSwap(0, ready(record))
    await expect(repo.compareAndSwap(1, { ...ready(record), revision: 2, packet: { ...packet, idempotency_key: 'key/changed-attempt-a' } })).rejects.toMatchObject({ code: 'invalid-record' })
    await expect(repo.compareAndSwap(1, { ...ready(record), revision: 2, expectedIdentities: ['message/changed'] })).rejects.toMatchObject({ code: 'invalid-record' })
    const dispatched = await repo.acquireDispatchPermit(record.id, 1)
    await expect(repo.compareAndSwap(2, { ...dispatched, revision: 3, mayHaveDispatched: false })).rejects.toMatchObject({ code: 'invalid-record' })
  })
})
