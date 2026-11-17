import type { ActionResult } from './Models.generated.ts';

/** A completed receipt already validated against the original packet by actionReceiptCheck. */
export type OutboxReceipt = ActionResult & { status: 'completed' };

/**
 * TPacket is the complete serializable action POST body, including its original signature
 * when present, not a transport, closure, or just the common action fields. Owners bind
 * repository instances to the authenticated account and original member/store.
 */
export type OutboxRecord<TPacket> = {
  readonly id: string;
  readonly revision: number;
  readonly packet: TPacket;
  /** Monotone: commit this marker before making the single action POST. */
  readonly mayHaveDispatched: boolean;
  /** Monotone history, retained even after a matching completed receipt arrives. */
  readonly ambiguous: boolean;
} & (
  | { readonly state: 'pending' }
  | { readonly state: 'unknown' }
  | { readonly state: 'delivered'; readonly receipt: OutboxReceipt }
  | { readonly state: 'not-applied'; readonly proof: { readonly applied: 'none' } }
);

/** Stable trusted identities, never a display name, reusable URL/port, or current client. */
export type OutboxScope = {
  readonly principalId: string;
  readonly deploymentId: string;
  readonly memberId: string;
};

/**
 * Storage contract only. Successful writes resolve after durable transaction completion;
 * storage errors reject (never report a successful in-memory fallback).
 *
 * commit inserts revision 0 only if id is absent. compareAndSwap atomically replaces
 * the same id only if its current revision equals expectedRevision and the replacement
 * revision is expectedRevision + 1. False means a conflict, never permission to POST.
 * Adapters must retain the exact packet. Every operation is restricted to the fixed
 * scope below, including list/read; changing the active client must not rebind it.
 */
export interface OutboxRepository<TPacket> {
  readonly scope: OutboxScope;
  commit(record: OutboxRecord<TPacket>): Promise<boolean>;
  compareAndSwap(expectedRevision: number, record: OutboxRecord<TPacket>): Promise<boolean>;
  list(): Promise<readonly OutboxRecord<TPacket>[]>;
  read(id: string): Promise<OutboxRecord<TPacket> | undefined>;
}

/** Evidence is correlated to the original packet by the caller, never a timeline absence. */
export type OutboxObservation =
  | { readonly kind: 'delivered'; readonly receipt: OutboxReceipt }
  // Use appliedAnswer(error): missing, malformed and contradictory flags are not none.
  | { readonly kind: 'failure'; readonly applied: 'none' | 'unknown' | undefined };

/** Commit the returned record before acknowledging local acceptance. No I/O occurs here. */
export const createOutboxRecord = <TPacket>(id: string, packet: TPacket): OutboxRecord<TPacket> => ({
  id, revision: 0, packet, state: 'pending', mayHaveDispatched: false, ambiguous: false,
});

/**
 * Propose consumption of the one-shot dispatch permission. POST the retained packet only
 * after compareAndSwap succeeds. Never retry the POST on a failed write, lost response,
 * expired lease or process restart. Undefined means this attempt cannot be dispatched.
 */
export const markOutboxDispatched = <TPacket>(record: OutboxRecord<TPacket>): OutboxRecord<TPacket> | undefined => {
  if (record.state !== 'pending' || record.mayHaveDispatched || record.ambiguous) return undefined;
  return { ...record, revision: record.revision + 1, mayHaveDispatched: true };
};

/**
 * Persist the proposed outcome by revision CAS. A later refusal cannot erase earlier
 * ambiguity. This core deliberately has no exclusion/seal protocol and therefore cannot
 * resolve unknown to not-applied, nor turn terminal attempts back into pending retries.
 * A validated completed receipt can settle unknown without granting dispatch permission.
 */
export const observeOutbox = <TPacket>(record: OutboxRecord<TPacket>, observation: OutboxObservation): OutboxRecord<TPacket> => {
  if (!record.mayHaveDispatched || record.state === 'delivered') return record;
  if (observation.kind === 'delivered') {
    return {
      id: record.id, revision: record.revision + 1, packet: record.packet,
      mayHaveDispatched: true, ambiguous: record.ambiguous,
      state: 'delivered', receipt: observation.receipt,
    };
  }
  if (record.state === 'not-applied') return record;
  if (observation.applied === 'none' && !record.ambiguous && record.state !== 'unknown') {
    return {
      id: record.id, revision: record.revision + 1, packet: record.packet,
      mayHaveDispatched: true, ambiguous: false, state: 'not-applied', proof: { applied: 'none' },
    };
  }
  if (record.state === 'unknown' && record.ambiguous) return record;
  return {
    id: record.id, revision: record.revision + 1, packet: record.packet,
    mayHaveDispatched: true, ambiguous: true, state: 'unknown',
  };
};

/**
 * Restore before exposing records to send owners; CAS any changed record before use.
 * A crash after the dispatch marker but before the socket call must still mean unknown.
 * Terminal nonapplication is retained only without contradictory ambiguity history.
 */
export const restoreOutboxRecord = <TPacket>(record: OutboxRecord<TPacket>): OutboxRecord<TPacket> => {
  if (record.state === 'delivered') return record;
  if (record.state === 'not-applied' && !record.ambiguous) return record;
  if (!record.mayHaveDispatched && !record.ambiguous && record.state === 'pending') return record;
  if (record.state === 'unknown' && record.ambiguous) return record;
  return {
    id: record.id, revision: record.revision + 1, packet: record.packet,
    mayHaveDispatched: record.mayHaveDispatched, ambiguous: true, state: 'unknown',
  };
};
