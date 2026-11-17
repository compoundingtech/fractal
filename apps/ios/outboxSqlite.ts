import { isAllowedOutboxTransition } from '../../clients/typescript/fractal-client/outbox.ts';
import type { OutboxEntry, OutboxRecord, OutboxRepository, OutboxScope } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { ActionRequest, ActionResult, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';

export type SqlValue = string | number | null;
export interface OutboxSqlSession {
  exec(sql: string): Promise<void>;
  run(sql: string, parameters: readonly SqlValue[]): Promise<{ changes: number }>;
  get<TRow>(sql: string, parameters: readonly SqlValue[]): Promise<TRow | undefined>;
  all<TRow>(sql: string, parameters: readonly SqlValue[]): Promise<TRow[]>;
}
/** Serialize transactions and roll back on rejection; callbacks must use the supplied session. */
export interface OutboxSqlExecutor extends OutboxSqlSession {
  transaction<TResult>(operation: (session: OutboxSqlSession) => Promise<TResult>): Promise<TResult>;
}

/** Preserve the original failure even if rollback itself fails. Caller serializes access. */
export const withOutboxTransaction = async <TResult>(
  session: OutboxSqlSession, operation: (session: OutboxSqlSession) => Promise<TResult>,
): Promise<TResult> => {
  await session.exec('BEGIN IMMEDIATE;');
  try {
    const value = await operation(session);
    await session.exec('COMMIT;');
    return value;
  } catch (error) {
    try { await session.exec('ROLLBACK;'); }
    catch (rollbackError) {
      throw new AggregateError([error, rollbackError], 'Outbox transaction and rollback failed', { cause: error });
    }
    throw error;
  }
};

/** Missing/corrupt storage is unknown, never a fabricated ready packet or an empty outbox. */
export class OutboxCorruptionError extends Error {
  readonly state = 'unknown';
  constructor() { super('Outbox storage is unreadable; delivery remains unknown.'); this.name = 'OutboxCorruptionError'; }
}

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const validatePacket = decodeUnknownSync(ActionRequest, 'strict');
const validateReceipt = decodeUnknownSync(ActionResult, 'strict');

// Validate disk bytes at this one boundary, retaining wire DTOs rather than decoded Option fields.
const validRecord = (value: unknown): value is OutboxRecord<PreparedPacket> => {
  if (!object(value) || typeof value.id !== 'string' || !value.id || !integer(value.revision)
    || typeof value.mayHaveDispatched !== 'boolean' || typeof value.ambiguous !== 'boolean'
    || !object(value.intent) || typeof value.intent.text !== 'string' || !Array.isArray(value.intent.attachments)
    || !value.intent.attachments.every(item => object(item) && typeof item.contentHash === 'string'
      && typeof item.reference === 'string' && typeof item.mediaType === 'string'
      && (item.name === undefined || typeof item.name === 'string'))) return false;
  if (value.state === 'pending' && value.phase === 'preparing') {
    return !value.mayHaveDispatched && !value.ambiguous && !('packet' in value) && !('expectedIdentities' in value);
  }
  if (!object(value.packet) || !['message.send', 'thread.start', 'thread.reply'].includes(String(value.packet.type))
    || !strings(value.expectedIdentities)) return false;
  try { validatePacket(value.packet); } catch { return false; }
  switch (value.state) {
    case 'pending': return value.phase === 'ready' && !value.ambiguous;
    case 'unknown': return value.ambiguous && !('phase' in value);
    case 'not-applied': return value.mayHaveDispatched && !value.ambiguous && !('phase' in value)
      && object(value.proof) && value.proof.applied === 'none';
    case 'delivered': {
      if ('phase' in value || !object(value.evidence)) return false;
      if (value.evidence.kind === 'timeline') return value.evidence.source === 'timeline'
        && typeof value.evidence.identity === 'string' && value.expectedIdentities.includes(value.evidence.identity);
      if (value.evidence.kind !== 'receipt' || !object(value.evidence.receipt)
        || value.evidence.receipt.status !== 'completed') return false;
      try { validateReceipt(value.evidence.receipt); return true; } catch { return false; }
    }
    default: return false;
  }
};

const parseRecord = (json: string): OutboxRecord<PreparedPacket> => {
  let value: unknown;
  try { value = JSON.parse(json); } catch { throw new OutboxCorruptionError(); }
  if (!validRecord(value)) throw new OutboxCorruptionError();
  return value;
};
const encodeRecord = (record: OutboxRecord<PreparedPacket>): { json: string; record: OutboxRecord<PreparedPacket> } | undefined => {
  try {
    const json = JSON.stringify(record);
    return { json, record: parseRecord(json) };
  } catch { return undefined; }
};

type Row = { id: string; revision: number; state: string; record_json: string };
const decodeRow = (row: Row): OutboxRecord<PreparedPacket> => {
  const record = parseRecord(row.record_json);
  if (row.id !== record.id || row.revision !== record.revision || row.state !== record.state) throw new OutboxCorruptionError();
  return record;
};
const filter = 'principal_id = ? AND deployment_id = ? AND member_id = ?';
const columns = 'id, revision, state, record_json';

/** Dedicated database only: user_version belongs to this outbox schema. No destructive reset. */
export const migrateOutboxDatabase = async (executor: OutboxSqlExecutor): Promise<void> => {
  await executor.exec('PRAGMA synchronous = FULL;');
  await executor.transaction(async sql => {
    const version = await sql.get<{ user_version: number }>('PRAGMA user_version;', []);
    if (version?.user_version === 1) return;
    if (version?.user_version !== 0) throw new OutboxCorruptionError();
    await sql.exec(`CREATE TABLE outbox_records (
      principal_id TEXT NOT NULL,
      deployment_id TEXT NOT NULL,
      member_id TEXT NOT NULL,
      id TEXT NOT NULL,
      revision INTEGER NOT NULL CHECK (revision >= 0),
      state TEXT NOT NULL CHECK (state IN ('pending', 'unknown', 'delivered', 'not-applied')),
      record_json TEXT NOT NULL,
      PRIMARY KEY (principal_id, deployment_id, member_id, id)
    );
    PRAGMA user_version = 1;`);
  });
};

/** Open/migrate once before constructing repositories. Every instance has an immutable scope. */
export const createSqliteOutboxRepository = (
  executor: OutboxSqlExecutor, originalScope: OutboxScope,
): OutboxRepository<PreparedPacket> => {
  const scope = Object.freeze({ ...originalScope });
  if (!scope.principalId || !scope.deploymentId || !scope.memberId) throw new Error('Outbox scope is required');
  const parameters = [scope.principalId, scope.deploymentId, scope.memberId];
  return {
    scope,
    async commit(record) {
      // Snapshot before awaiting: callers cannot mutate accepted intent during a transaction.
      const encoded = encodeRecord(record);
      if (!encoded) return false;
      const { json, record: snapshot } = encoded;
      if (snapshot.revision !== 0 || snapshot.state !== 'pending' || snapshot.phase !== 'preparing'
        || snapshot.mayHaveDispatched || snapshot.ambiguous) return false;
      return executor.transaction(async sql => {
        const result = await sql.run(`INSERT INTO outbox_records
          (principal_id, deployment_id, member_id, id, revision, state, record_json)
          VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING;`,
        [...parameters, snapshot.id, snapshot.revision, snapshot.state, json]);
        return result.changes === 1;
      });
    },
    async compareAndSwap(expectedRevision, record) {
      const encoded = encodeRecord(record);
      if (!encoded) return false;
      const { json, record: replacement } = encoded;
      return executor.transaction(async sql => {
        const row = await sql.get<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, replacement.id]);
        if (!row) return false;
        const previous = decodeRow(row);
        if (previous.revision !== expectedRevision || !isAllowedOutboxTransition(previous, replacement)) return false;
        const result = await sql.run(`UPDATE outbox_records SET revision = ?, state = ?, record_json = ?
          WHERE ${filter} AND id = ? AND revision = ?;`,
        [replacement.revision, replacement.state, json, ...parameters, replacement.id, expectedRevision]);
        return result.changes === 1;
      });
    },
    async read(id) {
      const row = await executor.get<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, id]);
      return row ? decodeRow(row) : undefined;
    },
    async list() {
      const rows = await executor.all<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter} ORDER BY id;`, parameters);
      return rows.map<OutboxEntry<PreparedPacket>>(row => {
        try { return { _tag: 'record', record: decodeRow(row) }; }
        catch (error) {
          if (!(error instanceof OutboxCorruptionError)) throw error;
          return { _tag: 'corrupt', id: row.id, ...(integer(row.revision) ? { revision: row.revision } : {}) };
        }
      });
    },
  };
};
