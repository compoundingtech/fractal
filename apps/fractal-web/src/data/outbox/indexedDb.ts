import { Effect, Schema } from 'effect'
import {
  OutboxConflict, OutboxRecord, OutboxStorageError, retentionAction, sameScope, terminal,
  type OutboxError, type OutboxRepository, type OutboxScope, type QuarantinedAttempt, type SpoolInput,
} from './repository.ts'

export const attemptsStore = 'attempts-v1'
export const spoolStore = 'spool-v1'
export const defaultDatabaseName = 'fractal-web-outbox-v1'
const decode = Schema.decodeUnknownSync(OutboxRecord, { onExcessProperty: 'error' })
const error = (cause: unknown, operation: string): OutboxError => {
  if (cause instanceof OutboxStorageError || cause instanceof OutboxConflict) return cause
  const name = cause instanceof Error || cause instanceof DOMException ? cause.name : ''
  const code = name === 'QuotaExceededError' ? 'quota'
    : name === 'SecurityError' || name === 'NotAllowedError' ? 'denied'
    : name === 'InvalidStateError' || name === 'NotSupportedError' ? 'disabled' : 'storage'
  return new OutboxStorageError({ code, operation, message: `Outbox ${operation} failed (${name || 'storage error'}).` })
}
const invalid = (message: string): OutboxStorageError => new OutboxStorageError({ code: 'invalid-record', operation: 'validate', message })
const equalBytes = (left: readonly number[], right: readonly number[]): boolean =>
  left.length === right.length && left.every((byte, index) => byte === right[index])
const validRecord = (raw: unknown): OutboxRecord => {
  const record = decode(raw)
  if (record.tombstoneAt === undefined && record.intent === undefined) throw invalid('Live attempts require immutable intent.')
  if (record.state.status === 'pending' && record.tombstoneAt === undefined && record.state.phase !== 'preparing' && record.preparedPacket === undefined) throw invalid('Ready/dispatched attempts require the exact packet.')
  if (record.state.status === 'pending' && (record.state.phase === 'dispatched') !== record.mayHaveDispatched) throw invalid('Pending dispatch marker contradicts phase.')
  if (record.state.status === 'unknown' && !record.mayHaveDispatched) throw invalid('Unknown requires a dispatch marker.')
  if (terminal(record) && record.terminalAt === undefined) throw invalid('Terminal attempts require terminalAt.')
  if (record.tombstoneAt !== undefined && record.dismissed === undefined && !terminal(record)) throw invalid('Only terminal or explicitly dismissed content can be removed.')
  return record
}
const inspect = (raw: unknown): OutboxRecord | undefined => {
  try { return validRecord(raw) } catch { return undefined }
}

/** Hash image bytes before accepting them; commit owns the durable copy. */
export const spoolAttachment = Effect.fn('Outbox.spoolAttachment')(function* (bytes: Uint8Array) {
  const owned = new Uint8Array(bytes)
  const digest = yield* Effect.tryPromise({
    try: () => globalThis.crypto.subtle.digest('SHA-256', owned),
    catch: (cause) => error(cause, 'hashAttachment'),
  })
  return { hash: Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join(''), bytes: owned }
})

/** IndexedDB owns durability. No memory fallback; browser eviction/site-data deletion remain possible. */
export interface IndexedDbOutbox extends OutboxRepository {
  readonly persistence: 'granted' | 'denied' | 'unsupported'
  readonly acquireDispatchPermit: (id: string, revision: number) => Effect.Effect<OutboxRecord, OutboxError>
  readonly readSpool: (id: string, hash: string) => Effect.Effect<Uint8Array | undefined, OutboxError>
  readonly listQuarantined: () => Effect.Effect<readonly QuarantinedAttempt[], OutboxError>
  readonly cleanupTerminal: () => Effect.Effect<void, OutboxError>
  readonly close: () => void
}
export interface OpenOutboxOptions {
  readonly databaseName?: string
  readonly indexedDB?: Pick<IDBFactory, 'open'>
  readonly storage?: Pick<StorageManager, 'persist'>
  readonly now?: () => number
}

const transaction = <T>(db: IDBDatabase, mode: IDBTransactionMode, operation: string,
  work: (tx: IDBTransaction, done: (value: T) => void, fail: (cause: unknown) => void) => void,
): Promise<T> => {
  const { promise, resolve, reject } = Promise.withResolvers<T>()
  let tx: IDBTransaction
  try {
    try { tx = db.transaction([attemptsStore, spoolStore], mode, { durability: 'strict' }) }
    catch (cause) {
      // Older browsers reject the options overload. Do not retry real storage errors.
      if (!(cause instanceof TypeError)) throw cause
      tx = db.transaction([attemptsStore, spoolStore], mode)
    }
  } catch (cause) { reject(error(cause, operation)); return promise }
  let result: T
  let failure: OutboxError | undefined
  const fail = (cause: unknown): void => {
    failure = error(cause, operation)
    try { tx.abort() } catch { reject(failure) }
  }
  tx.oncomplete = () => resolve(result)
  tx.onabort = () => reject(failure ?? error(tx.error, operation))
  tx.onerror = () => { /* The abort event is the failure barrier. */ }
  try { work(tx, (value) => { result = value }, fail) } catch (cause) { fail(cause) }
  return promise
}

const makeRepository = (db: IDBDatabase, persistence: IndexedDbOutbox['persistence'], now: () => number): IndexedDbOutbox => {
  const run = <T>(mode: IDBTransactionMode, operation: string,
    work: Parameters<typeof transaction<T>>[3],
  ): Effect.Effect<T, OutboxError> => Effect.tryPromise({
    try: () => transaction(db, mode, operation, work), catch: (cause) => error(cause, operation),
  }).pipe(Effect.withSpan(`Outbox.${operation}`))
  const change = (id: string, revision: number, update: (record: OutboxRecord) => OutboxRecord): Effect.Effect<OutboxRecord, OutboxError> =>
    run('readwrite', 'compareAndSwap', (tx, done, fail) => {
      const store = tx.objectStore(attemptsStore)
      const request = store.get(id)
      request.onsuccess = () => {
        try {
          const current = request.result === undefined ? undefined : inspect(request.result)
          if (request.result !== undefined && current === undefined) throw new OutboxStorageError({ code: 'quarantined', operation: 'compareAndSwap', message: 'Attempt is quarantined.' })
          if (current === undefined || current.revision !== revision) throw new OutboxConflict({ localAttemptId: id, expectedRevision: revision, ...(current === undefined ? {} : { actualRevision: current.revision }) })
          const next = validRecord(update(current))
          if (next.localAttemptId !== id || next.revision !== revision + 1 || !sameScope(current.scope, next.scope)
            || next.createdAt !== current.createdAt || (current.mayHaveDispatched && !next.mayHaveDispatched)) throw invalid('CAS must preserve identity and monotone dispatch history, and increment revision.')
          if (current.tombstoneAt !== undefined || next.tombstoneAt !== undefined) throw invalid('Tombstones are managed only by cleanup.')
          if (current.intent === undefined || next.intent === undefined || !equalBytes(current.intent.bytes, next.intent.bytes)
            || current.intent.attachments.length !== next.intent.attachments.length
            || current.intent.attachments.some((attachment, index) => {
              const other = next.intent?.attachments[index]
              return other === undefined || attachment.hash !== other.hash || attachment.name !== other.name || attachment.mediaType !== other.mediaType
            })) throw invalid('Intent is immutable.')
          const packet = current.preparedPacket
          if (packet !== undefined) {
            const other = next.preparedPacket
            if (other === undefined || !equalBytes(packet.bytes, other.bytes) || packet.actionId !== other.actionId
              || packet.idempotencyKey !== other.idempotencyKey || packet.fingerprint.sha256 !== other.fingerprint.sha256
              || packet.expectedIdentities.length !== other.expectedIdentities.length
              || packet.expectedIdentities.some((value, index) => value !== other.expectedIdentities[index])) throw invalid('Prepared packet is immutable.')
          }
          if ((terminal(current) && next.state.status !== current.state.status)
            || (current.state.status === 'unknown' && next.state.status === 'pending')
            || (current.mayHaveDispatched && next.state.status === 'pending')) throw invalid('Dispatched attempts cannot acquire another send permit.')
          if (next.mayHaveDispatched && !current.mayHaveDispatched
            && !(current.state.status === 'pending' && current.state.phase === 'ready' && next.state.status === 'pending' && next.state.phase === 'dispatched')) throw invalid('Dispatch marker requires ready-to-dispatched transition.')
          store.put(next)
          done(next)
        } catch (cause) { fail(cause) }
      }
    })
  return {
    persistence,
    close: () => db.close(),
    commit: (record, spool = []) => Effect.gen(function* () {
      const ownedSpool: SpoolInput[] = []
      for (const item of spool) {
        const verified = yield* spoolAttachment(item.bytes)
        if (verified.hash !== item.hash) return yield* invalid('Spool key must match the attachment content hash.')
        ownedSpool.push(verified)
      }
      return yield* run<OutboxRecord>('readwrite', 'commit', (tx, done) => {
        const accepted = validRecord(record)
        if (accepted.revision !== 0 || accepted.mayHaveDispatched || accepted.state.status !== 'pending' || accepted.state.phase !== 'preparing' || accepted.tombstoneAt !== undefined || accepted.dismissed !== undefined) throw invalid('New attempts must begin never-dispatched and preparing at revision zero.')
        const attachments = accepted.intent?.attachments ?? []
        if (attachments.some((attachment) => !ownedSpool.some((item) => item.hash === attachment.hash))
          || ownedSpool.some((item) => !attachments.some((attachment) => attachment.hash === item.hash))) throw invalid('All attachment bytes must be committed with intent.')
        tx.objectStore(attemptsStore).add(accepted)
        for (const item of ownedSpool) tx.objectStore(spoolStore).add({ localAttemptId: accepted.localAttemptId, hash: item.hash, bytes: item.bytes })
        done(accepted)
      })
    }),
    compareAndSwap: (revision, record) => change(record.localAttemptId, revision, () => record),
    acquireDispatchPermit: (id, revision) => change(id, revision, (record) => {
      if (record.dismissed !== undefined || record.state.status !== 'pending' || record.state.phase !== 'ready' || record.mayHaveDispatched) throw invalid('Only a never-dispatched ready attempt may send.')
      return { ...record, revision: revision + 1, updatedAt: now(), mayHaveDispatched: true, state: { status: 'pending', phase: 'dispatched' } }
    }),
    read: (id) => run('readonly', 'read', (tx, done, fail) => {
      const request = tx.objectStore(attemptsStore).get(id)
      request.onsuccess = () => {
        const record = request.result === undefined ? undefined : inspect(request.result)
        if (request.result !== undefined && record === undefined) fail(new OutboxStorageError({ code: 'quarantined', operation: 'read', message: 'Attempt is quarantined.' }))
        else done(record)
      }
    }),
    list: (scope) => run('readonly', 'list', (tx, done) => {
      const request = tx.objectStore(attemptsStore).getAll()
      request.onsuccess = () => done(request.result.flatMap((raw: unknown) => {
        const record = inspect(raw)
        return record !== undefined && sameScope(scope, record.scope) ? [record] : []
      }))
    }),
    readSpool: (id, hash) => run('readonly', 'readSpool', (tx, done, fail) => {
      const request = tx.objectStore(spoolStore).get([id, hash])
      request.onsuccess = () => {
        const raw: unknown = request.result
        if (raw === undefined) done(undefined)
        else if (typeof raw === 'object' && raw !== null && 'bytes' in raw && raw.bytes instanceof Uint8Array) done(raw.bytes)
        else fail(new OutboxStorageError({ code: 'quarantined', operation: 'readSpool', message: 'Attachment bytes are corrupt.' }))
      }
    }),
    listQuarantined: () => run('readonly', 'listQuarantined', (tx, done) => {
      const records: QuarantinedAttempt[] = []
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        const cursor = request.result
        if (cursor === null) { done(records); return }
        if (inspect(cursor.value) === undefined) records.push({ key: cursor.primaryKey, reason: 'corrupt-or-unsupported', raw: cursor.value })
        cursor.continue()
      }
    }),
    cleanupTerminal: () => run('readwrite', 'cleanupTerminal', (tx, done, fail) => {
      const timestamp = now()
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        try {
          const cursor = request.result
          if (cursor === null) { done(undefined); return }
          const record = inspect(cursor.value)
          if (record !== undefined) {
            const action = retentionAction(record, timestamp)
            if (action === 'delete') cursor.delete()
            if (action === 'strip-content') {
              const { intent: _intent, preparedPacket: _packet, ...identity } = record
              cursor.update({ ...identity, revision: record.revision + 1, updatedAt: timestamp, tombstoneAt: timestamp })
              const spool = tx.objectStore(spoolStore).openCursor(IDBKeyRange.bound([record.localAttemptId], [record.localAttemptId, []]))
              spool.onsuccess = () => { const item = spool.result; if (item !== null) { item.delete(); item.continue() } }
            }
          }
          cursor.continue()
        } catch (cause) { fail(cause) }
      }
    }),
  }
}

export const openIndexedDbOutbox = Effect.fn('Outbox.open')(function* (options: OpenOutboxOptions = {}) {
  const factory = yield* Effect.try({ try: () => options.indexedDB ?? globalThis.indexedDB, catch: (cause) => error(cause, 'open') })
  if (factory === undefined) return yield* new OutboxStorageError({ code: 'disabled', operation: 'open', message: 'IndexedDB is unavailable; Send was not accepted.' })
  const storage = yield* Effect.try({ try: () => options.storage ?? globalThis.navigator?.storage, catch: (cause) => error(cause, 'persist') })
  const persistence = storage?.persist === undefined ? 'unsupported' : yield* Effect.tryPromise({
    try: () => storage.persist().then((granted) => granted ? 'granted' as const : 'denied' as const),
    catch: (cause) => error(cause, 'persist'),
  })
  const db = yield* Effect.tryPromise({
    try: () => {
      const { promise, resolve, reject } = Promise.withResolvers<IDBDatabase>()
      let blocked = false
      const request = factory.open(options.databaseName ?? defaultDatabaseName, 1)
      request.onupgradeneeded = () => {
        request.result.createObjectStore(attemptsStore, { keyPath: 'localAttemptId' })
        request.result.createObjectStore(spoolStore, { keyPath: ['localAttemptId', 'hash'] })
      }
      request.onerror = () => reject(request.error)
      request.onblocked = () => {
        blocked = true
        reject(new OutboxStorageError({ code: 'disabled', operation: 'open', message: 'Outbox database upgrade is blocked.' }))
      }
      request.onsuccess = () => { if (blocked) request.result.close(); else resolve(request.result) }
      return promise
    }, catch: (cause) => error(cause, 'open'),
  })
  db.onversionchange = () => db.close()
  // Serialize startup recovery with all other writers; never expose an old dispatch permit.
  yield* Effect.tryPromise({
    try: () => transaction<void>(db, 'readwrite', 'recover', (tx, done, fail) => {
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        try {
          const cursor = request.result
          if (cursor === null) { done(undefined); return }
          const record = inspect(cursor.value)
          if (record !== undefined && record.mayHaveDispatched && !terminal(record) && record.state.status !== 'unknown' && record.tombstoneAt === undefined) {
            cursor.update({ ...record, revision: record.revision + 1, updatedAt: (options.now ?? Date.now)(), state: { status: 'unknown' } })
          }
          cursor.continue()
        } catch (cause) { fail(cause) }
      }
    }).catch((cause: unknown) => { db.close(); throw cause }),
    catch: (cause) => error(cause, 'recover'),
  })
  return makeRepository(db, persistence, options.now ?? Date.now)
})
