import { defaultDatabaseDirectory, openDatabaseAsync } from 'expo-sqlite';
import type { SQLiteDatabase } from 'expo-sqlite';
import { createSqliteOutboxRepository, migrateOutboxDatabase } from './outboxSqlite.ts';
import type { OutboxSqlExecutor, OutboxSqlSession } from './outboxSqlite.ts';
import type { OutboxScope } from '../../clients/typescript/fractal-client/outbox.ts';

// Expo's default database directory is under app-private Documents on iOS, not cache.
// Keep all outbox files under one root for the step-3 backup-exclusion bridge.
// This step does NOT establish exclusion and is not wired into production sending.
export const OUTBOX_DATABASE_DIRECTORY = `${defaultDatabaseDirectory}/fractal-outbox`;

const sessionFor = (database: SQLiteDatabase): OutboxSqlSession => ({
  exec: sql => database.execAsync(sql),
  run: (sql, parameters) => database.runAsync(sql, [...parameters]),
  get: async <TRow>(sql: string, parameters: readonly (string | number | null)[]) =>
    (await database.getFirstAsync<TRow>(sql, [...parameters])) ?? undefined,
  all: <TRow>(sql: string, parameters: readonly (string | number | null)[]) =>
    database.getAllAsync<TRow>(sql, [...parameters]),
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
    transaction: operation => serialize(async () => {
      await session.exec('BEGIN IMMEDIATE;');
      try {
        const value = await operation(session);
        await session.exec('COMMIT;');
        return value;
      } catch (error) {
        await session.exec('ROLLBACK;');
        throw error;
      }
    }),
  };
};

/** Explicit owner lifecycle; closing/reopening never resets records or changes their scope. */
export const openNativeOutbox = async (scope: OutboxScope) => {
  const database = await openDatabaseAsync('outbox.sqlite', { useNewConnection: true }, OUTBOX_DATABASE_DIRECTORY);
  try {
    const executor = executorFor(database);
    await migrateOutboxDatabase(executor);
    return { repository: createSqliteOutboxRepository(executor, scope), close: () => executor.close() };
  } catch (error) {
    await database.closeAsync();
    throw error;
  }
};
