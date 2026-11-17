import { isAllowedOutboxTransition } from '../../clients/typescript/fractal-client/outbox.ts';
import type { OutboxEntry, OutboxRecord, OutboxRepository, OutboxScope } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { ActionRequest, ActionResult, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import type { AttachmentInput } from '../../clients/typescript/fractal-client/Models.generated.ts';

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

export interface IosOutboxRepository extends OutboxRepository<PreparedPacket> {
  uploads(id: string): Promise<readonly AttachmentInput[]>;
  saveUploads(id: string, attachments: readonly AttachmentInput[]): Promise<void>;
  observeTimeline(id: string, identity: string): Promise<void>;
  discard(id: string): Promise<void>;
  cleanup(now: number, deleteAttachment: (reference: string) => Promise<void>): Promise<void>;
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
    if (version?.user_version === 2) return;
    if (version?.user_version !== 0 && version?.user_version !== 1) throw new OutboxCorruptionError();
    if (version.user_version === 0) await sql.exec(`CREATE TABLE outbox_records (
      principal_id TEXT NOT NULL,
      deployment_id TEXT NOT NULL,
      member_id TEXT NOT NULL,
      id TEXT NOT NULL,
      revision INTEGER NOT NULL CHECK (revision >= 0),
      state TEXT NOT NULL CHECK (state IN ('pending', 'unknown', 'delivered', 'not-applied')),
      record_json TEXT NOT NULL,
      PRIMARY KEY (principal_id, deployment_id, member_id, id)
    );`);
    await sql.exec(`ALTER TABLE outbox_records ADD COLUMN terminal_at INTEGER;
      ALTER TABLE outbox_records ADD COLUMN timeline_observed INTEGER NOT NULL DEFAULT 0;
      ALTER TABLE outbox_records ADD COLUMN uploads_json TEXT NOT NULL DEFAULT '[]';
      CREATE TABLE outbox_tombstones (
        principal_id TEXT NOT NULL, deployment_id TEXT NOT NULL, member_id TEXT NOT NULL,
        id TEXT NOT NULL, state TEXT NOT NULL, expires_at INTEGER NOT NULL,
        PRIMARY KEY (principal_id, deployment_id, member_id, id)
      );
      CREATE TABLE outbox_spool_deletions (reference TEXT PRIMARY KEY);
      PRAGMA user_version = 2;`);
    // Existing terminal records get a fresh retention interval, never immediate deletion.
    await sql.run("UPDATE outbox_records SET terminal_at = ? WHERE state IN ('delivered', 'not-applied');", [Date.now()]);
  });
};

/** Open/migrate once before constructing repositories. Every instance has an immutable scope. */
export const createSqliteOutboxRepository = (
  executor: OutboxSqlExecutor, originalScope: OutboxScope,
): IosOutboxRepository => {
  const scope = Object.freeze({ ...originalScope });
  if (!scope.principalId || !scope.deploymentId || !scope.memberId) throw new Error('Outbox scope is required');
  const parameters = [scope.principalId, scope.deploymentId, scope.memberId];
  const remove = async (sql: OutboxSqlSession, row: Row, now: number): Promise<void> => {
    let terminal: OutboxRecord<PreparedPacket> | undefined;
    try {
      const record = decodeRow(row);
      if (record.state === 'delivered' || record.state === 'not-applied') terminal = record;
    } catch (error) { if (!(error instanceof OutboxCorruptionError)) throw error; }
    await sql.run(`INSERT INTO outbox_tombstones (principal_id, deployment_id, member_id, id, state, expires_at)
      VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING;`, [...parameters, row.id, terminal?.state ?? 'discarded-unknown', now + 30 * 86400_000]);
    await sql.run(`DELETE FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, row.id]);
    // Explicit unknown/corrupt discard never authorizes deletion of unproven spool files.
    for (const attachment of terminal?.intent.attachments ?? []) {
      if (/^spool\/[a-f0-9]{64}$/.test(attachment.reference)) {
        await sql.run('INSERT INTO outbox_spool_deletions (reference) VALUES (?) ON CONFLICT DO NOTHING;', [attachment.reference]);
      }
    }
  };
  return {
    scope,
    async uploads(id) {
      const row = await executor.get<{ uploads_json: string }>(`SELECT uploads_json FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, id]);
      if (!row) throw new OutboxCorruptionError();
      try {
        const value: unknown = JSON.parse(row.uploads_json);
        if (!Array.isArray(value) || !value.every(item => object(item) && typeof item.blob === 'string' && typeof item.media_type === 'string'
          && (item.name === undefined || typeof item.name === 'string'))) throw new OutboxCorruptionError();
        return value;
      } catch { throw new OutboxCorruptionError(); }
    },
    async saveUploads(id, attachments) {
      const snapshot = JSON.stringify(attachments);
      await executor.transaction(async sql => {
        const row = await sql.get<Row & { uploads_json: string }>(`SELECT ${columns}, uploads_json FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, id]);
        if (!row) throw new OutboxCorruptionError();
        const record = decodeRow(row);
        const prior: unknown = JSON.parse(row.uploads_json);
        if (record.state !== 'pending' || record.phase !== 'preparing' || record.mayHaveDispatched || !Array.isArray(prior)
          || attachments.length > record.intent.attachments.length || prior.length > attachments.length
          || !prior.every((item, index) => JSON.stringify(item) === JSON.stringify(attachments[index]))) throw new OutboxCorruptionError();
        await sql.run(`UPDATE outbox_records SET uploads_json = ? WHERE ${filter} AND id = ?;`, [snapshot, ...parameters, id]);
      });
    },
    async observeTimeline(id, identity) {
      await executor.transaction(async sql => {
        const row = await sql.get<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, id]);
        if (!row) return;
        const record = decodeRow(row);
        if (record.state === 'delivered' && record.expectedIdentities.includes(identity)) {
          await sql.run(`UPDATE outbox_records SET timeline_observed = 1 WHERE ${filter} AND id = ?;`, [...parameters, id]);
        }
      });
    },
    async discard(id) {
      await executor.transaction(async sql => {
        const row = await sql.get<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter} AND id = ?;`, [...parameters, id]);
        if (row) await remove(sql, row, Date.now());
      });
    },
    async cleanup(now, deleteAttachment) {
      await executor.transaction(async sql => {
        const rows = await sql.all<Row>(`SELECT ${columns} FROM outbox_records WHERE ${filter}
          AND terminal_at <= ? AND (state = 'not-applied' OR (state = 'delivered' AND timeline_observed = 1));`, [...parameters, now - 7 * 86400_000]);
        for (const row of rows) {
          // Damage never becomes eligible merely because denormalized SQL columns look terminal.
          try { const record = decodeRow(row); if (record.state !== 'delivered' && record.state !== 'not-applied') continue; }
          catch (error) { if (error instanceof OutboxCorruptionError) continue; throw error; }
          await remove(sql, row, now);
        }
        await sql.run(`DELETE FROM outbox_tombstones WHERE ${filter} AND expires_at <= ?;`, [...parameters, now]);
      });
      // Caller serializes cleanup with spool-before-commit acceptance across every scope.
      const rows = await executor.all<Row>(`SELECT ${columns} FROM outbox_records;`, []);
      const references = new Set<string>();
      for (const row of rows) {
        try { for (const attachment of decodeRow(row).intent.attachments) references.add(attachment.reference); }
        catch (error) { if (error instanceof OutboxCorruptionError) return; throw error; }
      }
      const deletions = await executor.all<{ reference: string }>('SELECT reference FROM outbox_spool_deletions;', []);
      for (const { reference } of deletions) {
        if (!references.has(reference)) await deleteAttachment(reference);
        await executor.run('DELETE FROM outbox_spool_deletions WHERE reference = ?;', [reference]);
      }
    },
    async commit(record) {
      // Snapshot before awaiting: callers cannot mutate accepted intent during a transaction.
      const encoded = encodeRecord(record);
      if (!encoded) return false;
      const { json, record: snapshot } = encoded;
      if (snapshot.revision !== 0 || snapshot.state !== 'pending' || snapshot.phase !== 'preparing'
        || snapshot.mayHaveDispatched || snapshot.ambiguous) return false;
      return executor.transaction(async sql => {
        const tombstone = await sql.get<{ id: string }>(`SELECT id FROM outbox_tombstones WHERE ${filter} AND id = ?;`, [...parameters, snapshot.id]);
        if (tombstone) return false;
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
        const terminal = replacement.state === 'delivered' || replacement.state === 'not-applied';
        const observed = replacement.state === 'delivered' && replacement.evidence.kind === 'timeline';
        const result = await sql.run(`UPDATE outbox_records SET revision = ?, state = ?, record_json = ?,
          terminal_at = CASE WHEN ? THEN COALESCE(terminal_at, ?) ELSE terminal_at END,
          timeline_observed = MAX(timeline_observed, ?)
          WHERE ${filter} AND id = ? AND revision = ?;`,
        [replacement.revision, replacement.state, json, terminal ? 1 : 0, Date.now(), observed ? 1 : 0,
          ...parameters, replacement.id, expectedRevision]);
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
