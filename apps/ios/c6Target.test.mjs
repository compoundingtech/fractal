import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';
import { conversationEntries, sessionPerson } from '@fractal/views';
import { C6Recorder } from './c6Recorder.ts';

// Exercise the actual C6 selection and run callback without loading native UI,
// following conversationResend.test.mjs. Echo decoding and recording are real.
const source = readFileSync(new URL('./screens/Fleet.tsx', import.meta.url), 'utf8');
const start = source.indexOf('function C6Section() {');
const end = source.indexOf('  return <>', start);
assert(start >= 0 && end > start);
const component = stripTypeScriptTypes(`${source.slice(start, end)}  return run;\n}`);
const fixture = (explicitId, selected = 'agent/listed') => {
  let id = 0, now = 0, handler;
  const sent = [], followed = [];
  const feed = { followConversation: (target, handlers) => {
    followed.push(target); handler = handlers;
    return { close: () => { handler = undefined; } };
  } };
  const identity = {};
  const sample = { identity, context: { route: 'tailscale', path: 'unknown' }, feed, actor: 'person/test/session/phone', send: async (target, text, session, _tags, _images, hooks) => {
    sent.push({ target, session });
    hooks.request(`action/${sent.length}`); now++; hooks.ack();
    const base = { timestamp: '2026-11-09T00:00:00Z', revision: 1, role: 'user', final: true };
    handler.onEntries({ items: [
      { ...base, id: 'header', sequence: 1, type: 'message', body: { message_id: `message/${sent.length}`, from: 'person/test', to: target } },
      { ...base, id: 'content', sequence: 2, type: 'content', body: { media_type: 'text/plain', text } },
    ] });
    return { how: 'sent' };
  } };
  const c6 = new C6Recorder({ now: () => now, id: () => `${++id}`, snapshot: () => sample, readFabricPath: async () => assert.fail('not Fabric'), version: 'test', changed: () => {} });
  const state = [selected, explicitId, ''];
  const run = new Function('useStore', 'useState', 'useFocusEffect', 'useCallback', 'conversationEntries', 'sessionPerson', `${component}; return C6Section();`)(
    () => ({ c6, client: identity, feed, data: { agents: [{ id: 'agent/listed', current_session_id: 'session/listed' }] }, status: 'online' }),
    () => [state.shift(), () => {}], () => {}, callback => callback, conversationEntries, sessionPerson,
  );
  return { run, c6, sent, followed };
};

test('explicit stopped agent ID overrides listed selection, omits session, and correlates decoded echoes', async () => {
  const target = 'agent/probe/st-delegation/alpha';
  const { run, c6, sent, followed } = fixture(target);
  await run('send-echo');
  assert.equal(sent.length, 10);
  assert(sent.every(call => call.target === target && call.session === undefined));
  assert(followed.every(value => value === target));
  assert.equal(c6.rows.length, 10);
  assert(c6.rows.every(row => row.status === 'ok' && row.ackMs === 1 && row.echoMs === 1 && row.messageId?.startsWith('message/')));
});

test('invalid explicit IDs refuse sends even with a valid listed selection', async () => {
  for (const target of ['session/not-agent', 'agent/', 'agent/a//b', 'agent/a b', 'agent/../b']) {
    const { run, c6, sent, followed } = fixture(target);
    await run('send-echo');
    assert.equal(sent.length, 0); assert.equal(followed.length, 0); assert.equal(c6.rows.length, 0);
  }
});

test('empty explicit ID preserves listed-agent send and current session', async () => {
  const { run, c6, sent, followed } = fixture('');
  await run('send-echo');
  assert.equal(sent.length, 10);
  assert(sent.every(call => call.target === 'agent/listed' && call.session === 'session/listed'));
  assert(followed.every(value => value === 'agent/listed'));
  assert(c6.rows.every(row => row.status === 'ok'));
});
