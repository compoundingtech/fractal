import { describe, expect, expectTypeOf, it } from 'vitest';
import { ClientError } from './Client.generated.ts';
import { appliedAnswer } from './errors.ts';
import {
  createOutboxRecord, prepareOutboxRecord, markOutboxDispatched, observeOutbox, restoreOutboxRecord, isAllowedOutboxTransition,
  type OutboxIntent, type OutboxObservation, type OutboxPreparedRecord, type OutboxReceipt, type OutboxRecord,
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
const expectedIdentities = ['message/original', 'message/alternate'] as const;
const intent: OutboxIntent = {
  text: packet.content,
  attachments: [
    { contentHash: 'sha256/original', name: 'image.png', mediaType: 'image/png', reference: 'blob/original' },
    { contentHash: 'sha256/other', mediaType: 'text/plain', reference: 'blob/other' },
  ],
};
const preparing = () => createOutboxRecord('attempt/original', intent);
const pending = () => prepareOutboxRecord(preparing(), packet, expectedIdentities)!;
const dispatched = (): OutboxPreparedRecord<typeof packet> => {
  const record = markOutboxDispatched(pending());
  expect(record).toBeDefined();
  return record!;
};
const failure = (applied: 'none' | 'unknown' | undefined): OutboxObservation => ({ kind: 'failure', applied });
const delivered: OutboxObservation = { kind: 'delivered', receipt };
const observed: OutboxObservation = { kind: 'observed', identity: expectedIdentities[0], source: 'timeline' };
const refusal = (details: Record<string, unknown>, applied?: 'none' | 'unknown') => new ClientError({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/original',
  code: 'stale-fence', message: 'Refused', retryable: false, details,
  ...(applied === undefined ? {} : { applied }),
}, 409);

// Round-trip through the same serialization boundary as a repository adapter, without storage I/O.
const reopen = <TRecord extends OutboxRecord<typeof packet>>(record: TRecord): TRecord => JSON.parse(JSON.stringify(record));

describe('outbox dispatch barrier', () => {
  it('accepts durable intent before uploads or packet preparation', () => {
    const original = preparing();
    expect(original).toEqual({
      id: 'attempt/original', revision: 0, intent, state: 'pending', phase: 'preparing',
      mayHaveDispatched: false, ambiguous: false,
    });
    expect('packet' in original).toBe(false);
    expect('expectedIdentities' in original).toBe(false);
    expect(markOutboxDispatched(original)).toBeUndefined();
    for (const observation of [observed, delivered, failure('none'), failure('unknown')]) {
      expect(observeOutbox(original, observation)).toBe(original);
    }
    const persisted = reopen(original);
    expect(persisted.intent).toEqual(intent);
    expect(restoreOutboxRecord(persisted)).toBe(persisted);
    expect(markOutboxDispatched(persisted)).toBeUndefined();
    const prepared = prepareOutboxRecord(persisted, packet, expectedIdentities)!;
    expect(prepared.intent).toBe(persisted.intent);
    expect(prepared.packet).toBe(packet);
    expect(prepared.expectedIdentities).toBe(expectedIdentities);
    expect(prepared.revision).toBe(1);
    expect(prepared.phase).toBe('ready');
    expect(original.phase).toBe('preparing');
  });

  it('never replaces an already prepared packet or a terminal packet', () => {
    const replacement = { ...packet, content: 'Replacement must not be used' };
    for (const original of [
      pending(), dispatched(), restoreOutboxRecord(dispatched()),
      observeOutbox(dispatched(), delivered), observeOutbox(dispatched(), failure('none')),
    ]) {
      expect(prepareOutboxRecord(original, replacement, ['message/replacement'])).toBeUndefined();
      expect('packet' in original && original.packet).toEqual(packet);
      expect(original.intent).toEqual(intent);
    }
    expect(prepareOutboxRecord({ ...preparing(), ambiguous: true }, packet)).toBeUndefined();
    expect(prepareOutboxRecord({ ...preparing(), mayHaveDispatched: true }, packet)).toBeUndefined();
  });

  it('creates never-dispatched pending work and consumes its permission only once', () => {
    const original = pending();
    expect(original).toEqual({
      id: 'attempt/original', revision: 1, intent, packet, expectedIdentities, state: 'pending', phase: 'ready',
      mayHaveDispatched: false, ambiguous: false,
    });
    const marked = markOutboxDispatched(original)!;
    expect(marked).toEqual({ ...original, revision: 2, mayHaveDispatched: true });
    expect(marked.packet).toBe(packet);
    expect(original.mayHaveDispatched).toBe(false);
    expect(markOutboxDispatched(marked)).toBeUndefined();
  });

  it('defaults packet-derived identities to an empty set when preparing', () => {
    const original = prepareOutboxRecord(preparing(), packet)!;
    expect(original.expectedIdentities).toEqual([]);
    expect(observeOutbox(original, observed)).toBe(original);
    const unknown = restoreOutboxRecord(markOutboxDispatched(original)!);
    expect(observeOutbox(unknown, observed)).toBe(unknown);
  });

  it('ignores receipt and failure evidence before dispatch', () => {
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
      id: original.id, revision: 3, intent, packet, expectedIdentities, state: 'delivered',
      evidence: { kind: 'receipt', receipt },
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
      id: 'attempt/original', revision: 3, intent, packet, expectedIdentities, state: 'not-applied',
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
      id: 'attempt/original', revision: 3, intent, packet, expectedIdentities, state: 'unknown',
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
    expect('packet' in completed && completed.packet).toEqual(packet);
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

describe('outbox timeline presence', () => {
  it.each(expectedIdentities)('settles never-dispatched pending work by exact identity %s', identity => {
    const original = pending();
    const result = observeOutbox(original, { kind: 'observed', identity, source: 'timeline' });
    expect(result).toEqual({
      id: original.id, revision: 2, intent, packet, expectedIdentities, state: 'delivered',
      mayHaveDispatched: false, ambiguous: false,
      evidence: { kind: 'timeline', identity, source: 'timeline' },
    });
    expect(result.mayHaveDispatched).toBe(false);
    expect(result.ambiguous).toBe(false);
    expect(markOutboxDispatched(result)).toBeUndefined();
    expect(observeOutbox(result, delivered)).toBe(result);
    expect(observeOutbox(result, failure('none'))).toBe(result);
  });

  it('settles unknown while preserving durable ambiguity and dispatch history', () => {
    const original = restoreOutboxRecord(reopen(dispatched()));
    const result = observeOutbox(original, observed);
    expect(result).toEqual({
      ...original, revision: original.revision + 1, state: 'delivered',
      evidence: { kind: 'timeline', identity: expectedIdentities[0], source: 'timeline' },
    });
    expect(result.ambiguous).toBe(true);
    expect(result.mayHaveDispatched).toBe(true);
    expect('expectedIdentities' in result && result.expectedIdentities).toEqual(expectedIdentities);
    expect(markOutboxDispatched(result)).toBeUndefined();
    expect(observeOutbox(result, failure('none'))).toBe(result);
  });

  it('preserves ambiguity without manufacturing a dispatch marker', () => {
    const original = restoreOutboxRecord({ ...pending(), ambiguous: true });
    const result = observeOutbox(original, observed);
    expect(result.state).toBe('delivered');
    expect(result.ambiguous).toBe(true);
    expect(result.mayHaveDispatched).toBe(false);
    expect(markOutboxDispatched(result)).toBeUndefined();
  });

  it.each(['message/unrelated', 'original', 'message/original/other', ''])(
    'leaves pending and unknown unchanged for mismatched identity %s', identity => {
      for (const original of [pending(), restoreOutboxRecord(dispatched())]) {
        const result = observeOutbox(original, { kind: 'observed', identity, source: 'timeline' });
        expect(result).toBe(original);
        expect('expectedIdentities' in result && result.expectedIdentities).toEqual(expectedIdentities);
        if (original.state === 'unknown') {
          expect(result.ambiguous).toBe(true);
          expect(markOutboxDispatched(result)).toBeUndefined();
        }
      }
    },
  );

  it('has no absence observation in its public type contract', () => {
    expectTypeOf<OutboxObservation['kind']>().toEqualTypeOf<'delivered' | 'observed' | 'failure'>();
    expectTypeOf<Extract<OutboxObservation, { kind: 'absent' }>>().toEqualTypeOf<never>();
    expectTypeOf<Extract<OutboxObservation, { kind: 'observed' }>>().toEqualTypeOf<{
      readonly kind: 'observed'; readonly identity: string; readonly source: 'timeline';
    }>();
  });

  it.each([
    { kind: 'absent', identity: expectedIdentities[0], source: 'timeline' },
    { kind: 'observed', source: 'timeline' },
    { kind: 'observed', identity: expectedIdentities[0], source: 'cache' },
    { kind: 'observed', identity: null, source: 'timeline' },
  ])('never turns absence or invalid presence into delivery ($kind, $source)', raw => {
    // Exercise the untyped serialization boundary without granting it a valid observation type.
    const observation = JSON.parse(JSON.stringify(raw));
    for (const original of [pending(), restoreOutboxRecord(dispatched())]) {
      expect(observeOutbox(original, observation)).toBe(original);
    }
  });

  it('does not replace terminal nonapplication with timeline evidence', () => {
    const original = observeOutbox(dispatched(), failure('none'));
    expect(observeOutbox(original, observed)).toBe(original);
  });
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
    expect(restored).toEqual({
      id: marked.id, revision: 3, intent, packet, expectedIdentities,
      state: 'unknown', mayHaveDispatched: true, ambiguous: true,
    });
    expect(markOutboxDispatched(restored)).toBeUndefined();
    expect(observeOutbox(restored, failure('none'))).toBe(restored);
    expect(restoreOutboxRecord(restored)).toBe(restored);
    expect(marked.state).toBe('pending');
  });

  it.each([delivered, observed, failure('none'), failure('unknown')])('retains persisted terminal evidence or ambiguity ($kind)', observation => {
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

  it.each([delivered, observed])('round-trips discriminated delivery evidence ($kind)', observation => {
    for (const original of [dispatched(), restoreOutboxRecord(dispatched())]) {
      const completed = observeOutbox(original, observation);
      const persisted = reopen(completed);
      expect(persisted).toEqual(completed);
      expect('expectedIdentities' in persisted && persisted.expectedIdentities).toEqual(expectedIdentities);
      expect(persisted.intent).toEqual(intent);
      expect(persisted.ambiguous).toBe(original.ambiguous);
      expect(persisted.mayHaveDispatched).toBe(original.mayHaveDispatched);
      expect(restoreOutboxRecord(persisted)).toBe(persisted);
      expect(markOutboxDispatched(persisted)).toBeUndefined();
      if (persisted.state === 'delivered') {
        expect(persisted.evidence).toEqual(observation.kind === 'delivered'
          ? { kind: 'receipt', receipt }
          : { kind: 'timeline', identity: expectedIdentities[0], source: 'timeline' });
      } else {
        expect.unreachable('positive delivery evidence must settle the record');
      }
    }
  });

  it('retains timeline evidence without inventing dispatch or ambiguity on restart', () => {
    const completed = reopen(observeOutbox(pending(), observed));
    expect(restoreOutboxRecord(completed)).toBe(completed);
    expect(completed.mayHaveDispatched).toBe(false);
    expect(completed.ambiguous).toBe(false);
    expect('expectedIdentities' in completed && completed.expectedIdentities).toEqual(expectedIdentities);
    expect(markOutboxDispatched(completed)).toBeUndefined();
  });
});

describe('outbox CAS transition authorization', () => {
  it('accepts exactly helper-produced durable proposals across the lifecycle', () => {
    const accepted = preparing();
    const ready = prepareOutboxRecord(accepted, packet, expectedIdentities)!;
    const marked = markOutboxDispatched(ready)!;
    const unknown = restoreOutboxRecord(marked);
    const refused = observeOutbox(marked, failure('none'));
    for (const [previous, next] of [
      [accepted, ready],
      [ready, marked],
      [marked, unknown],
      [marked, refused],
      [marked, observeOutbox(marked, failure(undefined))],
      [marked, observeOutbox(marked, delivered)],
      [ready, observeOutbox(ready, observed)],
      [unknown, observeOutbox(unknown, observed)],
      [unknown, observeOutbox(unknown, delivered)],
      [refused, observeOutbox(refused, delivered)],
      [{ ...ready, ambiguous: true }, restoreOutboxRecord({ ...ready, ambiguous: true })],
    ] as const) {
      expect(isAllowedOutboxTransition(previous, reopen(next))).toBe(true);
    }
  });

  it('compares immutable JSON content independently of object insertion order', () => {
    const previous = preparing();
    const ready = pending();
    const reordered = {
      ambiguous: ready.ambiguous, mayHaveDispatched: ready.mayHaveDispatched,
      phase: ready.phase, state: ready.state, expectedIdentities: [...ready.expectedIdentities],
      packet: {
        signature: { ...packet.signature }, content: packet.content, recipient: packet.recipient,
        fence: { ...packet.fence }, type: packet.type, idempotency_key: packet.idempotency_key,
        id: packet.id, api_version: packet.api_version,
      },
      intent: {
        attachments: intent.attachments.map(attachment => ({
          reference: attachment.reference, mediaType: attachment.mediaType,
          ...(attachment.name === undefined ? {} : { name: attachment.name }),
          contentHash: attachment.contentHash,
        })),
        text: intent.text,
      },
      revision: ready.revision, id: ready.id,
    };
    expect(isAllowedOutboxTransition(previous, reordered)).toBe(true);
    const marked = markOutboxDispatched(reordered)!;
    expect(isAllowedOutboxTransition(ready, marked)).toBe(true);
  });

  it('rejects caller-crafted bypasses, history clearing and immutable-content changes', () => {
    const ready = pending();
    const marked = dispatched();
    const unknown = restoreOutboxRecord(marked);
    const completed = observeOutbox(unknown, observed);
    const refusal = observeOutbox(marked, failure('none'));
    for (const [previous, next] of [
      [ready, ready],
      [ready, { ...ready, revision: ready.revision + 1 }],
      [ready, { ...marked, revision: ready.revision + 2 }],
      [ready, { ...marked, id: 'attempt/other' }],
      [ready, { ...marked, intent: { ...intent, text: 'Changed text' } }],
      [ready, { ...marked, intent: { ...intent, attachments: [] } }],
      [ready, { ...marked, intent: { ...intent, attachments: [
        { ...intent.attachments[0]!, reference: 'blob/replaced' },
      ] } }],
      [ready, { ...marked, packet: { ...packet, content: 'Changed packet' } }],
      [ready, { ...marked, expectedIdentities: ['message/replaced'] }],
      [ready, { ...marked, extra: 'unauthorized metadata' }],
      [ready, { ...marked, evidence: { kind: 'timeline', identity: expectedIdentities[0], source: 'timeline' } }],
      [ready, { ...ready, revision: ready.revision + 1, phase: 'preparing' }],
      [preparing(), { ...preparing(), revision: 1, mayHaveDispatched: true }],
      [preparing(), { ...ready, evidence: { kind: 'timeline', identity: expectedIdentities[0], source: 'timeline' } }],
      [unknown, { ...refusal, revision: unknown.revision + 1 }],
      [unknown, { ...unknown, revision: unknown.revision + 1, ambiguous: false }],
      [unknown, { ...completed, ambiguous: false }],
      [unknown, { ...completed, mayHaveDispatched: false }],
      [unknown, { ...completed, evidence: { kind: 'timeline', identity: 'message/unrelated', source: 'timeline' } }],
      [unknown, { ...completed, evidence: { kind: 'timeline', identity: expectedIdentities[0], source: 'cache' } }],
      [completed, { ...completed, revision: completed.revision + 1 }],
      [completed, { ...ready, revision: completed.revision + 1 }],
      [refusal, { ...completed, revision: refusal.revision + 1 }],
    ] as const) {
      // JSON boundary permits malformed proposals that cannot be constructed through the typed API.
      expect(isAllowedOutboxTransition(previous, JSON.parse(JSON.stringify(next)))).toBe(false);
    }
  });
});
