import type { OutboxRecord, OutboxScope, PreparedPacket } from '@fractal/client'
import { Schema } from 'effect'

export const Count = Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))
export const ScopeSchema = Schema.Struct({
  principalId: Schema.NonEmptyString,
  deploymentId: Schema.NonEmptyString,
  memberId: Schema.NonEmptyString,
})
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
/** Unknown-scope rows expose only their key and reason, never another principal's payload. */
export interface QuarantinedAttempt { readonly key: IDBValidKey; readonly reason: 'corrupt-or-unsupported'; readonly raw?: unknown }

export const sameScope = (left: OutboxScope, right: OutboxScope): boolean =>
  left.principalId === right.principalId && left.deploymentId === right.deploymentId && left.memberId === right.memberId

export const terminal = (record: OutboxRecord<PreparedPacket>): boolean => record.state === 'delivered' || record.state === 'not-applied'

/** Storage-only retention metadata, never a second delivery state machine. */
export interface RetentionMetadata {
  readonly terminalAt?: number
  readonly timelineObserved: boolean
  readonly dismissed?: { readonly at: number }
  readonly tombstoneAt?: number
}
const day = 86_400_000
/** Thirty days of minimal tombstone retention starts when content is actually removed. */
export const retentionAction = (metadata: RetentionMetadata, now: number): 'keep' | 'strip-content' | 'delete' => {
  if (metadata.tombstoneAt !== undefined) return now - metadata.tombstoneAt >= 30 * day ? 'delete' : 'keep'
  if (metadata.dismissed !== undefined) return 'strip-content'
  if (metadata.terminalAt === undefined || !metadata.timelineObserved) return 'keep'
  return now - metadata.terminalAt >= 7 * day ? 'strip-content' : 'keep'
}
