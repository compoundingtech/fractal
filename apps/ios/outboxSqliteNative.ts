import * as Crypto from 'expo-crypto';
import type { SQLiteDatabase } from 'expo-sqlite';
import { createSqliteOutboxRepository, migrateOutboxDatabase, withOutboxTransaction } from './outboxSqlite.ts';
import type { IosOutboxRepository, OutboxSqlExecutor, OutboxSqlSession } from './outboxSqlite.ts';
import type { OutboxScope } from '../../clients/typescript/fractal-client/outbox.ts';
import type { PreparedPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import FractalOutboxFiles from './modules/fractal-outbox-files/index.ts';
import { acceptProtectedOutboxIntent, openProtectedOutboxStorage, readOutboxAttachment, requireProtectedOutboxStorage, OutboxStorageProtectionError, outboxStorageOperation } from './outboxSpool.ts';
import type { OutboxHash } from './outboxSpool.ts';
import type { Picked } from './images.ts';
import type { OutboxMessageMetadata } from './outboxMessage.ts';

// One filesystem acceptance/cleanup owner across all repository scopes.
let pendingFiles = Promise.resolve();
const serializeFiles = <TResult>(operation: () => Promise<TResult>): Promise<TResult> => {
  const result = pendingFiles.then(operation);
  pendingFiles = result.then(() => undefined, () => undefined);
  return result;
};

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
  const requestedScope = Object.freeze({ ...scope });
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
      const storage = createSqliteOutboxRepository(executor, requestedScope);
      // Read/list remain available under the persisted scope during re-pair recovery.
      // All acceptance/transition writes require the live, original client's identity.
      const repository: IosOutboxRepository = {
        scope: storage.scope, read: id => storage.read(id), list: () => storage.list(),
        uploads: id => storage.uploads(id),
        saveUploads: (id, attachments) => { authorize(storage.scope); return storage.saveUploads(id, attachments); },
        observeTimeline: (id, identity) => { authorize(storage.scope); return storage.observeTimeline(id, identity); },
        discard: id => storage.discard(id),
        cleanup: (now, deleteAttachment) => serializeFiles(() => storage.cleanup(now, deleteAttachment)),
        commit: record => { authorize(storage.scope); return storage.commit(record); },
        compareAndSwap: (revision, record) => { authorize(storage.scope); return storage.compareAndSwap(revision, record); },
      };
      const sha256: OutboxHash = async bytes => new Uint8Array(await Crypto.digest(
        Crypto.CryptoDigestAlgorithm.SHA256, arrayBufferBacked(bytes) ? bytes : new Uint8Array(bytes),
      ));
      return {
        repository, close: () => executor.close(),
        // Future status/dispatch owners must invoke this immediately before network work.
        authorize: () => authorize(storage.scope),
        acceptIntent: (id: string, text: string, images: readonly Picked[] = [], metadata?: OutboxMessageMetadata) => {
          const captured = images.map(image => ({ ...image }));
          const capturedMetadata: OutboxMessageMetadata | undefined = metadata ? JSON.parse(JSON.stringify(metadata)) : undefined;
          authorize(storage.scope);
          return serializeFiles(() => acceptProtectedOutboxIntent(repository, bridge, directory, sha256, id, text, captured, capturedMetadata));
        },
        cleanup: (now: number) => serializeFiles(async () => {
          await requireProtectedOutboxStorage(bridge, directory);
          await storage.cleanup(now, reference => outboxStorageOperation(() => bridge.deleteAttachment(reference)));
        }),
        readAttachment: (reference: string) => readOutboxAttachment(bridge, directory, sha256, reference),
      };
    } catch (error) {
      try { await database.closeAsync(); }
      catch (closeError) { throw new AggregateError([error, closeError], 'Outbox open and close failed', { cause: error }); }
      throw error;
    }
  });
};
