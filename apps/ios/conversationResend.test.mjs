import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';
import { ConversationAttempts } from './conversationAttempts.ts';
import { restoreDraft } from './composer.ts';

const source = readFileSync(new URL('./screens/Conversation.tsx', import.meta.url), 'utf8');
const start = source.indexOf('  function sendAgain(item: Pending) {');
const end = source.indexOf('  // A sent message waits', start);
assert.ok(start >= 0 && end > start);
const confirmation = stripTypeScriptTypes(source.slice(start, end));
const invoke = (body, name, deps) => new Function(...Object.keys(deps), `${stripTypeScriptTypes(body)}; return ${name};`)(...Object.values(deps));

for (const changed of ['none', 'owner', 'generation']) test(`unknown copy restores text only without sending, respecting ${changed}`, async () => {
  const attempts = new ConversationAttempts(() => undefined, error => ({ how: 'failed', error: String(error) }));
  await attempts.send('pending/original', {}, async () => ({ how: 'unconfirmed' }));
  const original = attempts.getSnapshot()[0], target = 'agent/dotfiles/manager';
  const cache = new Map([[target, 'later typing']]), owner = { current: 'person/avery' }, generation = { current: 0 };
  let buttons, draft = '', issue = '', saves = 0, posts = 0;
  const deps = { Alert: { alert: (_title, _body, options) => { buttons = options; } }, attempts,
    draftsOwner: owner, draftGeneration: generation, draftCache: { current: cache }, target, restoreDraft,
    draftTouched: { current: false }, setDraft: value => { draft = value; }, saveDrafts: () => saves++,
    setSendIssue: value => { issue = value; }, box: { current: { focus() {} } } };
  const recover = invoke(confirmation, 'sendAgain', deps);
  const item = { id: original.id, pickedImages: [{ base64: 'image' }], unconfirmed: { member: 'member/original',
    request: { parameters: { to: target, content: 'hello', session_id: 'session/original' } } } };
  recover(item);
  assert.equal(buttons.some(button => button.text === 'Send as new'), false);
  assert.ok(buttons.some(button => button.text === 'Discard'));
  if (changed === 'owner') owner.current = 'person/ada';
  if (changed === 'generation') generation.current++;
  await buttons.find(button => button.text === 'Copy to composer').onPress();
  assert.equal(posts, 0);
  assert.equal(attempts.getSnapshot()[0], original);
  if (changed !== 'none') { assert.equal(saves, 0); assert.equal(cache.get(target), 'later typing'); return; }
  assert.equal(draft, restoreDraft('later typing', 'hello'));
  assert.equal(cache.get(target), draft); assert.equal(saves, 1);
  assert.equal(issue, 'Sending this copy may duplicate it');
  const a = source.indexOf('  async function send(spoken?'), b = source.indexOf('  function sendAgain(', a);
  const send = invoke(source.slice(a, b), 'send', { ...deps, draft, images: [], agent: { id: target }, nextAttempt: { current: 0 },
    nowClock: () => 'now', emptyBox: () => cache.delete(target), setImages: () => {}, setImageIssue: () => {},
    list: { current: null }, offset: { current: 0 }, dictated: false, setDictated: () => {}, settle: () => {},
    actions: { send: async (...args) => { posts++; assert.equal(args[1], draft); assert.deepEqual(args[4], []); args[5](); return { how: 'unconfirmed' }; } } });
  await send();
  assert.equal(posts, 1, 'only the separate normal composer send dispatches');
  assert.equal(attempts.getSnapshot().length, 2);
  assert.notEqual(attempts.getSnapshot()[1].id, original.id);
  assert.equal(attempts.getSnapshot()[0], original);
});

test('unknown recovery Discard removes local pending only and never sends', async () => {
  const attempts = new ConversationAttempts(() => undefined, error => ({ how: 'failed', error }));
  await attempts.send('pending/original', {}, async () => ({ how: 'unconfirmed' }));
  let buttons;
  const recover = invoke(confirmation, 'sendAgain', { attempts, draftsOwner: { current: 'person/avery' }, draftGeneration: { current: 0 },
    Alert: { alert: (_title, _body, options) => { buttons = options; } } });
  recover({ id: 'pending/original', unconfirmed: { member: 'member/original' } });
  buttons.find(button => button.text === 'Discard').onPress();
  assert.deepEqual(attempts.getSnapshot(), []);
});
