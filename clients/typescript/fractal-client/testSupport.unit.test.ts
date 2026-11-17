import { describe, expect, it } from 'vitest';
import { createOutboxRecord, markOutboxDispatched, observeOutbox, prepareOutboxRecord } from './outbox.ts';
import { createMemoryOutboxRepository } from './testSupport.ts';

const scope = { principalId: 'person/original', deploymentId: 'deployment/original', memberId: 'member/original' };
const intent = () => ({ text: 'Original text', attachments: [] });
const packet = { id: 'action/original', idempotency_key: 'key/original', content: 'Original text' };
const prepared = () => prepareOutboxRecord(createOutboxRecord('attempt/original', intent()), packet, ['message/original'])!;

describe('reference memory outbox repository', () => {
  it('commits acceptance once and preserves a fixed original scope', async () => {
    const originalScope = { ...scope };
    const repository = createMemoryOutboxRepository<typeof packet>(originalScope);
    originalScope.memberId = 'member/other';
    const accepted = createOutboxRecord('attempt/original', intent());
    expect(await repository.commit(accepted)).toBe(true);
    expect(await repository.commit(accepted)).toBe(false);
    expect(repository.scope).toEqual(scope);
    expect(Object.isFrozen(repository.scope)).toBe(true);
    expect(await repository.read('absent')).toBeUndefined();
  });

  it('allows only one successful preparation and dispatch CAS owner', async () => {
    const repository = createMemoryOutboxRepository<typeof packet>(scope);
    await repository.commit(createOutboxRecord('attempt/original', intent()));
    const ready = prepared();
    expect(await Promise.all([repository.compareAndSwap(0, ready), repository.compareAndSwap(0, ready)]))
      .toEqual([true, false]);
    const dispatched = markOutboxDispatched(ready)!;
    expect(await repository.compareAndSwap(1, dispatched)).toBe(true);
    expect(await repository.compareAndSwap(1, dispatched)).toBe(false);
    expect(await repository.list()).toEqual([{ _tag: 'record', record: dispatched }]);
  });

  it('rejects caller-crafted certainty, packet and intent changes inside CAS', async () => {
    const repository = createMemoryOutboxRepository<typeof packet>(scope);
    await repository.commit(createOutboxRecord('attempt/original', intent()));
    const ready = prepared();
    await repository.compareAndSwap(0, ready);
    const dispatched = markOutboxDispatched(ready)!;
    await repository.compareAndSwap(1, dispatched);
    const unknown = observeOutbox(dispatched, { kind: 'failure', applied: 'unknown' });
    expect(await repository.compareAndSwap(2, unknown)).toBe(true);
    expect(await repository.compareAndSwap(3, {
      ...ready, revision: 4, state: 'not-applied', proof: { applied: 'none' },
    })).toBe(false);
    expect(await repository.compareAndSwap(3, {
      ...unknown, revision: 4, intent: { ...intent(), text: 'Changed' },
    })).toBe(false);
    expect(await repository.compareAndSwap(3, { ...ready, revision: 4, packet: { ...packet, content: 'Changed' } })).toBe(false);
    expect(await repository.read(unknown.id)).toEqual(unknown);
    const delivered = observeOutbox(unknown, { kind: 'observed', source: 'timeline', identity: 'message/original' });
    expect(await repository.compareAndSwap(3, delivered)).toBe(true);
    expect(await repository.read(unknown.id)).toEqual(delivered);
  });

  it('isolates snapshots and independent repository instances', async () => {
    const repository = createMemoryOutboxRepository<typeof packet>(scope);
    const other = createMemoryOutboxRepository<typeof packet>({ ...scope, memberId: 'member/other' });
    const mutableIntent = intent();
    await repository.commit(createOutboxRecord('attempt/original', mutableIntent));
    mutableIntent.text = 'Changed after commit';
    const snapshot = await repository.read('attempt/original');
    expect(snapshot?.intent.text).toBe('Original text');
    expect(snapshot).not.toBe(await repository.read('attempt/original'));
    const entries = await repository.list();
    expect(entries[0]?._tag).toBe('record');
    if (entries[0]?._tag === 'record') expect(entries[0].record).not.toBe(snapshot);
    expect(await other.list()).toEqual([]);
    expect(await other.compareAndSwap(0, prepared())).toBe(false);
  });
});
