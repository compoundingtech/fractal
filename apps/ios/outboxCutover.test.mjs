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
    draftsOwner: { current: 'owner/original' }, draftCache: { current: cache }, threadDraftKey: id => 'thread-draft/' + id,
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
    draftsOwner: { current: 'owner/original' }, draftCache: { current: cache }, threadDraftKey: id => 'thread-draft/' + id };
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
