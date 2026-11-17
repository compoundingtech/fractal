import { Effect, Schema } from 'effect'

const Count = Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))
const Bytes = Schema.Array(Schema.Int.check(Schema.isBetween({ minimum: 0, maximum: 255 })))
export const OutboxScope = Schema.Struct({
  account: Schema.NonEmptyString,
  deployment: Schema.NonEmptyString,
  member: Schema.NonEmptyString,
  profile: Schema.NonEmptyString,
  kind: Schema.Literals(['chat', 'thread-start', 'thread-reply']),
})
export type OutboxScope = typeof OutboxScope.Type
const Attachment = Schema.Struct({ hash: Schema.NonEmptyString, name: Schema.String, mediaType: Schema.NonEmptyString })
const Intent = Schema.Struct({
  /** Exact serialized intent, including target, captured session, reply/tags and Start context. */
  bytes: Bytes,
  attachments: Schema.Array(Attachment),
})
const Packet = Schema.Struct({
  /** Complete authorized body, unsigned when authorized that way; never credentials. */
  bytes: Bytes,
  fingerprint: Schema.Struct({ version: Schema.Literal(1), sha256: Schema.NonEmptyString }),
  actionId: Schema.NonEmptyString,
  idempotencyKey: Schema.NonEmptyString,
  expectedIdentities: Schema.Array(Schema.NonEmptyString),
})
const Evidence = Schema.Struct({
  member: Schema.NonEmptyString,
  actionId: Schema.NonEmptyString,
  idempotencyKey: Schema.NonEmptyString,
  requestSha256: Schema.NonEmptyString,
  bytes: Bytes,
})
export const OutboxRecord = Schema.Struct({
  schemaVersion: Schema.Literal(1),
  localAttemptId: Schema.NonEmptyString,
  scope: OutboxScope,
  intent: Schema.optionalKey(Intent),
  preparedPacket: Schema.optionalKey(Packet),
  state: Schema.Union([
    Schema.Struct({ status: Schema.Literal('pending'), phase: Schema.Literals(['preparing', 'ready', 'dispatched']) }),
    Schema.Struct({ status: Schema.Literals(['delivered', 'not-applied', 'unknown']) }),
  ]),
  mayHaveDispatched: Schema.Boolean,
  revision: Count,
  createdAt: Count,
  updatedAt: Count,
  terminalAt: Schema.optionalKey(Count),
  timelineObserved: Schema.Boolean,
  receipt: Schema.optionalKey(Evidence),
  proof: Schema.optionalKey(Evidence),
  diagnosticCode: Schema.optionalKey(Schema.NonEmptyString),
  dismissed: Schema.optionalKey(Schema.Struct({ at: Count })),
  tombstoneAt: Schema.optionalKey(Count),
}).annotate({ identifier: 'Outbox.Record.V1' })
export type OutboxRecord = typeof OutboxRecord.Type

export class OutboxStorageError extends Schema.TaggedError<OutboxStorageError>()('OutboxStorageError', {
  code: Schema.Literals(['disabled', 'denied', 'quota', 'storage', 'invalid-record', 'quarantined']),
  operation: Schema.String,
  message: Schema.String,
}) {}
export class OutboxConflict extends Schema.TaggedError<OutboxConflict>()('OutboxConflict', {
  localAttemptId: Schema.String,
  expectedRevision: Schema.Int,
  actualRevision: Schema.optionalKey(Count),
}) {}
export type OutboxError = OutboxStorageError | OutboxConflict
export interface SpoolInput { readonly hash: string; readonly bytes: Uint8Array }
export interface QuarantinedAttempt { readonly key: IDBValidKey; readonly reason: 'corrupt-or-unsupported'; readonly raw: unknown }

/**
 * Temporary local mirror until the core repository interface lands on
 * schickling-assistant/2026-11-17-937-delivery-certainty. No transport or policy driver lives here.
 * Commit/CAS acknowledge only after transaction completion, never request success.
 */
export interface OutboxRepository {
  readonly commit: (record: OutboxRecord, spool?: readonly SpoolInput[]) => Effect.Effect<OutboxRecord, OutboxError>
  readonly compareAndSwap: (expectedRevision: number, record: OutboxRecord) => Effect.Effect<OutboxRecord, OutboxError>
  readonly read: (localAttemptId: string) => Effect.Effect<OutboxRecord | undefined, OutboxError>
  readonly list: (scope: OutboxScope) => Effect.Effect<readonly OutboxRecord[], OutboxError>
}

export const sameScope = (left: OutboxScope, right: OutboxScope): boolean =>
  left.account === right.account && left.deployment === right.deployment && left.member === right.member
  && left.profile === right.profile && left.kind === right.kind

export const terminal = (record: OutboxRecord): boolean =>
  record.state.status === 'delivered' || record.state.status === 'not-applied'

const day = 24 * 60 * 60 * 1000
/** 30 days of minimal tombstone retention starts when content is actually removed. */
export const retentionAction = (record: OutboxRecord, now: number): 'keep' | 'strip-content' | 'delete' => {
  if (record.tombstoneAt !== undefined) return now - record.tombstoneAt >= 30 * day ? 'delete' : 'keep'
  if (record.dismissed !== undefined) return 'strip-content'
  if (!terminal(record) || record.terminalAt === undefined) return 'keep'
  if (record.state.status === 'delivered' && !record.timelineObserved) return 'keep'
  return now - record.terminalAt >= 7 * day ? 'strip-content' : 'keep'
}
