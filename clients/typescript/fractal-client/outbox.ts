import type { ActionResult } from './Models.generated.ts';

/** A completed receipt already validated against the original packet by actionReceiptCheck. */
export type OutboxReceipt = ActionResult & { status: 'completed' };

export type OutboxDeliveryEvidence =
  | { readonly kind: 'receipt'; readonly receipt: OutboxReceipt }
  | { readonly kind: 'timeline'; readonly identity: string; readonly source: 'timeline' };

/** Serializable local intent retained before uploads or packet preparation begin. */
export type OutboxIntent = {
  readonly text: string;
  readonly attachments: readonly {
    readonly contentHash: string;
    readonly name?: string;
    readonly mediaType: string;
    /** Durable storage reference, not an in-memory file or expiring upload URL. */
    readonly reference: string;
  }[];
};

/** Durable history shared by intent preparation and all prepared-packet states. */
type OutboxHistory = {
  readonly id: string;
  readonly revision: number;
  readonly intent: OutboxIntent;
  /** Monotone: commit this marker before making the single action POST. */
  readonly mayHaveDispatched: boolean;
  /** Monotone history, retained even after delivery evidence arrives. */
  readonly ambiguous: boolean;
};

export type OutboxPreparingRecord = OutboxHistory & {
  readonly state: 'pending';
  readonly phase: 'preparing';
};

/**
 * TPacket is the complete serializable action POST body, including its original signature
 * when present, not a transport, closure, or just the common action fields.
 */
export type OutboxPreparedRecord<TPacket> = OutboxHistory & {
  readonly packet: TPacket;
  /** Packet-derived identities persisted before dispatch; timeline absence is never evidence. */
  readonly expectedIdentities: readonly string[];
} & (
  | { readonly state: 'pending'; readonly phase: 'ready' }
  | { readonly state: 'unknown' }
  | { readonly state: 'delivered'; readonly evidence: OutboxDeliveryEvidence }
  | { readonly state: 'not-applied'; readonly proof: { readonly applied: 'none' } }
);

export type OutboxRecord<TPacket> = OutboxPreparingRecord | OutboxPreparedRecord<TPacket>;

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
 * commit inserts pristine pending/preparing revision 0 only if id is absent.
 * compareAndSwap atomically replaces the same id only if its current revision equals
 * expectedRevision and the replacement revision is expectedRevision + 1.
 * False means a conflict, never permission to POST.
 * Adapters must require isAllowedOutboxTransition(current, replacement) inside the same
 * transaction; revision equality alone does not authorize a replacement.
 * Adapters must retain the exact intent, prepared packet and expected identities. Every operation
 * is restricted to the fixed scope below; changing the active client must not rebind it.
 */
export interface OutboxRepository<TPacket> {
  readonly scope: OutboxScope;
  commit(record: OutboxPreparingRecord): Promise<boolean>;
  compareAndSwap(expectedRevision: number, record: OutboxRecord<TPacket>): Promise<boolean>;
  list(): Promise<readonly OutboxRecord<TPacket>[]>;
  read(id: string): Promise<OutboxRecord<TPacket> | undefined>;
}

/** Receipts are validated by the caller; observed identities come from the original member's authoritative timeline. */
export type OutboxObservation =
  | { readonly kind: 'delivered'; readonly receipt: OutboxReceipt }
  | { readonly kind: 'observed'; readonly identity: string; readonly source: 'timeline' }
  // Use appliedAnswer(error): missing, malformed and contradictory flags are not none.
  | { readonly kind: 'failure'; readonly applied: 'none' | 'unknown' | undefined };

/** Commit durable intent before acknowledging local acceptance or starting uploads. No I/O occurs here. */
export const createOutboxRecord = (id: string, intent: OutboxIntent): OutboxPreparingRecord => ({
  id, revision: 0, intent, state: 'pending', phase: 'preparing', mayHaveDispatched: false, ambiguous: false,
});

/** CAS this proposal after preparation; a prepared packet can never be replaced. */
export const prepareOutboxRecord = <TPacket>(
  record: OutboxRecord<TPacket>,
  packet: TPacket,
  expectedIdentities: readonly string[] = [],
): (OutboxPreparedRecord<TPacket> & { readonly state: 'pending'; readonly phase: 'ready' }) | undefined => {
  if (record.state !== 'pending' || record.phase !== 'preparing' || record.mayHaveDispatched || record.ambiguous) return undefined;
  return {
    id: record.id, revision: record.revision + 1, intent: record.intent,
    state: 'pending', phase: 'ready', packet, expectedIdentities,
    mayHaveDispatched: false, ambiguous: false,
  };
};

/**
 * Propose consumption of the one-shot dispatch permission. POST the retained packet only
 * after compareAndSwap succeeds. Never retry the POST on a failed write, lost response,
 * expired lease or process restart. Undefined means this attempt cannot be dispatched.
 */
export const markOutboxDispatched = <TPacket>(record: OutboxRecord<TPacket>): OutboxPreparedRecord<TPacket> | undefined => {
  if (record.state !== 'pending' || record.phase !== 'ready' || record.mayHaveDispatched || record.ambiguous) return undefined;
  return {
    id: record.id, revision: record.revision + 1, intent: record.intent,
    state: 'pending', phase: 'ready', packet: record.packet, expectedIdentities: record.expectedIdentities,
    mayHaveDispatched: true, ambiguous: record.ambiguous,
  };
};

/**
 * Persist the proposed outcome by revision CAS. A later refusal cannot erase earlier
 * ambiguity. This core deliberately has no exclusion/seal protocol and therefore cannot
 * resolve unknown to not-applied, nor turn terminal attempts back into pending retries.
 * A validated receipt or matching timeline presence can settle unknown without granting dispatch.
 */
export const observeOutbox = <TPacket>(record: OutboxRecord<TPacket>, observation: OutboxObservation): OutboxRecord<TPacket> => {
  if (record.state === 'delivered') return record;
  if (record.state === 'pending' && record.phase === 'preparing') return record;
  if (observation.kind === 'observed') {
    if (
      (record.state !== 'pending' && record.state !== 'unknown') ||
      observation.source !== 'timeline' ||
      !record.expectedIdentities.includes(observation.identity)
    ) return record;
    return {
      id: record.id, revision: record.revision + 1, intent: record.intent, packet: record.packet,
      expectedIdentities: record.expectedIdentities,
      mayHaveDispatched: record.mayHaveDispatched, ambiguous: record.ambiguous,
      state: 'delivered',
      evidence: { kind: 'timeline', identity: observation.identity, source: observation.source },
    };
  }
  if (!record.mayHaveDispatched) return record;
  if (observation.kind === 'delivered') {
    return {
      id: record.id, revision: record.revision + 1, intent: record.intent, packet: record.packet,
      expectedIdentities: record.expectedIdentities,
      mayHaveDispatched: true, ambiguous: record.ambiguous,
      state: 'delivered', evidence: { kind: 'receipt', receipt: observation.receipt },
    };
  }
  if (observation.kind !== 'failure' || record.state === 'not-applied') return record;
  if (observation.applied === 'none' && !record.ambiguous && record.state !== 'unknown') {
    return {
      id: record.id, revision: record.revision + 1, intent: record.intent, packet: record.packet,
      expectedIdentities: record.expectedIdentities,
      mayHaveDispatched: true, ambiguous: false, state: 'not-applied', proof: { applied: 'none' },
    };
  }
  if (record.state === 'unknown' && record.ambiguous) return record;
  return {
    id: record.id, revision: record.revision + 1, intent: record.intent, packet: record.packet,
    expectedIdentities: record.expectedIdentities,
    mayHaveDispatched: true, ambiguous: true, state: 'unknown',
  };
};

/**
 * Restore before exposing records to send owners; CAS any changed record before use.
 * A crash after the dispatch marker but before the socket call must still mean unknown.
 * Terminal nonapplication is retained only without contradictory ambiguity history.
 */
export const restoreOutboxRecord = <TPacket>(record: OutboxRecord<TPacket>): OutboxRecord<TPacket> => {
  if (record.state === 'pending' && record.phase === 'preparing') return record;
  if (record.state === 'delivered') return record;
  if (record.state === 'not-applied' && !record.ambiguous) return record;
  if (!record.mayHaveDispatched && !record.ambiguous && record.state === 'pending') return record;
  if (record.state === 'unknown' && record.ambiguous) return record;
  return {
    id: record.id, revision: record.revision + 1, intent: record.intent, packet: record.packet,
    expectedIdentities: record.expectedIdentities,
    mayHaveDispatched: record.mayHaveDispatched, ambiguous: true, state: 'unknown',
  };
};

const isJsonObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

/** Compare serialized values structurally: object insertion order is not identity. */
const sameJsonValue = (left: unknown, right: unknown): boolean => {
  if (left === right) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right) &&
      left.length === right.length && left.every((value, index) => sameJsonValue(value, right[index]));
  }
  if (!isJsonObject(left) || !isJsonObject(right)) return false;
  const leftKeys = Object.keys(left).filter(key => left[key] !== undefined);
  const rightKeys = Object.keys(right).filter(key => right[key] !== undefined);
  return leftKeys.length === rightKeys.length &&
    leftKeys.every(key => Object.prototype.hasOwnProperty.call(right, key) && sameJsonValue(left[key], right[key]));
};

/**
 * Pure CAS authorization. The helpers above are the only transition rules; repositories
 * must reject caller-crafted replacements that cannot be produced by one of them.
 */
export const isAllowedOutboxTransition = <TPacket>(
  previous: OutboxRecord<TPacket>,
  next: OutboxRecord<TPacket>,
): boolean => {
  if (next.revision !== previous.revision + 1) return false;
  if (
    sameJsonValue(markOutboxDispatched(previous), next) ||
    sameJsonValue(restoreOutboxRecord(previous), next) ||
    sameJsonValue(observeOutbox(previous, { kind: 'failure', applied: 'none' }), next) ||
    sameJsonValue(observeOutbox(previous, { kind: 'failure', applied: 'unknown' }), next)
  ) return true;
  if (previous.state === 'pending' && previous.phase === 'preparing' &&
    next.state === 'pending' && next.phase === 'ready') {
    return sameJsonValue(prepareOutboxRecord(previous, next.packet, next.expectedIdentities), next);
  }
  if (next.state !== 'delivered') return false;
  const observation: OutboxObservation = next.evidence.kind === 'receipt'
    ? { kind: 'delivered', receipt: next.evidence.receipt }
    : { kind: 'observed', identity: next.evidence.identity, source: next.evidence.source };
  return sameJsonValue(observeOutbox(previous, observation), next);
};
