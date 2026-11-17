import * as Crypto from 'expo-crypto';
import type { SQLiteDatabase } from 'expo-sqlite';
import { createSqliteOutboxRepository, migrateOutboxDatabase, withOutboxTransaction } from './outboxSqlite.ts';
import type { OutboxSqlExecutor, OutboxSqlSession } from './outboxSqlite.ts';
import type { OutboxScope, OutboxRepository } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import FractalOutboxFiles from './modules/fractal-outbox-files/index.ts';
import { acceptProtectedOutboxIntent, openProtectedOutboxStorage, readOutboxAttachment, OutboxStorageProtectionError, outboxStorageOperation } from './outboxSpool.ts';
import type { OutboxHash } from './outboxSpool.ts';
import type { Picked } from './images.ts';

const arrayBufferBacked = (bytes: Uint8Array): bytes is Uint8Array<ArrayBuffer> => bytes.buffer instanceof ArrayBuffer;

const sessionFor = (database: SQLiteDatabase): OutboxSqlSession => ({
  exec: sql => outboxStorageOperation(() => database.execAsync(sql)),
  run: (sql, parameters) => outboxStorageOperation(() => database.runAsync(sql, [...parameters])),
  get: async <TRow>(sql: string, parameters: readonly (string | number | null)[]) =>
    (await outboxStorageOperation(() => database.getFirstAsync<TRow>(sql, [...parameters]))) ?? undefined,
  all: <TRow>(sql: string, parameters: readonly (string | number | null)[]) =>
    outboxStorageOperation(() => database.getAllAsync<TRow>(sql, [...parameters])),
});

const executorFor = (database: SQLiteDatabase): OutboxSqlExecutor & { close(): Promise<void> } => {
  const session = sessionFor(database);
  let pending = Promise.resolve();
  const serialize = <TResult>(operation: () => Promise<TResult>): Promise<TResult> => {
    const result = pending.then(operation);
    pending = result.then(() => undefined, () => undefined);
    return result;
  };
  return {
    close: () => serialize(() => database.closeAsync()),
    exec: sql => serialize(() => session.exec(sql)),
    run: (sql, parameters) => serialize(() => session.run(sql, parameters)),
    get: (sql, parameters) => serialize(() => session.get(sql, parameters)),
    all: (sql, parameters) => serialize(() => session.all(sql, parameters)),
    transaction: operation => serialize(() => withOutboxTransaction(session, operation)),
  };
};

/** Explicit owner lifecycle; closing/reopening never resets records or changes their scope. */
export const openNativeOutbox = async (scope: OutboxScope, authorize: (scope: OutboxScope) => void) => {
  const bridge = FractalOutboxFiles;
  return openProtectedOutboxStorage(bridge, async protection => {
    if (!bridge) throw new OutboxStorageProtectionError();
    // Load SQLite only after the optional native protection module has passed read-back.
    // An old installed binary therefore fails as unprotected-storage, not a memory fallback.
    const { defaultDatabaseDirectory, openDatabaseAsync } = await import('expo-sqlite');
    const directory = `${defaultDatabaseDirectory}/fractal-outbox`;
    if (protection.databaseDirectory !== directory) throw new OutboxStorageProtectionError();
    const database = await openDatabaseAsync('outbox.sqlite', { useNewConnection: true }, directory);
    try {
      const executor = executorFor(database);
      await migrateOutboxDatabase(executor);
      const storage = createSqliteOutboxRepository(executor, scope);
      // Read/list remain available under the persisted scope during re-pair recovery.
      // All acceptance/transition writes require the live, original client's identity.
      const repository: OutboxRepository<PreparedPacket> = {
        scope: storage.scope, read: id => storage.read(id), list: () => storage.list(),
        commit: record => { authorize(scope); return storage.commit(record); },
        compareAndSwap: (revision, record) => { authorize(scope); return storage.compareAndSwap(revision, record); },
      };
      const sha256: OutboxHash = async bytes => new Uint8Array(await Crypto.digest(
        Crypto.CryptoDigestAlgorithm.SHA256, arrayBufferBacked(bytes) ? bytes : new Uint8Array(bytes),
      ));
      return {
        repository, close: () => executor.close(),
        // Future status/dispatch owners must invoke this immediately before network work.
        authorize: () => authorize(scope),
        acceptIntent: (id: string, text: string, images: readonly Picked[] = []) =>
          acceptProtectedOutboxIntent(repository, bridge, directory, sha256, id, text, images),
        readAttachment: (reference: string) => readOutboxAttachment(bridge, directory, sha256, reference),
      };
    } catch (error) {
      try { await database.closeAsync(); }
      catch (closeError) { throw new AggregateError([error, closeError], 'Outbox open and close failed', { cause: error }); }
      throw error;
    }
  });
};
