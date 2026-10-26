import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
const snapshot = { id: 'snapshot/test/1', host_id: 'host/example', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-01-01T12:00:00Z' };

test('feed receipt clock precedes view maintenance, and quality refusals carry only a typed code', async t => {
  let now = 10;
  t.mock.method(performance, 'now', () => now);
  const { client, sockets } = fakeClient();
  const received = [], failures = [];
  const feed = new Feed(client, { onWindow: (_name, _items, _more, _at, clock) => received.push(clock), onConnection() {} }, new ForegroundGate('active'), () => 'action/test');
  t.after(() => feed.close()); await settle();
  const agent = { kind: 'agent', id: 'agent/example', revision: 'claim/1', name: 'Example', runtime_ids: [] };
  sockets[0].frame({ kind: 'snapshot', id: 'agents', snapshot, get items() { now = 30; return [agent]; }, order: [agent.id], has_more: false });
  assert.deepEqual(received, [10]);
  const follow = feed.followConversation(agent.id, { onEntries() {}, onIssue() {}, onQualityError: code => failures.push(code) });
  sockets[0].frame({ kind: 'resync', id: 'conversation', code: 'remote-unavailable', message: 'private refusal detail', retryable: true });
  sockets[0].frame({ kind: 'error', id: 'conversation', code: 'timeline-history-incomplete', message: 'private transcript detail' });
  assert.deepEqual(failures, ['remote-unavailable', 'timeline-history-incomplete']);
  assert.equal(JSON.stringify(failures).includes('private'), false);
  follow.close();
});
function terminalClient() {
  const { client, sockets } = fakeClient();
  client.terminalScreen = async () => ({ snapshot, value: { runtime_incarnation: 'inc-test', next_sequence: 1 } });
  client.terminalAttach = async () => ({ value: { terminal_attachment: { stream_capability: 'stream-test', runtime_incarnation: 'inc-test' } } });
  client.terminalDetach = async () => ({});
  return { client, sockets };
}
test('terminal quality counts unexpected stream loss once, not background suspension or intentional close', async t => {
  const { client, sockets } = terminalClient(), gate = new ForegroundGate('active'), events = [];
  const feed = new Feed(client, { onWindow() {}, onConnection() {} }, gate, () => 'action/test', [10000]);
  t.after(() => feed.close()); await settle();
  const follow = feed.followTerminal('terminal/example', { onScreen() {}, onIssue() {}, onQuality: event => events.push(event) });
  await settle(); assert.deepEqual(events, ['attempt', 'attached']);
  gate.update('background'); assert.equal(events.filter(event => event === 'dead').length, 0);
  gate.update('active'); await settle();
  assert.deepEqual(events, ['attempt', 'attached', 'attempt', 'attached']);
  sockets.at(-1).drop(new Error('disconnected'));
  assert.equal(events.filter(event => event === 'dead').length, 1);
  follow.close(); assert.equal(events.filter(event => event === 'dead').length, 1);
});
test('a refused or incomplete terminal attach records an error without an attached or dead-tab success', async t => {
  const { client } = terminalClient(), events = [];
  client.terminalAttach = async () => { throw { response: { code: 'forbidden' } }; };
  const feed = new Feed(client, { onWindow() {}, onConnection() {} }, new ForegroundGate('active'), () => 'action/test');
  t.after(() => feed.close()); await settle();
  feed.followTerminal('terminal/example', { onScreen() {}, onIssue() {}, onQuality: event => events.push(event) });
  await settle(); assert.deepEqual(events, ['attempt', 'error']);
  client.terminalAttach = async () => ({ value: {} });
  feed.followTerminal('terminal/example', { onScreen() {}, onIssue() {}, onQuality: event => events.push(event) });
  await settle(); assert.deepEqual(events, ['attempt', 'error', 'attempt', 'error']);
});
