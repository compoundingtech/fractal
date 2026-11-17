import { isAllowedOutboxTransition } from './outbox.ts';
import type { OutboxRecord, OutboxRepository, OutboxScope } from './outbox.ts';

/**
 * Reference repository double, never a durable production adapter. Each instance is
 * permanently scoped. Copy-on-read/write prevents callers from changing stored records
 * outside CAS. The synchronous check-and-write section models a storage transaction.
 */
export const createMemoryOutboxRepository = <TPacket>(scope: OutboxScope): OutboxRepository<TPacket> => {
  const records = new Map<string, OutboxRecord<TPacket>>();
  return {
    scope: Object.freeze(structuredClone(scope)),
    async commit(record) {
      if (records.has(record.id) || record.revision !== 0 || record.state !== 'pending' ||
        record.phase !== 'preparing' || record.mayHaveDispatched || record.ambiguous ||
        'packet' in record || 'expectedIdentities' in record) return false;
      records.set(record.id, structuredClone(record));
      return true;
    },
    async compareAndSwap(expectedRevision, record) {
      const previous = records.get(record.id);
      if (previous === undefined || previous.revision !== expectedRevision ||
        !isAllowedOutboxTransition(previous, record)) return false;
      records.set(record.id, structuredClone(record));
      return true;
    },
    async list() {
      return structuredClone([...records.values()]);
    },
    async read(id) {
      const record = records.get(id);
      return record === undefined ? undefined : structuredClone(record);
    },
  };
};
