import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { ConversationAttempts } from './conversationAttempts.ts';
const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
const action = (name, end, dependencies) => {
  const a = source.indexOf(`    async ${name}(`), b = source.indexOf(end, a);
  assert.ok(a >= 0 && b > a);
  const { outputText } = ts.transpileModule(`exports.value = {${source.slice(a, b)}}.${name};`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
  const exports = {};
  new Function('exports', ...Object.keys(dependencies), outputText)(exports, ...Object.values(dependencies));
  return exports.value;
};
const accepted = { how: 'accepted', outboxId: 'action/original' };
for (const kind of ['chat', 'start', 'reply']) test(`${kind}: production protected branch accepts once and never invokes legacy signing/upload/transport`, async () => {
  const calls = [], cache = new Map([['thread-draft/thread/original', 'submitted']]);
  const deps = { client: {}, uploader: {}, status: 'online', proof: undefined, errorText: error => error.message,
    acceptOutbox: async (...args) => { calls.push(args); return accepted; },
    draftsOwner: { current: 'owner/original' }, draftGeneration: { current: 0 }, draftCache: { current: cache }, threadDraftKey: id => 'thread-draft/' + id,
    localDrafts: { save: async () => {} } };
  let result;
  if (kind === 'chat') {
    const send = action('send', '    /** Retain unknown delivery', deps);
    let acknowledged = 0;
    result = await send('agent/dotfiles/manager', 'hello', 'session/original', ['dictated'], [], () => acknowledged++);
    assert.equal(acknowledged, 1);
    assert.deepEqual(calls[0][0], { kind: 'chat', to: 'agent/dotfiles/manager', sessionId: 'session/original', tags: ['dictated'] });
  } else if (kind === 'start') {
    result = await action('startThread', '    async replyToThread(', deps)('thread/original', { agentId: 'agent/dotfiles/manager', sourceId: 'notice/original', episode: 'episode/original' }, 'Title', 'hello');
    assert.equal(calls[0][0].origin.agentId, 'agent/dotfiles/manager');
  } else {
    result = await action('replyToThread', '    async saveUrl(', deps)({ id: 'thread/original', agent_id: 'agent/dotfiles/manager' }, 'hello', 'message/quote');
    assert.equal(calls[0][0].quoteId, 'message/quote');
    assert.equal(cache.has('thread-draft/thread/original'), false);
  }
  assert.deepEqual(result, accepted); assert.equal(calls.length, 1);
});

for (const kind of ['chat', 'start', 'reply']) test(`${kind}: protected storage refusal returns failure with no legacy fallback or draft clearing`, async () => {
  const cache = new Map([['thread-draft/thread/original', 'kept']]);
  const deps = { client: {}, uploader: {}, status: 'online', proof: undefined, errorText: error => error.message,
    acceptOutbox: async () => { throw new Error('storage unavailable, retry later'); },
    draftsOwner: { current: 'owner/original' }, draftGeneration: { current: 0 }, draftCache: { current: cache }, threadDraftKey: id => 'thread-draft/' + id };
  const result = kind === 'chat' ? await action('send', '    /** Retain unknown delivery', deps)('agent/dotfiles/manager', 'hello', undefined, undefined, [], () => assert.fail('must keep composer'))
    : kind === 'start' ? await action('startThread', '    async replyToThread(', deps)('thread/original', { agentId: 'agent/dotfiles/manager', sourceId: 'notice/original', episode: 'episode/original' }, 'Title', 'hello')
    : await action('replyToThread', '    async saveUrl(', deps)({ id: 'thread/original', agent_id: 'agent/dotfiles/manager' }, 'hello', 'message/quote');
  assert.deepEqual(result, { how: 'failed', error: 'storage unavailable, retry later' });
  assert.equal(cache.get('thread-draft/thread/original'), 'kept');
});

test('ordinary composer clears only at acceptance and never clears later edits/images', async () => {
  const screen = readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8');
  const a = screen.indexOf('  async function send(spoken?'), b = screen.indexOf('  function sendAgain(', a);
  const { outputText } = ts.transpileModule(`${screen.slice(a, b)}; exports.send = send;`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
  for (const edited of [false, true]) {
    const originalImages = [{ base64: 'original' }], laterImages = [{ base64: 'later' }];
    const cache = new Map([['agent/dotfiles/manager', 'draft']]);
    let clearCount = 0, currentImages = originalImages, acknowledge, resolve;
    const attempts = new ConversationAttempts(() => undefined, error => ({ how: 'failed', error: String(error) }));
    const deps = { draft: 'draft', images: originalImages, agent: { id: 'agent/dotfiles/manager' }, target: 'agent/dotfiles/manager', nextAttempt: { current: 0 }, nowClock: () => 'now',
      draftsOwner: { current: 'owner' }, draftGeneration: { current: 0 }, draftCache: { current: cache },
      emptyBox: () => { clearCount++; cache.set('agent/dotfiles/manager', ''); },
      setImages: update => { currentImages = update(currentImages); }, setImageIssue: () => {}, setSendIssue: () => {},
      list: { current: null }, offset: { current: 0 }, dictated: true, setDictated: () => {}, attempts,
      actions: { send: (...args) => { acknowledge = args[5]; return new Promise(done => { resolve = done; }); } }, settle: () => {} };
    const exports = {}; new Function('exports', ...Object.keys(deps), outputText)(exports, ...Object.values(deps));
    const sending = exports.send();
    assert.equal(clearCount, 0); assert.equal(currentImages, originalImages);
    if (edited) { cache.set('agent/dotfiles/manager', 'later typing'); currentImages = laterImages; }
    acknowledge(); resolve(accepted); await sending;
    assert.equal(clearCount, edited ? 0 : 1);
    assert.equal(cache.get('agent/dotfiles/manager'), edited ? 'later typing' : '');
    assert.equal(currentImages, edited ? laterImages : currentImages);
    if (!edited) assert.deepEqual(currentImages, []);
    assert.equal(attempts.getSnapshot().length, 0);
  }
});

test('production restoration/status paths retain separate authoritative paging and no replica/text-match authority', () => {
  assert.match(source, /AppState.addEventListener\('change'/);
  assert.match(source, /if \(state === 'active'\)/);
  assert.match(source, /original.threadTimeline\(message.target, \{ around: identity \}\)/);
  assert.match(source, /entry.body.message_id/);
  assert.match(source, /context.start_recipient.agent_id !== message.origin.agentId/);
  assert.match(source, /owner.repository.saveUploads\(id, attachments\)/);
});

test('accepted thread reply clears only its captured draft revision and never another owner or later quote/text', async () => {
  const screen = readFileSync(new URL('./screens/Threads.tsx', import.meta.url), 'utf8');
  const a = screen.indexOf('  const send = async (run:'), b = screen.indexOf('\n  return <View', a);
  const { outputText } = ts.transpileModule(`${screen.slice(a, b)}; exports.send = send;`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
  for (const change of ['none', 'later-edit', 'owner', 'generation']) {
    const key = 'thread-draft/thread/original', initial = JSON.stringify({ text: 'submitted', quote: { id: 'message/quote' } });
    const cache = new Map([[key, initial]]), owner = { current: 'original' }, generation = { current: 0 }, cleared = [];
    let resolve;
    const deps = { start: undefined, thread: { id: 'thread/original' }, draft: JSON.parse(initial), key,
      draftCache: { current: cache }, draftsOwner: owner, draftGeneration: generation,
      storeDraft: draft => cleared.push(draft), setFailure: () => {} };
    const exports = {}; new Function('exports', ...Object.keys(deps), outputText)(exports, ...Object.values(deps));
    const sending = exports.send(() => new Promise(done => { resolve = done; }));
    assert.deepEqual(cleared, []);
    if (change === 'none') cache.delete(key); // Store removed exactly the accepted original.
    if (change === 'later-edit') cache.set(key, JSON.stringify({ text: 'later', quote: { id: 'message/later' } }));
    if (change === 'owner') owner.current = 'other';
    if (change === 'generation') generation.current++;
    resolve(accepted); await sending;
    assert.deepEqual(cleared, change === 'none' ? [{ text: '' }] : []);
    if (change === 'later-edit') assert.match(cache.get(key), /message\/later/);
  }
});

test('store reply acceptance cannot delete or save an identical restored draft from a newer generation', async () => {
  const key = 'thread-draft/thread/original', saved = JSON.stringify({ text: 'submitted', quote: { id: 'message/quote' } });
  for (const restored of [false, true]) {
    const cache = new Map([[key, saved]]), owner = { current: 'person/avery' }, generation = { current: 0 }, saves = [];
    let resolve;
    const deps = { proof: undefined, errorText: error => error.message,
      draftsOwner: owner, draftGeneration: generation, draftCache: { current: cache }, threadDraftKey: id => 'thread-draft/' + id,
      localDrafts: { save: async (...args) => { saves.push(args); } },
      acceptOutbox: () => new Promise(done => { resolve = done; }) };
    const sending = action('replyToThread', '    async saveUrl(', deps)({ id: 'thread/original', agent_id: 'agent/dotfiles/manager' }, 'submitted', 'message/quote');
    assert.equal(cache.get(key), saved); assert.equal(saves.length, 0);
    if (restored) {
      generation.current++; cache.clear(); owner.current = 'person/avery'; cache.set(key, saved);
    }
    resolve(accepted);
    assert.deepEqual(await sending, accepted);
    assert.equal(cache.has(key), restored);
    assert.equal(saves.length, restored ? 0 : 1);
    if (restored) assert.equal(cache.get(key), saved);
  }
});

test('production retry preflight rereads canonical recipient and retirement on the original client', async () => {
  const a = source.indexOf('        validateRetry: '), b = source.indexOf('\n        sha256:', a);
  assert.ok(a > 0 && b > a);
  const expression = source.slice(a + '        validateRetry: '.length, b).trim().replace(/,$/, '');
  const { outputText } = ts.transpileModule(`exports.validate = ${expression};`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
  for (const kind of ['chat', 'start', 'reply']) for (const state of ['current', 'retired', 'reassigned', 'unassigned']) {
    const originalAgent = 'agent/dotfiles/manager', calls = [];
    class Refusal extends Error { constructor(code) { super(code); this.response = { code }; this.status = 409; } }
    const original = {
      threadStartContext: async () => {
        calls.push('canonical');
        if (state === 'unassigned') throw new Refusal('recipient-unassigned');
        return { value: { start_recipient: { agent_id: state === 'reassigned' ? 'person/ada' : originalAgent } } };
      },
      threadGet: async () => { calls.push('thread'); return { value: { agent_id: state === 'reassigned' ? 'person/ada' : originalAgent } }; },
      agentsGet: async id => {
        calls.push('agent');
        if (state === 'unassigned') throw new Refusal('recipient-unassigned');
        return { value: { kind: 'agent', id: state === 'reassigned' ? 'person/ada' : id, state: state === 'retired' ? 'retired' : 'running' } };
      },
    };
    const exports = {};
    new Function('exports', 'original', 'authorizeIntent', 'ClientError', outputText)(exports, original, () => {}, Refusal);
    const message = kind === 'chat' ? { kind, to: originalAgent } : kind === 'start'
      ? { kind, target: 'thread/original', origin: { agentId: originalAgent, sourceId: 'notice/original', episode: 'episode/original' } }
      : { kind, target: 'thread/original', agentId: originalAgent };
    assert.equal(await exports.validate({ message }), state === 'current' ? undefined : state);
    assert.ok(calls.length > 0);
  }
  assert.match(source, /const currentFence = intent.parentAttemptId\s*\? \{ snapshot_id: \(await original.capabilities\(\)\).snapshot.id/);
});
