import { isAllowedOutboxTransition, markOutboxDispatched, restoreOutboxRecord, type OutboxEntry, type OutboxPreparingRecord, type OutboxReceipt, type OutboxRecord, type OutboxRepository, type OutboxScope, type PreparedPacket } from '@fractal/client'
import { Effect, Schema } from 'effect'
import { loadAttachmentSchema } from '../lazySdk.ts'
import { Count, OutboxConflict, OutboxStorageError, ScopeSchema, retentionAction, sameScope, terminal, type OutboxError, type QuarantinedAttempt, type SpoolInput } from './repository.ts'

export const attemptsStore = 'attempts-v1'
export const spoolStore = 'spool-v1'
export const defaultDatabaseName = 'fractal-web-outbox-v1'
const Bytes = Schema.Array(Schema.Int.check(Schema.isBetween({ minimum: 0, maximum: 255 })))
const Intent = Schema.Struct({ text: Schema.String, attachments: Schema.Array(Schema.Struct({ contentHash: Schema.NonEmptyString, name: Schema.optionalKey(Schema.String), mediaType: Schema.NonEmptyString, reference: Schema.NonEmptyString })) })
const history = { id: Schema.NonEmptyString, revision: Count, intent: Intent, mayHaveDispatched: Schema.Boolean, ambiguous: Schema.Boolean }
const json = Schema.fromJsonString(Schema.Unknown)
const encodeJson = Schema.encodeSync(json)
const encodeBytes = (value: unknown): Uint8Array => new TextEncoder().encode(encodeJson(value))
const error = (cause: unknown, operation: string): OutboxError => {
  if (cause instanceof OutboxStorageError || cause instanceof OutboxConflict) return cause
  const name = cause instanceof Error || cause instanceof DOMException ? cause.name : ''
  const code = name === 'QuotaExceededError' ? 'quota' : name === 'SecurityError' || name === 'NotAllowedError' ? 'denied' : name === 'InvalidStateError' || name === 'NotSupportedError' ? 'disabled' : 'storage'
  return new OutboxStorageError({ code, operation, message: `Outbox ${operation} failed (${name || 'storage error'}).` })
}
const invalid = (message: string): OutboxStorageError => new OutboxStorageError({ code: 'invalid-record', operation: 'validate', message })
const quarantined = (): OutboxStorageError => new OutboxStorageError({ code: 'quarantined', operation: 'read', message: 'Attempt is quarantined.' })

/** Hash an owned copy before acceptance; a failed durable write never becomes an in-memory send. */
export const spoolAttachment = Effect.fn('Outbox.spoolAttachment')(function* (bytes: Uint8Array) {
  const owned = new Uint8Array(bytes)
  const digest = yield* Effect.tryPromise({ try: () => globalThis.crypto.subtle.digest('SHA-256', owned), catch: (cause) => error(cause, 'hashAttachment') })
  return { hash: Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join(''), bytes: owned }
})
export interface IndexedDbOutbox extends OutboxRepository<PreparedPacket> {
  readonly persistence: 'granted' | 'denied' | 'unsupported'
  readonly accept: (record: OutboxPreparingRecord, spool?: readonly SpoolInput[], intentBytes?: Uint8Array) => Promise<boolean>
  readonly acquireDispatchPermit: (id: string, revision: number) => Promise<OutboxRecord<PreparedPacket>>
  readonly readSpool: (id: string, hash: string) => Promise<Uint8Array | undefined>
  readonly readPacketBytes: (id: string) => Promise<Uint8Array | undefined>
  readonly readIntentBytes: (id: string) => Promise<Uint8Array | undefined>
  readonly listQuarantined: () => Promise<readonly QuarantinedAttempt[]>
  readonly cleanupTerminal: () => Promise<void>
  readonly recover: () => Promise<void>
  readonly dismiss: (id: string) => Promise<void>
  readonly close: () => void
}
export interface OpenOutboxOptions {
  readonly scope: OutboxScope
  readonly databaseName?: string
  readonly indexedDB?: Pick<IDBFactory, 'open'>
  readonly storage?: Pick<StorageManager, 'persist'>
  readonly now?: () => number
  /** Identity-aware callers explicitly validate current scope before recovery. */
  readonly recoverOnOpen?: boolean
}
interface PacketMetadata { readonly packetBytes: readonly number[]; readonly fingerprint: { readonly version: 1; readonly sha256: string } }
const transaction = <T>(db: IDBDatabase, mode: IDBTransactionMode, operation: string,
  work: (tx: IDBTransaction, done: (value: T) => void, fail: (cause: unknown) => void) => void,
): Promise<T> => {
  const { promise, resolve, reject } = Promise.withResolvers<T>()
  let tx: IDBTransaction
  try {
    try { tx = db.transaction([attemptsStore, spoolStore], mode, { durability: 'strict' }) }
    catch (cause) { if (!(cause instanceof TypeError)) throw cause; tx = db.transaction([attemptsStore, spoolStore], mode) }
  } catch (cause) { reject(error(cause, operation)); return promise }
  let result: T
  let failure: OutboxError | undefined
  const fail = (cause: unknown): void => { failure = error(cause, operation); try { tx.abort() } catch { reject(failure) } }
  tx.oncomplete = () => resolve(result)
  tx.onabort = () => reject(failure ?? error(tx.error, operation))
  tx.onerror = () => { /* Transaction abort, not request success, is the durability barrier. */ }
  try { work(tx, (value) => { result = value }, fail) } catch (cause) { fail(cause) }
  return promise
}

export const openIndexedDbOutbox = Effect.fn('Outbox.open')(function* (options: OpenOutboxOptions) {
  // This write codec stays behind the existing lazy SDK boundary.
  const { ActionRequest, ActionResult } = yield* loadAttachmentSchema.pipe(Effect.mapError((cause) => error(cause, 'schema')))
  const isAction = Schema.is(Schema.toEncoded(ActionRequest))
  const Packet = Schema.declare((value: unknown): value is PreparedPacket => isAction(value) && (value.type === 'message.send' || value.type === 'thread.start' || value.type === 'thread.reply'))
  const Receipt = Schema.declare((value: unknown): value is OutboxReceipt => Schema.is(Schema.toEncoded(ActionResult))(value) && value.status === 'completed')
  const prepared = { ...history, packet: Packet, expectedIdentities: Schema.Array(Schema.NonEmptyString) }
  const RecordSchema = Schema.Union([
    Schema.Struct({ ...history, state: Schema.Literal('pending'), phase: Schema.Literal('preparing') }),
    Schema.Struct({ ...prepared, state: Schema.Literal('pending'), phase: Schema.Literal('ready') }),
    Schema.Struct({ ...prepared, state: Schema.Literal('unknown') }),
    Schema.Struct({ ...prepared, state: Schema.Literal('not-applied'), proof: Schema.Struct({ applied: Schema.Literal('none') }) }),
    Schema.Struct({ ...prepared, state: Schema.Literal('delivered'), evidence: Schema.Union([
      Schema.Struct({ kind: Schema.Literal('receipt'), receipt: Receipt }),
      Schema.Struct({ kind: Schema.Literal('timeline'), identity: Schema.NonEmptyString, source: Schema.Literal('timeline') }),
    ]) }),
  ])
  const Envelope = Schema.Struct({
    schemaVersion: Schema.Literal(2), localAttemptId: Schema.NonEmptyString, scope: ScopeSchema,
    record: Schema.optionalKey(RecordSchema), createdAt: Count, updatedAt: Count,
    intentBytes: Schema.optionalKey(Bytes), packetBytes: Schema.optionalKey(Bytes),
    fingerprint: Schema.optionalKey(Schema.Struct({ version: Schema.Literal(1), sha256: Schema.String.check(Schema.isPattern(/^[0-9a-f]{64}$/u)) })),
    terminalAt: Schema.optionalKey(Count), timelineObserved: Schema.Boolean,
    diagnosticCode: Schema.optionalKey(Schema.NonEmptyString), dismissed: Schema.optionalKey(Schema.Struct({ at: Count })),
    tombstoneAt: Schema.optionalKey(Count),
    tombstoneHistory: Schema.optionalKey(Schema.Struct({ state: Schema.Literals(['pending', 'unknown', 'delivered', 'not-applied']), revision: Count, mayHaveDispatched: Schema.Boolean, ambiguous: Schema.Boolean })),
  }).annotate({ identifier: 'Outbox.StorageEnvelope.V2' })
  type Envelope = typeof Envelope.Type
  const decodeRecord = (raw: unknown): OutboxRecord<PreparedPacket> => {
    try { return Schema.decodeUnknownSync(RecordSchema, { onExcessProperty: 'error' })(raw) }
    catch { throw invalid('Attempt does not match the core record schema.') }
  }
  const decodeEnvelope = Schema.decodeUnknownSync(Envelope, { onExcessProperty: 'error' })
  const scope = Object.freeze(Schema.decodeUnknownSync(ScopeSchema)(options.scope))
  const now = options.now ?? Date.now
  const inspect = (raw: unknown): Envelope | undefined => {
    try {
      const envelope = decodeEnvelope(raw)
      const record = envelope.record
      if (envelope.tombstoneAt !== undefined) {
        if (envelope.tombstoneHistory === undefined || record !== undefined || envelope.intentBytes !== undefined || envelope.packetBytes !== undefined || envelope.fingerprint !== undefined || (envelope.terminalAt === undefined && envelope.dismissed === undefined)) return undefined
      } else {
        if (record === undefined || record.id !== envelope.localAttemptId) return undefined
        if ('packet' in record) {
          if (envelope.packetBytes === undefined || envelope.fingerprint === undefined || encodeJson(Array.from(encodeBytes(record.packet))) !== encodeJson(envelope.packetBytes)) return undefined
        } else if (envelope.packetBytes !== undefined || envelope.fingerprint !== undefined) return undefined
        if (terminal(record) !== (envelope.terminalAt !== undefined)) return undefined
      }
      return envelope
    } catch { return undefined }
  }
  const rawScope = (raw: unknown): OutboxScope | undefined => {
    if (typeof raw !== 'object' || raw === null || !('scope' in raw)) return undefined
    try { return Schema.decodeUnknownSync(ScopeSchema)(raw.scope) } catch { return undefined }
  }
  const factory = yield* Effect.try({ try: () => options.indexedDB ?? globalThis.indexedDB, catch: (cause) => error(cause, 'open') })
  if (factory === undefined) return yield* new OutboxStorageError({ code: 'disabled', operation: 'open', message: 'IndexedDB is unavailable; Send was not accepted.' })
  const storage = yield* Effect.try({ try: () => options.storage ?? globalThis.navigator?.storage, catch: (cause) => error(cause, 'persist') })
  const persistence = storage?.persist === undefined ? 'unsupported' : yield* Effect.tryPromise({ try: () => storage.persist().then((granted) => granted ? 'granted' as const : 'denied' as const), catch: (cause) => error(cause, 'persist') })
  const db = yield* Effect.tryPromise({ try: () => {
    const { promise, resolve, reject } = Promise.withResolvers<IDBDatabase>()
    let blocked = false
    const request = factory.open(options.databaseName ?? defaultDatabaseName, 1)
    request.onupgradeneeded = () => {
      request.result.createObjectStore(attemptsStore, { keyPath: 'localAttemptId' })
      request.result.createObjectStore(spoolStore, { keyPath: ['localAttemptId', 'hash'] })
    }
    request.onerror = () => reject(request.error)
    request.onblocked = () => { blocked = true; reject(new OutboxStorageError({ code: 'disabled', operation: 'open', message: 'Outbox database upgrade is blocked.' })) }
    request.onsuccess = () => { if (blocked) request.result.close(); else resolve(request.result) }
    return promise
  }, catch: (cause) => error(cause, 'open') })
  db.onversionchange = () => db.close()
  const run = <T>(mode: IDBTransactionMode, operation: string, work: Parameters<typeof transaction<T>>[3]): Promise<T> => transaction(db, mode, operation, work)
  const get = (tx: IDBTransaction, id: string, done: (value: Envelope | undefined) => void, fail: (cause: unknown) => void): void => {
    const request = tx.objectStore(attemptsStore).get(id)
    request.onsuccess = () => {
      try {
        const raw: unknown = request.result
        if (raw === undefined) { done(undefined); return }
        const owner = rawScope(raw)
        if (owner !== undefined && !sameScope(owner, scope)) { done(undefined); return }
        const envelope = inspect(raw)
        if (envelope === undefined) fail(quarantined())
        else done(envelope)
      } catch (cause) { fail(cause) }
    }
  }
  const packetMetadata = async (record: OutboxRecord<PreparedPacket>): Promise<PacketMetadata | undefined> => {
    if (!('packet' in record)) return undefined
    const encoded = await Effect.runPromise(spoolAttachment(encodeBytes(record.packet)))
    return { packetBytes: Array.from(encoded.bytes), fingerprint: { version: 1, sha256: encoded.hash } }
  }
  const accept: IndexedDbOutbox['accept'] = async (record, spool = [], intentBytes) => {
    // Capture caller-owned inputs before the first asynchronous hashing operation.
    const accepted = decodeRecord(Schema.decodeUnknownSync(json)(encodeJson(record)))
    const ownedIntentBytes = intentBytes === undefined ? undefined : Array.from(intentBytes)
    const inputs = spool.map((item) => ({ hash: item.hash, bytes: new Uint8Array(item.bytes) }))
    if (accepted.state !== 'pending' || accepted.phase !== 'preparing' || accepted.revision !== 0 || accepted.mayHaveDispatched || accepted.ambiguous) throw invalid('New attempts must be pristine preparing records.')
    const owned: SpoolInput[] = []
    for (const item of inputs) {
      const verified = await Effect.runPromise(spoolAttachment(item.bytes))
      if (verified.hash !== item.hash) throw invalid('Spool key must match its content hash.')
      if (!owned.some((other) => other.hash === verified.hash)) owned.push(verified)
    }
    if (accepted.intent.attachments.some((attachment) => !owned.some((item) => item.hash === attachment.contentHash)) || owned.some((item) => !accepted.intent.attachments.some((attachment) => attachment.contentHash === item.hash))) throw invalid('All attachment bytes must be accepted atomically with intent.')
    const timestamp = now()
    const envelope: Envelope = { schemaVersion: 2, localAttemptId: accepted.id, scope, record: accepted, createdAt: timestamp, updatedAt: timestamp, timelineObserved: false, ...(ownedIntentBytes === undefined ? {} : { intentBytes: ownedIntentBytes }) }
    return run('readwrite', 'accept', (tx, done, fail) => {
      const store = tx.objectStore(attemptsStore)
      const request = store.get(accepted.id)
      request.onsuccess = () => {
        try {
          if (request.result !== undefined) { done(false); return }
          store.add(envelope)
          for (const item of owned) tx.objectStore(spoolStore).add({ localAttemptId: accepted.id, hash: item.hash, bytes: item.bytes })
          done(true)
        } catch (cause) { fail(cause) }
      }
    })
  }
  const change = (id: string, revision: number, update: (record: OutboxRecord<PreparedPacket>) => OutboxRecord<PreparedPacket> | undefined, metadata?: PacketMetadata, conflictThrows = false): Promise<OutboxRecord<PreparedPacket> | undefined> => run('readwrite', 'compareAndSwap', (tx, done, fail) => {
    get(tx, id, (envelope) => {
      try {
        const current = envelope?.record
        const conflict = (): void => {
          if (conflictThrows) throw new OutboxConflict({ localAttemptId: id, expectedRevision: revision, ...(current === undefined ? {} : { actualRevision: current.revision }) })
          done(undefined)
        }
        if (envelope === undefined || current === undefined || current.revision !== revision || envelope.dismissed !== undefined) { conflict(); return }
        const proposal = update(current)
        if (proposal === undefined) { conflict(); return }
        const next = decodeRecord(proposal)
        // Revision and ALL policy authorization occur under this same write transaction.
        if (next.id !== id || !isAllowedOutboxTransition(current, next)) {
          if (conflictThrows) { conflict(); return }
          throw invalid('Replacement is not an authorized core outbox transition.')
        }
        const firstPacket = !('packet' in current) && 'packet' in next
        if (firstPacket && metadata === undefined) throw invalid('Prepared packet bytes must be persisted before dispatch.')
        tx.objectStore(attemptsStore).put({ ...envelope, record: next, updatedAt: now(), ...(firstPacket ? metadata : {}), ...(terminal(next) ? { terminalAt: envelope.terminalAt ?? now(), timelineObserved: next.state === 'not-applied' || (next.state === 'delivered' && next.evidence.kind === 'timeline') || envelope.timelineObserved } : {}) })
        done(next)
      } catch (cause) { fail(cause) }
    }, fail)
  })
  // Serialize recovery with dispatch and require core authorization inside this transaction too.
  const recover = (): Promise<void> => run<void>('readwrite', 'recover', (tx, done, fail) => {
    const request = tx.objectStore(attemptsStore).openCursor()
    request.onsuccess = () => {
      try {
        const cursor = request.result
        if (cursor === null) { done(undefined); return }
        const envelope = inspect(cursor.value)
        if (envelope !== undefined && sameScope(envelope.scope, scope) && envelope.record !== undefined && envelope.dismissed === undefined) {
          const next = restoreOutboxRecord(envelope.record)
          if (next !== envelope.record) {
            if (!isAllowedOutboxTransition(envelope.record, next)) throw invalid('Core rejected startup recovery.')
            cursor.update({ ...envelope, record: next, updatedAt: now() })
          }
        }
        cursor.continue()
      } catch (cause) { fail(cause) }
    }
  })
  if (options.recoverOnOpen !== false) yield* Effect.tryPromise({
    try: () => recover().catch((cause: unknown) => { db.close(); throw cause }),
    catch: (cause) => error(cause, 'recover'),
  })
  const readBytes = (id: string, field: 'packetBytes' | 'intentBytes'): Promise<Uint8Array | undefined> => run('readonly', field, (tx, done, fail) => get(tx, id, (envelope) => done(envelope?.[field] === undefined ? undefined : new Uint8Array(envelope[field])), fail))
  const repo: IndexedDbOutbox = {
    scope, persistence, accept, recover, commit: (record) => accept(record), close: () => db.close(),
    compareAndSwap: async (revision, record) => {
      const owned = decodeRecord(Schema.decodeUnknownSync(json)(encodeJson(record)))
      const metadata = owned.state === 'pending' && owned.phase === 'ready' && !owned.mayHaveDispatched
        ? await packetMetadata(owned) : undefined
      return (await change(owned.id, revision, () => owned, metadata)) !== undefined
    },
    acquireDispatchPermit: async (id, revision) => {
      const record = await change(id, revision, markOutboxDispatched, undefined, true)
      if (record === undefined) throw new OutboxConflict({ localAttemptId: id, expectedRevision: revision })
      return record
    },
    read: (id) => run('readonly', 'read', (tx, done, fail) => get(tx, id, (envelope) => done(envelope?.record), fail)),
    list: () => run('readonly', 'list', (tx, done, fail) => {
      const entries: OutboxEntry<PreparedPacket>[] = []
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        try {
          const cursor = request.result
          if (cursor === null) { done(entries); return }
          const owner = rawScope(cursor.value)
          if (owner === undefined || sameScope(owner, scope)) {
            const envelope = inspect(cursor.value)
            if (envelope === undefined) {
              const raw: unknown = cursor.value
              const revision = typeof raw === 'object' && raw !== null && 'record' in raw
                && typeof raw.record === 'object' && raw.record !== null && 'revision' in raw.record
                && typeof raw.record.revision === 'number' && Number.isSafeInteger(raw.record.revision) && raw.record.revision >= 0
                ? raw.record.revision : undefined
              entries.push({ _tag: 'corrupt', id: String(cursor.primaryKey), ...(revision === undefined ? {} : { revision }) })
            } else if (envelope.record !== undefined && envelope.dismissed === undefined) {
              entries.push({ _tag: 'record', record: envelope.record })
            }
          }
          cursor.continue()
        } catch (cause) { fail(cause) }
      }
    }),
    readIntentBytes: (id) => readBytes(id, 'intentBytes'), readPacketBytes: (id) => readBytes(id, 'packetBytes'),
    readSpool: (id, hash) => run('readonly', 'readSpool', (tx, done, fail) => get(tx, id, (envelope) => {
      if (!envelope?.record?.intent.attachments.some((attachment) => attachment.contentHash === hash)) { done(undefined); return }
      const request = tx.objectStore(spoolStore).get([id, hash])
      request.onsuccess = () => {
        const raw: unknown = request.result
        if (raw === undefined) done(undefined)
        else if (typeof raw === 'object' && raw !== null && 'bytes' in raw && raw.bytes instanceof Uint8Array) done(raw.bytes)
        else fail(quarantined())
      }
    }, fail)),
    listQuarantined: () => run('readonly', 'listQuarantined', (tx, done) => {
      const rows: QuarantinedAttempt[] = []
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        const cursor = request.result
        if (cursor === null) { done(rows); return }
        const owner = rawScope(cursor.value)
        if (inspect(cursor.value) === undefined && (owner === undefined || sameScope(owner, scope))) rows.push({ key: cursor.primaryKey, reason: 'corrupt-or-unsupported', ...(owner === undefined ? {} : { raw: cursor.value }) })
        cursor.continue()
      }
    }),
    dismiss: (id) => run('readwrite', 'dismiss', (tx, done, fail) => {
      const store = tx.objectStore(attemptsStore)
      const request = store.get(id)
      request.onsuccess = () => {
        try {
          const raw: unknown = request.result
          const owner = rawScope(raw)
          if (raw === undefined || owner !== undefined && !sameScope(owner, scope)) { done(undefined); return }
          const envelope = inspect(raw)
          if (envelope === undefined) {
            // Explicit discard alone may remove damaged content; never claim nonapplication.
            store.delete(id)
            const spool = tx.objectStore(spoolStore).openCursor(IDBKeyRange.bound([id], [id, []]))
            spool.onsuccess = () => { const item = spool.result; if (item !== null) { item.delete(); item.continue() } }
          } else store.put({ ...envelope, dismissed: { at: now() }, updatedAt: now() })
          done(undefined)
        } catch (cause) { fail(cause) }
      }
    }),
    cleanupTerminal: () => run('readwrite', 'cleanupTerminal', (tx, done, fail) => {
      const timestamp = now()
      const request = tx.objectStore(attemptsStore).openCursor()
      request.onsuccess = () => {
        try {
          const cursor = request.result
          if (cursor === null) { done(undefined); return }
          const envelope = inspect(cursor.value)
          if (envelope !== undefined && sameScope(envelope.scope, scope)) {
            const action = retentionAction(envelope, timestamp)
            if (action === 'delete') cursor.delete()
            if (action === 'strip-content') {
              const { record: _record, intentBytes: _intent, packetBytes: _packet, fingerprint: _fingerprint, ...identity } = envelope
              const record = envelope.record
              if (record === undefined) throw invalid('Only live content can become a tombstone.')
              cursor.update({ ...identity, updatedAt: timestamp, tombstoneAt: timestamp, tombstoneHistory: { state: record.state, revision: record.revision, mayHaveDispatched: record.mayHaveDispatched, ambiguous: record.ambiguous } })
              const spool = tx.objectStore(spoolStore).openCursor(IDBKeyRange.bound([envelope.localAttemptId], [envelope.localAttemptId, []]))
              spool.onsuccess = () => { const item = spool.result; if (item !== null) { item.delete(); item.continue() } }
            }
          }
          cursor.continue()
        } catch (cause) { fail(cause) }
      }
    }),
  }
  return repo
})
