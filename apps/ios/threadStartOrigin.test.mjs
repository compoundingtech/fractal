import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

test('native start signs the canonical Inbox source and sends the exact generic origin', async () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const a = source.indexOf('    startThread(thread: string,');
  const b = source.indexOf('    replyToThread(', a);
  const { outputText } = ts.transpileModule(`exports.action = { ${source.slice(a, b)} }.startThread;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const signed = [], sent = [], presentations = [];
  const exports = {};
  const scope = origin => JSON.stringify([origin.sourceId, origin.episode]);
  const ports = {
    signingKeyRef: { current: 'profile/exact' },
    threadStartScope: scope,
    trackThreadDelivery: (_scope, _target, send, _onSent, presentation) => { presentations.push(presentation); return send(); },
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
  assert.deepEqual(signed, [{
    content: 'Follow up', from: 'person/avery', to: origin.agentId, thread_id: 'thread/new', title: 'Notice',
    thread_origin_source: origin.sourceId, thread_origin_episode: origin.episode,
  }]);
  assert.deepEqual(sent[0].parameters, {
    target_id: 'thread/new', agent_id: origin.agentId, origin: { source_id: origin.sourceId, episode: origin.episode },
    title: 'Notice', content: 'Follow up', signature: 'exact-signature',
  });
  assert.equal(scope(origin), scope({ ...origin, agentId: 'agent/reassigned' }), 'draft identity stays with source and episode');
});
