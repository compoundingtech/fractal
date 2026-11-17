import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createMemoryOutboxRepository } from '../../clients/typescript/fractal-client/testSupport.ts';
import { createOutboxRecord, prepareOutboxRecord, markOutboxDispatched, observeOutbox, restoreOutboxEntries } from '../../clients/typescript/fractal-client/outbox.ts';
import { prepareMessageSendPacket } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { createSqliteOutboxRepository, migrateOutboxDatabase, OutboxCorruptionError, withOutboxTransaction } from './outboxSqlite.ts';
import { restoreBoundAttempt } from './outboxBinding.ts';
import { importSavedThreadStarts, startRecoveryKey } from './threadStartRecovery.ts';

const scope = { principalId: 'profile/avery', deploymentId: '["host/original","key/original"]', memberId: 'member/original' };
const intent = () => ({ text: 'Original text', attachments: [{ reference: 'spool/original', contentHash: 'sha256:original', mediaType: 'image/png' }] });
const packet = prepareMessageSendPacket({ id: 'action/original', idempotency_key: 'original-key-at-least-16',
  fence: { snapshot_id: 'snapshot/original', subject_revisions: {} }, parameters: { to: 'agent/dotfiles/manager', content: 'Original text' } });
const preparing = (id = 'attempt/original') => createOutboxRecord(id, intent());
const prepared = (id = 'attempt/original') => prepareOutboxRecord(preparing(id), packet, ['message/original']);

// Real SQLite SQL/transaction executor, not a repository fake. Each test owns its file.
const nodeDatabase = path => {
  const db = new DatabaseSync(path);
  const session = {
    async exec(sql) { db.exec(sql); },
    async run(sql, parameters) { const result = db.prepare(sql).run(...parameters); return { changes: Number(result.changes) }; },
    async get(sql, parameters) { return db.prepare(sql).get(...parameters); },
    async all(sql, parameters) { return db.prepare(sql).all(...parameters); },
  };
  let pending = Promise.resolve();
  return {
    db,
    executor: {
      ...session,
      transaction(operation) {
        const result = pending.then(() => withOutboxTransaction(session, operation));
        pending = result.then(() => undefined, () => undefined);
        return result;
      },
    },
    close() { db.close(); },
  };
};
const sqliteFactory = async t => {
  const directory = mkdtempSync(join(tmpdir(), 'fractal-ios-outbox-'));
  const path = join(directory, 'outbox.sqlite');
  let connection = nodeDatabase(path);
  t.after(() => { connection.close(); rmSync(directory, { recursive: true }); });
  await migrateOutboxDatabase(connection.executor);
  return {
    repository: selectedScope => createSqliteOutboxRepository(connection.executor, selectedScope),
    connection: () => connection,
    async reopen() { connection.close(); connection = nodeDatabase(path); await migrateOutboxDatabase(connection.executor); },
  };
};

// The core exports a reference repository but no runnable shared contract suite. These
// four contracts mirror its acceptance/CAS/immutability/scope suite and run against BOTH.
const contracts = (name, factory) => {
  test(`${name}: acceptance is once-only with fixed scope`, async t => {
    const store = await factory(t), originalScope = { ...scope };
    const repo = store.repository(originalScope);
    originalScope.memberId = 'member/other';
    assert.equal(await repo.commit(preparing()), true);
    assert.equal(await repo.commit(preparing()), false);
    assert.deepEqual(repo.scope, scope);
    assert.equal(Object.isFrozen(repo.scope), true);
    assert.equal(await repo.read('absent'), undefined);
  });
  test(`${name}: only one preparation and dispatch CAS owner`, async t => {
    const store = await factory(t), repo = store.repository(scope);
    await repo.commit(preparing());
    const ready = prepared();
    assert.deepEqual(await Promise.all([repo.compareAndSwap(0, ready), repo.compareAndSwap(0, ready)]), [true, false]);
    const dispatched = markOutboxDispatched(ready);
    assert.equal(await repo.compareAndSwap(1, dispatched), true);
    assert.equal(await repo.compareAndSwap(1, dispatched), false);
    assert.deepEqual(await repo.list(), [{ _tag: 'record', record: dispatched }]);
  });
  test(`${name}: forged certainty, packet and intent transitions preserve stored record`, async t => {
    const store = await factory(t), repo = store.repository(scope);
    await repo.commit(preparing());
    const ready = prepared(); await repo.compareAndSwap(0, ready);
    const dispatched = markOutboxDispatched(ready); await repo.compareAndSwap(1, dispatched);
    const unknown = observeOutbox(dispatched, { kind: 'failure', applied: 'unknown' });
    assert.equal(await repo.compareAndSwap(2, unknown), true);
    for (const forged of [
      { ...ready, revision: 4, state: 'not-applied', proof: { applied: 'none' } },
      { ...unknown, revision: 4, intent: { ...intent(), text: 'Changed' } },
      { ...ready, revision: 4, packet: { ...packet, parameters: { ...packet.parameters, content: 'Changed' } } },
    ]) assert.equal(await repo.compareAndSwap(3, forged), false);
    assert.deepEqual(await repo.read(unknown.id), unknown);
    const delivered = observeOutbox(unknown, { kind: 'observed', source: 'timeline', identity: 'message/original' });
    assert.equal(await repo.compareAndSwap(3, delivered), true);
    assert.deepEqual(await repo.read(unknown.id), delivered);
  });
  test(`${name}: snapshots and other scopes cannot mutate or see records`, async t => {
    const store = await factory(t), repo = store.repository(scope);
    const mutable = intent(); await repo.commit(createOutboxRecord('attempt/original', mutable));
    mutable.text = 'Changed after commit';
    const snapshot = await repo.read('attempt/original');
    assert.equal(snapshot.intent.text, 'Original text');
    snapshot.intent.text = 'Changed snapshot';
    assert.equal((await repo.read(snapshot.id)).intent.text, 'Original text');
    for (const otherScope of [{ ...scope, principalId: 'profile/ada' }, { ...scope, memberId: 'member/other' }, { ...scope, deploymentId: 'deployment/other' }]) {
      const other = store.repository(otherScope);
      assert.deepEqual(await other.list(), []);
      assert.equal(await other.read(snapshot.id), undefined);
      assert.equal(await other.compareAndSwap(0, prepared()), false);
    }
  });
};
contracts('shared memory', async () => {
  const repositories = new Map();
  return { repository(selectedScope) {
    const key = JSON.stringify(selectedScope);
    if (!repositories.has(key)) repositories.set(key, createMemoryOutboxRepository(selectedScope));
    return repositories.get(key);
  } };
});
contracts('SQLite', sqliteFactory);

test('SQLite migrates empty schema once, rejects future version without resetting data', async t => {
  const store = await sqliteFactory(t), { db, executor } = store.connection();
  assert.equal(db.prepare('PRAGMA user_version').get().user_version, 3);
  assert.equal(db.prepare('PRAGMA synchronous').get().synchronous, 2);
  await store.repository(scope).commit(preparing());
  await migrateOutboxDatabase(executor);
  assert.equal((await store.repository(scope).list()).length, 1);
  db.exec('PRAGMA user_version = 4;');
  await assert.rejects(migrateOutboxDatabase(executor), OutboxCorruptionError);
  assert.equal(db.prepare('SELECT COUNT(*) AS count FROM outbox_records').get().count, 1);
  assert.equal(db.prepare('PRAGMA user_version').get().user_version, 4);
});

test('SQLite file reopen retains intent/ready packets and restores dispatched work to unknown', async t => {
  const store = await sqliteFactory(t);
  let repo = store.repository(scope);
  for (const id of ['attempt/preparing', 'attempt/ready', 'attempt/dispatched']) await repo.commit(preparing(id));
  await repo.compareAndSwap(0, prepared('attempt/ready'));
  const ready = prepared('attempt/dispatched');
  await repo.compareAndSwap(0, ready); await repo.compareAndSwap(1, markOutboxDispatched(ready));
  await store.reopen(); repo = store.repository(scope);
  assert.deepEqual(await repo.read('attempt/preparing'), preparing('attempt/preparing'));
  assert.deepEqual(await repo.read('attempt/ready'), prepared('attempt/ready'));
  const restored = await restoreBoundAttempt(repo, { mode: 'bound', scope, authorized: true, notice: undefined }, 'attempt/dispatched');
  assert.equal(restored.record.state, 'unknown');
  assert.equal(markOutboxDispatched(restored.record), undefined);
  await store.reopen();
  assert.deepEqual(await store.repository(scope).read('attempt/dispatched'), restored.record);
});

test('SQLite transaction rollback preserves original data after a write error', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), { db } = store.connection();
  await repo.commit(preparing());
  db.exec("CREATE TRIGGER refuse_update BEFORE UPDATE ON outbox_records BEGIN SELECT RAISE(ABORT, 'write refused'); END;");
  await assert.rejects(repo.compareAndSwap(0, prepared()), /write refused/);
  assert.deepEqual(await repo.read('attempt/original'), preparing());
  db.exec('DROP TRIGGER refuse_update;');
  assert.equal(await repo.compareAndSwap(0, prepared()), true);
});

for (const corruption of ['json', 'revision', 'packet']) test(`SQLite ${corruption} corruption fails closed as unknown without exposing a sendable packet`, async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), { db } = store.connection();
  await repo.commit(preparing());
  if (corruption === 'json') db.prepare('UPDATE outbox_records SET record_json = ?').run('{broken');
  else if (corruption === 'revision') db.exec('UPDATE outbox_records SET revision = 99');
  else {
    const broken = { ...prepared(), packet: { ...packet, parameters: { content: 'missing recipient' } } };
    db.prepare('UPDATE outbox_records SET revision = 1, record_json = ?').run(JSON.stringify(broken));
  }
  const unknown = error => error instanceof OutboxCorruptionError && error.state === 'unknown';
  await assert.rejects(repo.read('attempt/original'), unknown);
  const entries = await repo.list();
  assert.deepEqual(entries, [{ _tag: 'corrupt', id: 'attempt/original', revision: corruption === 'revision' ? 99 : corruption === 'packet' ? 1 : 0 }]);
  const recovery = restoreOutboxEntries(entries)[0];
  assert.equal(recovery.state, 'unknown');
  assert.equal(recovery.status, 'storage damaged; delivery unknown');
  assert.deepEqual(recovery.actions, ['discard']);
  assert.equal(recovery.expires, false);
  assert.equal('packet' in recovery, false);
  await assert.rejects(restoreBoundAttempt(repo, { mode: 'bound', scope, authorized: true, notice: undefined }, 'attempt/original'), unknown);
  await assert.rejects(repo.compareAndSwap(0, prepared()), unknown);
});

test('rollback failure retains the original operation error as cause and aggregate member', async () => {
  const original = new Error('original write failure'), rollback = new Error('rollback failure');
  const statements = [];
  const session = { async exec(sql) { statements.push(sql); if (sql === 'ROLLBACK;') throw rollback; } };
  await assert.rejects(withOutboxTransaction(session, async () => { throw original; }), error =>
    error instanceof AggregateError && error.cause === original
      && error.errors[0] === original && error.errors[1] === rollback);
  assert.deepEqual(statements, ['BEGIN IMMEDIATE;', 'ROLLBACK;']);
});

test('SQLite list retains good records beside persistent corrupt-row recovery entries', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), { db } = store.connection();
  await repo.commit(preparing('attempt/good'));
  await repo.commit(preparing('attempt/damaged'));
  db.prepare('UPDATE outbox_records SET record_json = ? WHERE id = ?').run('broken', 'attempt/damaged');
  const expected = [{ _tag: 'corrupt', id: 'attempt/damaged', revision: 0 },
    { _tag: 'record', record: preparing('attempt/good') }];
  assert.deepEqual(await repo.list(), expected);
  await store.reopen();
  assert.deepEqual(await store.repository(scope).list(), expected);
});

test('SQLite persists append-only upload checkpoints and forbids changing them after preparation', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope);
  await repo.commit(preparing());
  const uploads = [{ blob: 'blob/original', media_type: 'image/png' }];
  await repo.saveUploads('attempt/original', uploads);
  await store.reopen();
  const reopened = store.repository(scope);
  assert.deepEqual(await reopened.uploads('attempt/original'), uploads);
  await assert.rejects(reopened.saveUploads('attempt/original', [{ blob: 'blob/other', media_type: 'image/png' }]), OutboxCorruptionError);
  assert.equal(await reopened.compareAndSwap(0, prepared()), true);
  await assert.rejects(reopened.saveUploads('attempt/original', uploads), OutboxCorruptionError);
});

const terminal = async (repo, id, attachment = 'a'.repeat(64)) => {
  const intent = { text: 'terminal payload', attachments: [{ reference: 'spool/' + attachment, contentHash: 'sha256:' + attachment, mediaType: 'image/png' }] };
  const initial = createOutboxRecord(id, intent);
  await repo.commit(initial);
  const ready = prepareOutboxRecord(initial, packet, ['message/original']);
  await repo.compareAndSwap(0, ready);
  const dispatched = markOutboxDispatched(ready);
  await repo.compareAndSwap(1, dispatched);
  const delivered = observeOutbox(dispatched, { kind: 'observed', identity: 'message/original', source: 'timeline' });
  await repo.compareAndSwap(2, delivered);
  return delivered;
};

test('SQLite seven-day terminal retention waits for exact timeline observation; tombstones prevent reuse for 30 days', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), now = Date.now();
  const delivered = await terminal(repo, 'attempt/terminal');
  store.connection().db.prepare('UPDATE outbox_records SET timeline_observed = 0').run();
  const deleted = [];
  await repo.cleanup(now + 8 * 86400_000, async ref => deleted.push(ref));
  assert.deepEqual(await repo.read(delivered.id), delivered);
  await repo.observeTimeline(delivered.id, 'message/other');
  await repo.cleanup(now + 8 * 86400_000, async ref => deleted.push(ref));
  assert.deepEqual(await repo.read(delivered.id), delivered);
  await repo.observeTimeline(delivered.id, 'message/original');
  await repo.cleanup(now + 8 * 86400_000, async ref => deleted.push(ref));
  assert.equal(await repo.read(delivered.id), undefined);
  assert.deepEqual(deleted, ['spool/' + 'a'.repeat(64)]);
  assert.equal(await repo.commit(preparing(delivered.id)), false);
  await store.reopen();
  const reopened = store.repository(scope);
  assert.equal(await reopened.commit(preparing(delivered.id)), false);
  await reopened.cleanup(now + 39 * 86400_000, async () => {});
  assert.equal(await reopened.commit(preparing(delivered.id)), true);
});

test('SQLite never expires pending or damaged rows and preserves files shared across scopes', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), now = Date.now();
  await terminal(repo, 'attempt/terminal');
  const other = store.repository({ ...scope, principalId: 'profile/ada' });
  const shared = createOutboxRecord('attempt/pending', { text: 'must stay', attachments: [{ reference: 'spool/' + 'a'.repeat(64), contentHash: 'sha256:' + 'a'.repeat(64), mediaType: 'image/png' }] });
  await other.commit(shared);
  await repo.commit(preparing('attempt/damaged'));
  store.connection().db.prepare("UPDATE outbox_records SET record_json = 'broken' WHERE id = 'attempt/damaged'").run();
  const deleted = [];
  await repo.cleanup(now + 400 * 86400_000, async ref => deleted.push(ref));
  assert.equal((await repo.list())[0]._tag, 'corrupt');
  assert.deepEqual(await other.read(shared.id), shared);
  assert.deepEqual(deleted, []);
  await repo.discard('attempt/damaged');
  await repo.cleanup(now + 400 * 86400_000, async ref => deleted.push(ref));
  assert.deepEqual(deleted, []);
  assert.deepEqual(await other.read(shared.id), shared);
});

test('SQLite spool deletion queue survives failure/reopen; explicit pending discard never deletes files', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope), now = Date.now();
  await terminal(repo, 'attempt/terminal');
  await assert.rejects(repo.cleanup(now + 8 * 86400_000, async () => { throw new Error('locked'); }), /locked/);
  await store.reopen();
  const reopened = store.repository(scope), deleted = [];
  await reopened.cleanup(now + 8 * 86400_000, async ref => deleted.push(ref));
  assert.deepEqual(deleted, ['spool/' + 'a'.repeat(64)]);
  await reopened.commit(preparing('attempt/pending'));
  await reopened.discard('attempt/pending');
  await reopened.cleanup(now + 100 * 86400_000, async () => assert.fail('unknown files are not authorized for deletion'));
});

test('legacy Start import commits packetless metadata once; discard prevents reimport after restart', async t => {
  const store = await sqliteFactory(t), repo = store.repository(scope);
  const entry = { id: 'legacy-start/thread/original', message: { kind: 'start', target: 'thread/original', title: 'Original',
    origin: { agentId: 'agent/dotfiles/manager', sourceId: 'notice/original', episode: 'episode/original' } }, text: 'saved text', serializedDraft: '{\"text\":\"saved text\"}' };
  assert.equal(await repo.importLegacyStart(entry), true);
  await store.reopen();
  const reopened = store.repository(scope);
  assert.equal(await reopened.importLegacyStart(entry), true);
  assert.equal(await reopened.importLegacyStart({ ...entry, text: 'later changed' }), false);
  assert.equal(await reopened.importLegacyStart({ ...entry, serializedDraft: '{"text":"later changed"}' }), false);
  assert.deepEqual(await reopened.legacyStarts(), [entry]);
  assert.deepEqual(await reopened.list(), []);
  await reopened.discard(entry.id);
  assert.equal(await reopened.importLegacyStart(entry), false);
  assert.deepEqual(await reopened.legacyStarts(), []);
});

for (const sequence of ['edit-during-import', 'edit-after-failed-save']) {
  test(`SQLite legacy Start migration retains newer source on second pass: ${sequence}`, async t => {
    const store = await sqliteFactory(t), repo = store.repository(scope);
    const start = { target: 'thread/original', title: 'Original', origin: {
      agentId: 'agent/dotfiles/manager', sourceId: 'notice/original', episode: 'episode/original',
    } };
    const key = 'thread-draft/start/' + start.target, discoveryKey = startRecoveryKey(start.target);
    const original = JSON.stringify({ text: 'original', quote: { id: 'message/original' } });
    const newer = JSON.stringify({ text: 'later edit', quote: { id: 'message/later' } });
    const discovery = JSON.stringify(start), cache = new Map([[key, original], [discoveryKey, discovery]]);
    let saves = 0;
    if (sequence === 'edit-during-import') {
      await importSavedThreadStarts(cache, () => true, async entry => {
        const retained = await repo.importLegacyStart(entry);
        cache.set(key, newer);
        return retained;
      }, async () => { saves++; });
    } else {
      await assert.rejects(importSavedThreadStarts(cache, () => true,
        entry => repo.importLegacyStart(entry), async () => { throw new Error('source save failed'); }), /source save failed/);
      assert.equal(cache.get(key), original);
      cache.set(key, newer);
    }
    await store.reopen();
    const reopened = store.repository(scope);
    await importSavedThreadStarts(cache, () => true, entry => reopened.importLegacyStart(entry), async () => { saves++; });
    assert.equal(cache.get(key), newer);
    assert.equal(cache.get(discoveryKey), discovery);
    assert.equal(saves, 0);
    const [retained] = await reopened.legacyStarts();
    assert.equal(retained.serializedDraft, original);
    assert.equal(retained.text, 'original');
  });
}
