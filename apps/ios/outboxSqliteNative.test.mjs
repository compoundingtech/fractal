import test from 'node:test';
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { DatabaseSync } from 'node:sqlite';
import { createOutboxRecord } from '../../clients/typescript/fractal-client/outbox.ts';
import { OutboxBindingUnavailableError } from './outboxBinding.ts';

// Native dependency ports only; run the actual native wrapper and repository SQL in Node.
// This does not establish Expo/Swift compilation or installed storage behavior.
const ports = {};
const modules = {
  'expo-crypto': 'export const CryptoDigestAlgorithm = { SHA256: "SHA-256" }; export const digest = (...args) => crypto.subtle.digest(...args);',
  'expo': 'export const requireOptionalNativeModule = () => globalThis.testOutboxNativePorts.bridge;',
  'expo-sqlite': 'export const defaultDatabaseDirectory = "/private/test/SQLite"; export const openDatabaseAsync = (...args) => globalThis.testOutboxNativePorts.open(...args);',
};
globalThis.testOutboxNativePorts = ports;
const hooks = registerHooks({
  resolve(specifier, context, next) {
    return Object.hasOwn(modules, specifier)
      ? { url: `data:text/javascript,${encodeURIComponent(modules[specifier])}`, shortCircuit: true }
      : next(specifier, context);
  },
});

// The optional module is captured on import; its methods delegate to each test's ports.
ports.bridge = { prepareStorage: () => ports.prepareStorage() };
const { openNativeOutbox } = await import('./outboxSqliteNative.ts');

test('native wrapper snapshots scope before await and authorizes only the immutable storage scope', async t => {
  t.after(() => { hooks.deregister(); delete globalThis.testOutboxNativePorts; });
  let release;
  ports.prepareStorage = () => new Promise(resolve => { release = resolve; });
  const db = new DatabaseSync(':memory:');
  let closed = false;
  ports.open = async () => ({
    execAsync: async sql => db.exec(sql),
    runAsync: async (sql, parameters) => ({ changes: Number(db.prepare(sql).run(...parameters).changes) }),
    getFirstAsync: async (sql, parameters) => db.prepare(sql).get(...parameters),
    getAllAsync: async (sql, parameters) => db.prepare(sql).all(...parameters),
    closeAsync: async () => { db.close(); closed = true; },
  });
  let owner;
  t.after(async () => { if (owner) await owner.close(); else if (!closed) db.close(); });
  const requested = { principalId: 'profile/avery', memberId: 'member/original', deploymentId: '["host/original","key/original"]' };
  const original = { ...requested };
  let authorizedPrincipal = 'profile/ada';
  const checkedScopes = [];
  const opening = openNativeOutbox(requested, captured => {
    checkedScopes.push(captured);
    assert.equal(Object.isFrozen(captured), true);
    if (captured.principalId !== authorizedPrincipal) throw new OutboxBindingUnavailableError();
  });
  // Mutation while the native protection opener is pending must not change the DB scope.
  requested.principalId = 'profile/ada';
  requested.memberId = 'member/other';
  requested.deploymentId = '["host/other","key/other"]';
  release({ databaseDirectory: '/private/test/SQLite/fractal-outbox', spoolDirectory: '/private/test/SQLite/fractal-outbox/spool',
    databaseExcludedFromBackup: true, spoolExcludedFromBackup: true });
  owner = await opening;
  assert.deepEqual(owner.repository.scope, original);
  const record = createOutboxRecord('attempt/original', { text: 'kept', attachments: [] });
  assert.throws(() => owner.repository.commit(record), OutboxBindingUnavailableError);
  assert.throws(() => owner.repository.compareAndSwap(0, record), OutboxBindingUnavailableError);
  assert.throws(() => owner.authorize(), OutboxBindingUnavailableError);
  assert.deepEqual(await owner.repository.list(), []);
  // Once the original profile is authorized, later caller mutations still cannot retarget it.
  authorizedPrincipal = 'profile/avery';
  assert.equal(await owner.repository.commit(record), true);
  owner.authorize();
  assert.deepEqual(await owner.repository.read(record.id), record);
  assert.ok(checkedScopes.every(captured => captured === owner.repository.scope));
  const rows = db.prepare('SELECT principal_id, member_id, deployment_id FROM outbox_records').all();
  assert.deepEqual(rows.map(row => ({ ...row })), [{ principal_id: original.principalId, member_id: original.memberId, deployment_id: original.deploymentId }]);
});
