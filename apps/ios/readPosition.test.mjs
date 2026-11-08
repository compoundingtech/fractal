import assert from 'node:assert/strict';
import { test } from 'node:test';
import { READ_POSITIONS_KEY, ReadPositions, ROSTER_SKEW_MS, UNREAD_DOT, UNREAD_LABEL, countsAsNew, seenTail, unreadMark } from './readPosition.ts';

const at = iso => Date.parse(iso);
const entry = (type, role, body = {}, timestamp = '2026-11-08T10:00:00Z') => ({ id: `${type}-${role}-${timestamp}`, type, role, body, timestamp, sequence: 0, revision: 0, final: true });

test('only the other side\'s prose counts as new', () => {
  assert.equal(countsAsNew(entry('content', 'assistant', { media_type: 'text/markdown', text: 'hi' })), true);
  assert.equal(countsAsNew(entry('content', 'user', { media_type: 'text/markdown', text: 'hi' })), false);
  assert.equal(countsAsNew(entry('tool_call', 'assistant')), false);
  assert.equal(countsAsNew(entry('status', 'system')), false);
  assert.equal(countsAsNew(entry('message', 'assistant', { message_id: 'm', from: 'agent/a' })), true);
  assert.equal(countsAsNew(entry('message', 'user', { message_id: 'm', from: 'person/ada' })), false);
  assert.equal(countsAsNew(entry('message', 'system', { message_id: 'm', author: 'person/ada' })), false);
});

test('the seen tail is the newest of the roster prose and the loaded prose', () => {
  const entries = [entry('content', 'assistant', {}, '2026-11-08T10:00:00Z'), entry('content', 'user', {}, '2026-11-08T11:00:00Z')];
  assert.equal(seenTail(undefined, entries), at('2026-11-08T10:00:00Z'));
  assert.equal(seenTail('2026-11-08T10:30:00Z', entries), at('2026-11-08T10:30:00Z'));
  assert.equal(seenTail('2026-11-08T09:00:00Z', entries), at('2026-11-08T10:00:00Z'));
  assert.equal(seenTail(null, []), undefined);
  assert.equal(seenTail('not a time', []), undefined);
});

test('a mark needs a known position and roster prose beyond the skew', () => {
  const seen = at('2026-11-08T10:00:00Z');
  assert.equal(ROSTER_SKEW_MS, 2000);
  assert.equal(UNREAD_DOT, '•');
  assert.equal(UNREAD_LABEL, 'New messages');
  assert.equal(unreadMark(undefined, '2026-11-08T12:00:00Z'), undefined);
  assert.equal(unreadMark(seen, undefined), undefined);
  assert.equal(unreadMark(seen, null), undefined);
  assert.equal(unreadMark(seen, '2026-11-08T10:00:02Z'), undefined);
  assert.equal(unreadMark(seen, '2026-11-08T10:00:02.001Z'), UNREAD_DOT);
  assert.equal(unreadMark(seen, '2026-11-08T09:00:00Z'), undefined);
});

test('positions only move forward, are kept per gateway and person, and persist', async () => {
  const values = new Map(), storage = { async getItem(key) { return values.get(key) ?? null; }, async setItem(key, value) { values.set(key, value); } };
  const positions = new ReadPositions(storage);
  await positions.load('https://gw', 'person/ada');
  let changes = 0; const stop = positions.subscribe(() => changes++);
  assert.equal(positions.read('agent/a'), undefined);
  positions.advance('agent/a', 200);
  positions.advance('agent/a', 100);
  assert.equal(positions.read('agent/a'), 200);
  assert.equal(changes, 1);
  stop();
  await positions.flush();
  assert.deepEqual([...values.keys()], [READ_POSITIONS_KEY]);
  const again = new ReadPositions(storage);
  await again.load('https://gw', 'person/ada');
  assert.equal(again.read('agent/a'), 200);
  await again.load('https://gw', 'person/avery');
  assert.equal(again.read('agent/a'), undefined);
  again.advance('agent/a', 50);
  await again.flush();
  await again.load('https://gw', 'person/ada');
  assert.equal(again.read('agent/a'), 200);
  await again.load('https://other', 'person/ada');
  assert.equal(again.read('agent/a'), undefined);
});

test('nothing is kept before an owner is known; storage failures are ignored; storage is bounded', async () => {
  const positions = new ReadPositions({ async getItem() { throw new Error('x'); }, async setItem() { throw new Error('x'); } });
  positions.advance('agent/a', 10);
  assert.equal(positions.read('agent/a'), undefined);
  await positions.load('https://gw', 'person/ada');
  for (let i = 0; i < 600; i++) positions.advance(`agent/${i}`, i + 1);
  await positions.flush();
  assert.equal(positions.read('agent/0'), undefined);
  assert.equal(positions.read('agent/599'), 600);
});
