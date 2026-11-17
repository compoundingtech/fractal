import { describe, expect, it } from 'vitest';
import { ClientError } from './Client.generated.ts';
import { appliedAnswer } from './errors.ts';
import {
  createOutboxRecord, markOutboxDispatched, observeOutbox, restoreOutboxRecord,
  type OutboxObservation, type OutboxReceipt, type OutboxRecord,
} from './outbox.ts';

const packet = {
  api_version: 'st3.client.v0', id: 'action/original', idempotency_key: 'original',
  type: 'message.send', fence: { snapshot_id: 'snapshot/original' },
  recipient: 'actor/recipient', content: 'Original message',
  signature: { nonce: 'nonce/original', signed_at: '2026-11-17T00:00:00Z', signature: 'signature/original' },
};
const receipt: OutboxReceipt = {
  kind: 'action-result', action_id: packet.id, operation_id: 'operation/original',
  snapshot_id: 'snapshot/completed', status: 'completed', affected_ids: ['message/original'],
};
const pending = () => createOutboxRecord('attempt/original', packet);
const dispatched = (): OutboxRecord<typeof packet> => {
  const record = markOutboxDispatched(pending());
  expect(record).toBeDefined();
  return record!;
};
const failure = (applied: 'none' | 'unknown' | undefined): OutboxObservation => ({ kind: 'failure', applied });
const delivered: OutboxObservation = { kind: 'delivered', receipt };
const refusal = (details: Record<string, unknown>, applied?: 'none' | 'unknown') => new ClientError({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/original',
  code: 'stale-fence', message: 'Refused', retryable: false, details,
  ...(applied === undefined ? {} : { applied }),
}, 409);

// Round-trip through the same serialization boundary as a repository adapter, without storage I/O.
const reopen = (record: OutboxRecord<typeof packet>): OutboxRecord<typeof packet> => JSON.parse(JSON.stringify(record));

describe('outbox dispatch barrier', () => {
  it('creates never-dispatched pending work and consumes its permission only once', () => {
    const original = pending();
    expect(original).toEqual({
      id: 'attempt/original', revision: 0, packet, state: 'pending',
      mayHaveDispatched: false, ambiguous: false,
    });
    const marked = markOutboxDispatched(original)!;
    expect(marked).toEqual({ ...original, revision: 1, mayHaveDispatched: true });
    expect(marked.packet).toBe(packet);
    expect(original.mayHaveDispatched).toBe(false);
    expect(markOutboxDispatched(marked)).toBeUndefined();
  });

  it('ignores outcome evidence before dispatch', () => {
    const original = pending();
    for (const observation of [delivered, failure('none'), failure('unknown'), failure(undefined)]) {
      expect(observeOutbox(original, observation)).toBe(original);
    }
  });
});

describe('outbox outcomes', () => {
  it('records a validated completed receipt as delivered', () => {
    const original = dispatched();
    const result = observeOutbox(original, delivered);
    expect(result).toEqual({
      id: original.id, revision: 2, packet, state: 'delivered', receipt,
      mayHaveDispatched: true, ambiguous: false,
    });
    expect(original.state).toBe('pending');
    expect(markOutboxDispatched(result)).toBeUndefined();
    expect(observeOutbox(result, failure('unknown'))).toBe(result);
    expect(observeOutbox(result, failure('none'))).toBe(result);
  });

  it('records explicit none without granting the original packet another dispatch', () => {
    const result = observeOutbox(dispatched(), failure('none'));
    expect(result).toEqual({
      id: 'attempt/original', revision: 2, packet, state: 'not-applied',
      mayHaveDispatched: true, ambiguous: false, proof: { applied: 'none' },
    });
    expect(markOutboxDispatched(result)).toBeUndefined();
    expect(observeOutbox(result, failure('none'))).toBe(result);
    expect(observeOutbox(result, failure(undefined))).toBe(result);
    // Definitive completion wins over an earlier refusal, without replaying anything.
    expect(observeOutbox(result, delivered).state).toBe('delivered');
  });

  it.each(['unknown', undefined] as const)('records %s certainty as durable ambiguity', applied => {
    const result = observeOutbox(dispatched(), failure(applied));
    expect(result).toEqual({
      id: 'attempt/original', revision: 2, packet, state: 'unknown',
      mayHaveDispatched: true, ambiguous: true,
    });
    expect(markOutboxDispatched(result)).toBeUndefined();
    expect(observeOutbox(result, failure('none'))).toBe(result);
    expect(observeOutbox(result, failure('unknown'))).toBe(result);
    expect(observeOutbox(result, failure(undefined))).toBe(result);
  });

  it('preserves earlier ambiguity after serialization and a later explicit refusal', () => {
    const unknown = reopen(observeOutbox(dispatched(), failure(undefined)));
    const restored = restoreOutboxRecord(unknown);
    const later = observeOutbox(restored, failure(appliedAnswer(refusal({ applied: 'none' }))));
    expect(later).toBe(restored);
    expect(later.state).toBe('unknown');
    expect(later.ambiguous).toBe(true);
    expect(markOutboxDispatched(later)).toBeUndefined();
    const completed = observeOutbox(later, delivered);
    expect(completed.state).toBe('delivered');
    expect(completed.ambiguous).toBe(true);
    expect(completed.mayHaveDispatched).toBe(true);
    expect(completed.revision).toBe(later.revision + 1);
    expect(completed.packet).toEqual(packet);
    expect(markOutboxDispatched(completed)).toBeUndefined();
  });

  it.each([
    refusal({}),
    refusal({ applied: 'unknown' }),
    refusal({ applied: null }),
    refusal({ applied: 'none' }, 'unknown'),
    refusal({ applied: 'unknown' }, 'none'),
    refusal({ applied: 'invalid' }, 'none'),
  ])('does not mistake absent or contradictory applied flags for proof ($response.details)', error => {
    const result = observeOutbox(dispatched(), failure(appliedAnswer(error)));
    expect(result.state).toBe('unknown');
    expect(result.ambiguous).toBe(true);
    expect(markOutboxDispatched(result)).toBeUndefined();
  });

  it.each([refusal({ applied: 'none' }), refusal({}, 'none'), refusal({ applied: 'none' }, 'none')])(
    'accepts explicit uncontradicted none only on the first dispatch response', error => {
      expect(observeOutbox(dispatched(), failure(appliedAnswer(error))).state).toBe('not-applied');
    },
  );
});

describe('outbox restoration', () => {
  it('keeps a never-dispatched packet pending with its exact signed fields', () => {
    const record = reopen(pending());
    expect(restoreOutboxRecord(record)).toBe(record);
    expect(markOutboxDispatched(record)?.packet).toEqual(packet);
  });

  it('restores dispatch-before-IO and dispatch-with-lost-receipt identically as unknown', () => {
    const marked = reopen(dispatched());
    const restored = restoreOutboxRecord(marked);
    expect(restored).toEqual({ ...marked, revision: 2, state: 'unknown', ambiguous: true });
    expect(markOutboxDispatched(restored)).toBeUndefined();
    expect(observeOutbox(restored, failure('none'))).toBe(restored);
    expect(restoreOutboxRecord(restored)).toBe(restored);
    expect(marked.state).toBe('pending');
  });

  it.each([delivered, failure('none'), failure('unknown')])('retains persisted terminal evidence or ambiguity ($kind)', observation => {
    const record = reopen(observeOutbox(dispatched(), observation));
    expect(restoreOutboxRecord(record)).toBe(record);
    expect(markOutboxDispatched(record)).toBeUndefined();
  });

  it('never grants dispatch or refusal certainty to contradictory durable history', () => {
    for (const record of [
      { ...pending(), ambiguous: true },
      { ...dispatched(), ambiguous: true },
      { ...observeOutbox(dispatched(), failure('none')), ambiguous: true },
    ]) {
      const restored = restoreOutboxRecord(record);
      expect(restored.state).toBe('unknown');
      expect(restored.ambiguous).toBe(true);
      expect(markOutboxDispatched(restored)).toBeUndefined();
      expect(observeOutbox(restored, failure('none')).state).toBe('unknown');
    }
  });

  it('retains definitive completion and its ambiguity history across restart', () => {
    const unknown = restoreOutboxRecord(dispatched());
    const completed = reopen(observeOutbox(unknown, delivered));
    expect(restoreOutboxRecord(completed)).toBe(completed);
    expect(completed.ambiguous).toBe(true);
  });
});
