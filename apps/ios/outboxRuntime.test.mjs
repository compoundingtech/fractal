import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { createHash } from 'node:crypto';
import { API_VERSION, ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { createOutboxRecord, prepareOutboxRecord, markOutboxDispatched, observeOutbox } from '../../clients/typescript/fractal-client/outbox.ts';
import { prepareMessageSendPacket, prepareThreadStartPacket, prepareThreadReplyPacket, expectedPreparedPacketIdentities } from '../../clients/typescript/fractal-client/preparedPacket.ts';
import { createSqliteOutboxRepository, migrateOutboxDatabase, withOutboxTransaction } from './outboxSqlite.ts';
import { acceptProtectedOutboxIntent } from './outboxSpool.ts';
import { OutboxRuntime } from './outboxRuntime.ts';

const hash = async bytes => new Uint8Array(createHash('sha256').update(bytes).digest());
const scope = { principalId: 'profile/avery', deploymentId: '["host/original","key/original"]', memberId: 'member/original' };
const signature = { format: 'fields-v1', signed_fields: ['content'], signer: 'person/avery', key: 'key/original', chain: ['grant/original'], nonce: 'nonce/original-at-least-sixteen', signed_at_unix_ms: 1790000000000, signature: 'signature/original' };
const messages = [
  { kind: 'chat', to: 'agent/dotfiles/manager', sessionId: 'session/original', tags: ['dictated'] },
  { kind: 'start', target: 'thread/original', title: 'Title', origin: { agentId: 'agent/dotfiles/manager', sourceId: 'inbox/original', episode: 'episode/original' } },
  { kind: 'reply', target: 'thread/original', agentId: 'agent/dotfiles/manager', quoteId: 'message/quote' },
];
const metadata = message => ({ message, binding: { profileSlot: 'profile-slot', person: 'person/avery', signingKey: 'key/original', sessionActor: 'person/avery' }, acceptedAt: 1790000000000 });
const packetFor = (id, intent) => {
  const base = { id, idempotency_key: id + '-idempotency-key', fence: { snapshot_id: 'snapshot/original', subject_revisions: {} } };
  const message = intent.message;
  if (message.kind === 'chat') return prepareMessageSendPacket({ ...base, parameters: { to: message.to, content: intent.text, session_id: message.sessionId, tags: message.tags, signature } });
  if (message.kind === 'start') return prepareThreadStartPacket({ ...base, parameters: { target_id: message.target, title: message.title, content: intent.text, agent_id: message.origin.agentId,
    origin: { source_id: message.origin.sourceId, episode: message.origin.episode }, signature } });
  return prepareThreadReplyPacket({ ...base, parameters: { target_id: message.target, content: intent.text, thread_quote_id: message.quoteId, signature } });
};
const fixture = async t => {
  const db = new DatabaseSync(':memory:'); t.after(() => db.close());
  const session = { exec: async sql => db.exec(sql), run: async (sql, params) => ({ changes: Number(db.prepare(sql).run(...params).changes) }),
    get: async (sql, params) => db.prepare(sql).get(...params), all: async (sql, params) => db.prepare(sql).all(...params) };
  let queue = Promise.resolve();
  const executor = { ...session, transaction(operation) { const result = queue.then(() => withOutboxTransaction(session, operation)); queue = result.catch(() => {}); return result; } };
  await migrateOutboxDatabase(executor);
  const repository = createSqliteOutboxRepository(executor, scope), events = [], files = new Map();
  const commit = repository.commit, cas = repository.compareAndSwap;
  repository.commit = async record => { events.push('intent'); return commit(record); };
  repository.compareAndSwap = async (revision, record) => { events.push(record.mayHaveDispatched ? record.state === 'pending' ? 'dispatch-marker' : record.state : 'ready'); return cas(revision, record); };
  const bridge = {
    prepareStorage: async () => ({ databaseDirectory: '/private/outbox', spoolDirectory: '/private/outbox/spool', databaseExcludedFromBackup: true, spoolExcludedFromBackup: true }),
    writeAttachment: async (base64, contentHash) => { events.push('spool'); const reference = 'spool/' + contentHash.slice(7); files.set(reference, base64); return { reference, contentHash, bytes: Buffer.from(base64, 'base64').length }; },
  };
  let authorized = true;
  const owner = { repository, authorize: () => { if (!authorized) throw new Error('re-pair required'); },
    acceptIntent: (id, text, images, context) => acceptProtectedOutboxIntent(repository, bridge, '/private/outbox', hash, id, text, images, context),
    readAttachment: async reference => new Uint8Array(Buffer.from(files.get(reference), 'base64')),
    cleanup: now => repository.cleanup(now, async ref => files.delete(ref)), close: async () => {} };
  const transport = { authorize: () => owner.authorize(),
    prepare: async (record, intent) => { events.push('prepare'); return packetFor(record.id, intent); },
    post: async packet => {
      events.push('post'); const ids = await expectedPreparedPacketIdentities(packet, 'person/avery', hash);
      return { value: { kind: 'action-result', status: 'completed', action_id: packet.id, operation_id: 'operation/original', snapshot_id: 'snapshot/original',
        affected_ids: packet.type === 'message.send' ? ids : [...ids, packet.parameters.target_id] } };
    }, timeline: async () => [], sha256: hash, errorText: error => error.message };
  return { db, repository, owner, transport, events, files, runtime: () => new OutboxRuntime(owner, transport), setAuthorized: value => { authorized = value; } };
};

for (const message of messages) test(`${message.kind}: durable intent precedes preparation; packet/permit precede one exact POST`, async t => {
  const f = await fixture(t), runtime = f.runtime(), context = metadata(message);
  assert.deepEqual(await runtime.accept('action/original', 'hello', [], context), { how: 'accepted', outboxId: 'action/original' });
  await runtime.settled();
  assert.deepEqual(f.events, ['intent', 'prepare', 'ready', 'dispatch-marker', 'post', 'delivered']);
  const record = await f.repository.read('action/original');
  assert.equal(record.state, 'delivered');
  assert.deepEqual(record.packet, packetFor(record.id, { text: 'hello', ...context }));
  assert.deepEqual(record.intent.message, message);
  assert.equal(runtime.attempts.getSnapshot()[0].draft.state, 'delivered');
  await runtime.restore(); await runtime.settled();
  assert.equal(f.events.filter(event => event === 'post').length, 1);
});

test('images are privately spooled before intent acceptance, with draft snapshot unchanged', async t => {
  const f = await fixture(t), runtime = f.runtime();
  const images = [{ key: 'image', base64: Buffer.from('bytes').toString('base64'), bytes: 5, mediaType: 'image/png' }];
  const context = metadata(messages[0]);
  const accepted = runtime.accept('action/image', 'draft', images, context);
  images[0].base64 = 'later edit'; context.message.to = 'agent/changed';
  await accepted; await runtime.settled();
  assert.deepEqual(f.events.slice(0, 3), ['spool', 'intent', 'prepare']);
  assert.equal((await f.repository.read('action/image')).packet.parameters.to, 'agent/dotfiles/manager');
});

for (const message of messages) test(`${message.kind}: restart of ready never-dispatched record submits retained packet without signing/preparing`, async t => {
  const f = await fixture(t), context = metadata(message), initial = createOutboxRecord('action/ready', { text: 'ready', attachments: [], ...context });
  await f.repository.commit(initial);
  const packet = packetFor(initial.id, initial.intent), identities = await expectedPreparedPacketIdentities(packet, 'person/avery', hash);
  await f.repository.compareAndSwap(0, prepareOutboxRecord(initial, packet, identities));
  f.events.length = 0;
  f.transport.prepare = async () => assert.fail('ready record must not be re-signed');
  f.transport.post = async value => { assert.deepEqual(value, packet); throw new TypeError('reply lost'); };
  const restored = f.runtime(); await restored.restore(); await restored.settled();
  assert.equal((await f.repository.read(initial.id)).state, 'unknown');
  assert.equal(f.events.filter(event => event === 'dispatch-marker').length, 1);
  assert.equal(f.events.includes('prepare'), false);
});

for (const message of messages) test(`${message.kind}: dispatched restart stays unknown; absence and later none cannot permit replay`, async t => {
  const f = await fixture(t), initial = createOutboxRecord('action/dispatched', { text: 'maybe', attachments: [], ...metadata(message) });
  await f.repository.commit(initial);
  const packet = packetFor(initial.id, initial.intent), identities = await expectedPreparedPacketIdentities(packet, 'person/avery', hash);
  const ready = prepareOutboxRecord(initial, packet, identities); await f.repository.compareAndSwap(0, ready);
  await f.repository.compareAndSwap(1, markOutboxDispatched(ready));
  f.transport.prepare = f.transport.post = async () => assert.fail('unknown must not dispatch or prepare');
  const runtime = f.runtime(); await runtime.restore(); await runtime.settled();
  await runtime.checkStatus(initial.id);
  let unknown = await f.repository.read(initial.id);
  assert.equal(unknown.state, 'unknown');
  assert.equal(observeOutbox(unknown, { kind: 'failure', applied: 'none' }).state, 'unknown');
  f.transport.timeline = async () => [identities[0]];
  await runtime.checkStatus(initial.id);
  assert.equal((await f.repository.read(initial.id)).state, 'delivered');
  assert.deepEqual((await f.repository.read(initial.id)).packet, packet);
});

test('unauthorized restore exposes persisted entries but never prepares, reads status or dispatches', async t => {
  const f = await fixture(t), context = metadata({ ...messages[0] });
  const runtime = f.runtime();
  f.transport.prepare = async () => { f.setAuthorized(false); throw new Error('connection changed'); };
  await runtime.accept('action/bound', 'kept', [], context); await runtime.settled();
  const restarted = f.runtime(); await restarted.restore(); await restarted.settled();
  assert.equal(restarted.attempts.getSnapshot()[0].draft.text, 'kept');
  assert.equal(f.events.includes('post'), false);
  f.transport.timeline = async () => assert.fail('wrong member must not check status');
  await restarted.checkStatus('action/bound');
});

test('simultaneous owners consume only one SQL dispatch permit', async t => {
  const f = await fixture(t), initial = createOutboxRecord('action/once', { text: 'once', attachments: [], ...metadata({ ...messages[0] }) });
  await f.repository.commit(initial);
  const packet = packetFor(initial.id, initial.intent), ids = await expectedPreparedPacketIdentities(packet, 'person/avery', hash);
  await f.repository.compareAndSwap(0, prepareOutboxRecord(initial, packet, ids));
  const first = f.runtime(), second = f.runtime();
  await Promise.all([first.restore(), second.restore()]); await Promise.all([first.settled(), second.settled()]);
  assert.equal(f.events.filter(event => event === 'post').length, 1);
});

test('storage refusal never reports accepted or starts preparation/network; drafts stay untouched', async t => {
  const f = await fixture(t), runtime = f.runtime();
  const draft = { text: 'kept', images: [] }, original = structuredClone(draft);
  f.owner.acceptIntent = async () => { throw new Error('disk full'); };
  await assert.rejects(runtime.accept('action/refused', draft.text, draft.images, metadata({ ...messages[0] })), /disk full/);
  assert.deepEqual(draft, original); assert.deepEqual(f.events, []);
});

test('post refusal is observed once without retry; unknown has no generic retry closure', async t => {
  const f = await fixture(t), runtime = f.runtime();
  f.transport.post = async () => { f.events.push('post'); throw new ClientError({ api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: 'action/refused',
    code: 'stale-fence', message: 'not applied', details: { applied: 'none' }, retryable: true }, 409); };
  await runtime.accept('action/refused', 'kept', [], metadata({ ...messages[0] })); await runtime.settled();
  assert.equal((await f.repository.read('action/refused')).state, 'not-applied');
  await runtime.restore(); await runtime.settled();
  assert.equal(f.events.filter(event => event === 'post').length, 1);
  assert.equal(runtime.attempts.getSnapshot()[0].retry, undefined);
});
