import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { appliedAnswer, ClientError } from '../../clients/typescript/fractal-client/index.ts';

test('native start signs the canonical Inbox source and sends the exact generic origin', async () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const a = source.indexOf('    startThread(thread: string,');
  const b = source.indexOf('    replyToThread(', a);
  const { outputText } = ts.transpileModule(`exports.action = { ${source.slice(a, b)} }.startThread;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const signed = [], sent = [], presentations = [], scopes = [];
  const exports = {};
  const scope = origin => JSON.stringify([origin.sourceId, origin.episode]);
  const attemptScope = (origin, thread) => JSON.stringify([origin.sourceId, origin.episode, thread]);
  const ports = {
    signingKeyRef: { current: 'profile/exact' },
    threadStartAttemptScope: attemptScope,
    trackThreadDelivery: (deliveryScope, _target, send, _onSent, presentation) => { scopes.push(deliveryScope); presentations.push(presentation); return send(); },
    readPhoneSigning: async () => ({ person: 'person/avery' }),
    signThreadMessage: async (_key, _signing, fields) => { signed.push(fields); return { signature: 'exact-signature', messageId: 'message/first' }; },
    submitThreadAction: async (_target, _kind, _revisions, prepare) => {
      const prepared = await prepare({ threadStart: input => { sent.push(input); return { how: 'sent' }; } }, 'action/key');
      return prepared.send({ id: 'action/key', idempotency_key: 'action/key', fence: { snapshot_id: 'snapshot/one' } });
    },
  };
  new Function('exports', ...Object.keys(ports), outputText)(exports, ...Object.values(ports));
  const origin = { agentId: 'agent/canonical', sourceId: 'notice/canonical-source', episode: 'episode/original' };
  await exports.action('thread/new', origin, 'Notice', 'Follow up');
  assert.deepEqual(presentations, [{ origin, title: 'Notice' }]);
  await exports.action('thread/another', origin, 'Notice', 'Another question');
  assert.deepEqual(scopes, [attemptScope(origin, 'thread/new'), attemptScope(origin, 'thread/another')],
    'each Start of one source episode is delivered and retained as its own attempt');
  assert.deepEqual(signed[0], {
    content: 'Follow up', from: 'person/avery', to: origin.agentId, thread_id: 'thread/new', title: 'Notice',
    thread_origin_source: origin.sourceId, thread_origin_episode: origin.episode,
  });
  assert.equal(signed[1].thread_id, 'thread/another');
  assert.deepEqual(sent[0].parameters, {
    target_id: 'thread/new', agent_id: origin.agentId, origin: { source_id: origin.sourceId, episode: origin.episode },
    title: 'Notice', content: 'Follow up', signature: 'exact-signature',
  });
  assert.equal(sent[1].parameters.target_id, 'thread/another');
  assert.equal(scope(origin), scope({ ...origin, agentId: 'agent/reassigned' }), 'draft identity stays with source and episode');
});

test('only definitive canonical Start recipient refusals request a visible context refresh; unknown outcomes keep the exact retry', async () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const tree = ts.createSourceFile('store.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let finish;
  const visit = node => {
    if (ts.isVariableDeclaration(node) && node.name.getText(tree) === 'finish' && node.initializer?.getText(tree).includes('refreshStartContext')) finish = node;
    ts.forEachChild(node, visit);
  };
  visit(tree);
  assert.ok(finish, 'test exercises the real store completion mapping');
  const { outputText } = ts.transpileModule(`exports.make = (kind) => { const ${finish.getText(tree)}; return finish; };`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const exports = {}, sent = [];
  new Function('exports', 'ClientError', 'acceptedSnapshot', 'threadActionErrorText', 'setThreadRefresh', 'resendBoundAction', 'appliedAnswer', outputText)(
    exports, ClientError, undefined, String, () => {}, async request => { sent.push(request); return { how: 'sent' }; }, appliedAnswer);
  const goldens = JSON.parse(readFileSync(new URL('../../docs/st3/client-v0/fixtures/thread-start-refusals.json', import.meta.url), 'utf8'));
  const start = exports.make('start');
  for (const [key, envelope] of Object.entries(goldens)) {
    const error = new ClientError(envelope, 409);
    assert.equal(start({ how: 'failed', error }).refreshStartContext, key === 'stale-snapshot' ? undefined : true);
    assert.equal(exports.make('reply')({ how: 'failed', error }).refreshStartContext, undefined);
  }
  const original = Object.freeze({ agent_id: 'agent/A', id: 'action/original', signature: 'signed-original' });
  const unknown = start({ how: 'unconfirmed', request: original });
  assert.equal(unknown.how, 'unconfirmed');
  assert.equal(unknown.refreshStartContext, undefined);
  assert.equal((await unknown.retry()).how, 'sent');
  assert.deepEqual(sent, [original], 'an unknown receipt never goes through a new recipient or action key');
});
