import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/Client.generated.ts';
import { ADVANCE_EVERY_MS, READ_POSITIONS_KEY, ReadPositions, ROSTER_SKEW_MS, UNREAD_DOT, UNREAD_LABEL, UNSENT_READ_POSITIONS_KEY, countsAsNew, readPositionRemote, seenTail, syncsReadPositions, unreadMark } from './readPosition.ts';

const at = iso => Date.parse(iso);
const entry = (type, role, body = {}, timestamp = '2026-11-08T10:00:00Z') => ({ id: `${type}-${role}-${timestamp}`, type, role, body, timestamp, sequence: 0, revision: 0, final: true });
const iso = ms => new Date(ms).toISOString();
const settle = () => new Promise(resolve => setImmediate(resolve));
const flush = async () => { for (let i = 0; i < 5; i++) await settle(); };
function memory() {
  const values = new Map();
  return { values, storage: { async getItem(key) { return values.get(key) ?? null; }, async setItem(key, value) { values.set(key, value); } } };
}
/** A daemon whose list answers `listed` and whose advances the test settles one by one. */
function daemon(listed = []) {
  const sent = [];
  let lists = 0;
  const port = {
    async list() { lists++; return listed.map(item => ({ ...item })); },
    advance(conversation, at) {
      return new Promise((resolve, reject) => sent.push({ conversation, at, resolve, reject }));
    },
  };
  return { port, sent, listed, get lists() { return lists; } };
}
const unsent = values => JSON.parse(values.get(UNSENT_READ_POSITIONS_KEY) ?? '{}');

test('only the other side\'s prose counts as new', () => {
  assert.equal(countsAsNew(entry('content', 'assistant', { media_type: 'text/markdown', text: 'hi' })), true);
  assert.equal(countsAsNew(entry('content', 'user', { media_type: 'text/markdown', text: 'hi' })), false);
  assert.equal(countsAsNew(entry('tool_call', 'assistant')), false);
  assert.equal(countsAsNew(entry('status', 'system')), false);
  assert.equal(countsAsNew(entry('message', 'assistant', { message_id: 'm', from: 'agent/a' })), true);
  assert.equal(countsAsNew(entry('message', 'user', { message_id: 'm', from: 'person/ada' })), false);
  assert.equal(countsAsNew(entry('message', 'system', { message_id: 'm', author: 'person/ada' })), false);
});

test('the seen tail is the newest presented prose, not a later prompt', () => {
  const entries = [entry('content', 'assistant', {}, '2026-11-08T10:00:00Z'), entry('content', 'user', {}, '2026-11-08T11:00:00Z')];
  assert.equal(seenTail(entries, new Set(entries.map(entry => entry.id))), at('2026-11-08T10:00:00Z'));
  assert.equal(seenTail(entries, new Set()), undefined);
});

test('a roster update before its transcript cannot advance the presented frontier', async () => {
  const { storage } = memory();
  const positions = new ReadPositions(storage);
  await positions.load('https://gateway.example', 'person/example');
  const old = entry('content', 'assistant', {}, '2026-11-08T10:00:00Z');
  const newer = entry('content', 'assistant', {}, '2026-11-08T11:00:00Z');
  const present = entries => {
    const tail = seenTail(entries, new Set([old.id]));
    if (tail !== undefined) positions.advance('agent/example', tail);
  };
  present([]);
  assert.equal(positions.read('agent/example'), undefined, 'a roster timestamp alone establishes no frontier');
  present([old]);
  assert.equal(unreadMark(positions.read('agent/example'), newer.timestamp), UNREAD_DOT);
  present([old, newer]);
  assert.equal(positions.read('agent/example'), at(old.timestamp), 'loaded but not presented prose stays unread');
  positions.advance('agent/example', seenTail([old, newer], new Set([newer.id])));
  assert.equal(positions.read('agent/example'), at(newer.timestamp));
});

test('a failed conversation load with no presented prose cannot advance synced reads', async () => {
  const { storage } = memory();
  const positions = new ReadPositions(storage);
  await positions.load('https://gateway.example', 'person/example');
  const remote = daemon();
  positions.connect('https://gateway.example', 'person/example', remote.port);
  await flush();
  const tail = seenTail([], new Set());
  if (tail !== undefined) positions.advance('agent/example', tail);
  await flush();
  assert.equal(positions.read('agent/example'), undefined);
  assert.equal(remote.sent.length, 0);
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
  assert.deepEqual([...values.keys()].sort(), [READ_POSITIONS_KEY, UNSENT_READ_POSITIONS_KEY].sort());
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

test('positions sync only with a daemon that grants read-positions', async () => {
  assert.equal(syncsReadPositions(undefined), false);
  assert.equal(syncsReadPositions([{ id: 'arrangements', version: 2, state: 'granted' }]), false);
  assert.equal(syncsReadPositions([{ id: 'read-positions', version: 1, state: 'denied' }]), false);
  assert.equal(syncsReadPositions([{ id: 'read-positions', version: 1, state: 'granted' }]), true);
  // Never connected (an older daemon): positions behave as before, kept on this device alone.
  const { storage, values } = memory();
  const positions = new ReadPositions(storage);
  await positions.load('https://gw', 'person/ada');
  positions.advance('agent/a', 100);
  await positions.relist();
  await positions.flush();
  assert.equal(positions.read('agent/a'), 100);
  assert.deepEqual(JSON.parse(values.get(READ_POSITIONS_KEY)), { 'https://gw|person/ada': [['agent/a', 100]] });
});

test('a listed position merges by max: the daemon wins only when it is larger', async () => {
  const positions = new ReadPositions(memory().storage);
  await positions.load('https://gw', 'person/ada');
  positions.advance('agent/a', 200);
  positions.advance('agent/b', 200);
  const server = daemon([{ conversation: 'agent/a', at: iso(300) }, { conversation: 'agent/b', at: iso(100) }, { conversation: 'agent/c', at: iso(50) }, { conversation: 'agent/d', at: 'not a time' }]);
  let changes = 0;
  positions.subscribe(() => changes++);
  const stop = positions.connect('https://gw', 'person/ada', server.port);
  await flush();
  assert.equal(server.lists, 1, 'connecting lists the person\'s positions');
  assert.equal(positions.read('agent/a'), 300);
  assert.equal(positions.read('agent/b'), 200, 'a smaller daemon value never moves a position back');
  assert.equal(positions.read('agent/c'), 50);
  assert.equal(positions.read('agent/d'), undefined);
  assert.ok(changes >= 1);
  assert.deepEqual(server.sent.map(send => [send.conversation, send.at]), [['agent/b', iso(200)]], 'only what is ahead of the daemon is sent');
  // A frame re-lists; a larger value from another device arrives.
  server.listed.push({ conversation: 'agent/b', at: iso(900) });
  await positions.relist();
  assert.equal(positions.read('agent/b'), 900);
  stop();
});

test('advances coalesce per conversation: one in flight, one per interval, only beyond the confirmed value', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_000_000 });
  const positions = new ReadPositions(memory().storage);
  await positions.load('https://gw', 'person/ada');
  const server = daemon();
  const stop = positions.connect('https://gw', 'person/ada', server.port);
  await flush();
  positions.advance('agent/a', 100);
  await flush();
  assert.equal(server.sent.length, 1);
  positions.advance('agent/a', 200);
  positions.advance('agent/a', 300);
  await flush();
  assert.equal(server.sent.length, 1, 'one in flight per conversation');
  server.sent[0].resolve(iso(100));
  await flush();
  assert.equal(server.sent.length, 1, 'the next waits out the interval');
  t.mock.timers.tick(ADVANCE_EVERY_MS);
  await flush();
  assert.deepEqual(server.sent.map(send => send.at), [iso(100), iso(300)], 'and carries only the newest position');
  server.sent[1].resolve(iso(300));
  await flush();
  positions.advance('agent/a', 250);
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(server.sent.length, 2, 'nothing is sent at or below the confirmed value');
  // The stored value after the merge may be larger (another device): the position takes it.
  positions.advance('agent/b', 10);
  await flush();
  server.sent[2].resolve(iso(999));
  await flush();
  assert.equal(positions.read('agent/b'), 999);
  // A daemon without `read_position_at` confirms the value sent.
  positions.advance('agent/c', 10);
  await flush();
  server.sent[3].resolve(undefined);
  await flush();
  positions.advance('agent/c', 10);
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(server.sent.length, 4);
  stop();
});

test('a failed advance is tried again after a growing wait', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_000_000 });
  const positions = new ReadPositions(memory().storage, { retryDelaysMs: [5_000, 10_000] });
  await positions.load('https://gw', 'person/ada');
  const server = daemon();
  const stop = positions.connect('https://gw', 'person/ada', server.port);
  await flush();
  positions.advance('agent/a', 100);
  await flush();
  server.sent[0].reject(new Error('offline'));
  await flush();
  t.mock.timers.tick(4_999);
  await flush();
  assert.equal(server.sent.length, 1);
  t.mock.timers.tick(1);
  await flush();
  assert.equal(server.sent.length, 2);
  server.sent[1].reject(new Error('offline'));
  await flush();
  t.mock.timers.tick(9_999);
  await flush();
  assert.equal(server.sent.length, 2);
  t.mock.timers.tick(1);
  await flush();
  assert.equal(server.sent.length, 3);
  assert.equal(server.sent[2].at, iso(100));
  stop();
  server.sent[2].reject(new Error('offline'));
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(server.sent.length, 3, 'nothing is sent once disconnected');
});

test('advances made offline are kept and replayed when the daemon is reachable', async () => {
  const { storage, values } = memory();
  const first = new ReadPositions(storage);
  await first.load('https://gw', 'person/ada');
  first.advance('agent/a', 500);
  first.advance('agent/b', 40);
  await first.flush();
  assert.deepEqual(unsent(values), { 'https://gw|person/ada': ['agent/a', 'agent/b'] });
  const again = new ReadPositions(storage);
  await again.load('https://gw', 'person/ada');
  const server = daemon([{ conversation: 'agent/a', at: iso(400) }, { conversation: 'agent/b', at: iso(40) }]);
  again.connect('https://gw', 'person/ada', server.port);
  await flush();
  assert.deepEqual(server.sent.map(send => [send.conversation, send.at]), [['agent/a', iso(500)]], 'what the daemon already holds is not sent again');
  server.sent[0].resolve(iso(500));
  await flush();
  await again.flush();
  assert.deepEqual(unsent(values), {});
  assert.equal(again.read('agent/a'), 500);
});

test('a daemon that cannot list still receives the unsent advances', async () => {
  const positions = new ReadPositions(memory().storage);
  await positions.load('https://gw', 'person/ada');
  positions.advance('agent/a', 500);
  const server = daemon();
  server.port.list = async () => { throw new Error('offline'); };
  positions.connect('https://gw', 'person/ada', server.port);
  await flush();
  assert.deepEqual(server.sent.map(send => send.conversation), ['agent/a']);
});

test('a failed invalidation list exposes its issue and retries unreachable peers with a capped wait', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_000_000 });
  const positions = new ReadPositions(memory().storage, { retryDelaysMs: [1_000, 2_000] });
  await positions.load('https://gw', 'person/ada');
  const unavailable = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/example', code: 'http-error', message: 'HTTP 503', retryable: false, details: {} }, 503);
  let calls = 0;
  const stop = positions.connect('https://gw', 'person/ada', {
    async list() {
      calls++;
      if (calls >= 2 && calls <= 4) throw unavailable;
      return [{ conversation: 'agent/a', at: iso(calls === 1 ? 10 : 50) }];
    },
    async advance() { assert.fail('no local advances'); },
  });
  t.after(stop);
  await flush();
  const revision = positions.revision;
  await positions.relist(); // Invalidation while the socket remains live.
  assert.equal(positions.listIssue, 'HTTP 503');
  assert.ok(positions.revision > revision, 'the error notifies state subscribers');
  assert.equal(positions.read('agent/a'), 10, 'failure preserves the known frontier');
  for (const delay of [1_000, 2_000, 2_000]) {
    const before = calls;
    await positions.relist();
    assert.equal(calls, before, 'another invalidation cannot bypass the network backoff');
    t.mock.timers.tick(delay - 1);
    await flush();
    assert.equal(calls, before, 'does not hammer an unreachable peer');
    t.mock.timers.tick(1);
    await flush();
    assert.equal(calls, before + 1);
  }
  assert.equal(positions.read('agent/a'), 50, 'recovers without another invalidation frame');
  assert.equal(positions.listIssue, undefined, 'success clears the observable failure');
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(calls, 5, 'successful lists are not polled');
});

test('list refusals and local defects stay visible without starting retries', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  for (const failure of [
    new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/example', code: 'forbidden', message: 'not allowed', retryable: false, details: {} }, 403),
    new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/example', code: 'internal', message: 'local defect', retryable: true, details: {} }, 500),
    new TypeError('local defect'),
  ]) {
    const positions = new ReadPositions(memory().storage);
    await positions.load('https://gw', 'person/ada');
    let calls = 0;
    const stop = positions.connect('https://gw', 'person/ada', {
      async list() { calls++; throw failure; },
      async advance() { assert.fail('no local advances'); },
    });
    await flush();
    assert.equal(positions.listIssue, failure.message);
    t.mock.timers.tick(60_000);
    await flush();
    assert.equal(calls, 1);
    stop();
    assert.equal(positions.listIssue, undefined);
  }
});

test('list retry timers and late failures cannot outlive their connection', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const positions = new ReadPositions(memory().storage, { retryDelaysMs: [1_000] });
  await positions.load('https://gw', 'person/ada');
  let calls = 0;
  const stop = positions.connect('https://gw', 'person/ada', {
    async list() { calls++; throw new TypeError('Network request failed'); },
    async advance() { assert.fail('no local advances'); },
  });
  await flush();
  assert.equal(positions.listIssue, 'Network request failed');
  stop();
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(calls, 1, 'disconnect cancels the scheduled retry');
  let rejectOld;
  positions.connect('https://gw', 'person/ada', {
    list: () => new Promise((resolve, reject) => { rejectOld = reject; }),
    async advance() { assert.fail('no local advances'); },
  });
  await flush();
  const current = daemon([{ conversation: 'agent/a', at: iso(50) }]);
  const stopCurrent = positions.connect('https://gw', 'person/ada', current.port);
  t.after(stopCurrent);
  await flush();
  rejectOld(new TypeError('Network request failed'));
  await flush();
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(current.lists, 1);
  assert.equal(positions.listIssue, undefined, 'the old failure cannot stain the new connection');
  assert.equal(positions.read('agent/a'), 50);
});

test('a connection belongs to one gateway and person', async () => {
  const positions = new ReadPositions(memory().storage);
  await positions.load('https://gw', 'person/ada');
  const server = daemon([{ conversation: 'agent/a', at: iso(70) }]);
  const stop = positions.connect('https://gw', 'person/ada', server.port);
  await flush();
  await positions.load('https://gw', 'person/avery');
  positions.advance('agent/b', 10);
  await positions.relist();
  await flush();
  assert.equal(server.sent.length, 0, 'another person\'s advance never goes over this connection');
  assert.equal(positions.read('agent/a'), undefined);
  await positions.load('https://gw', 'person/ada');
  assert.equal(positions.read('agent/a'), 70);
  stop();
});

test('the remote reads every page of the person\'s positions and advances for that person', async () => {
  const position = (conversation, at, owner = 'person/ada') => ({ kind: 'read-position', id: `read-position/${owner}/${conversation}`, revision: 'claim/1', owner, conversation, at });
  const pages = new Map([
    [undefined, { items: [position('agent/a', iso(1)), position('agent/x', iso(1), 'person/other'), { kind: 'arrangement' }], page: { has_more: true, next_cursor: 'c1' } }],
    ['c1', { items: [position('session/s', iso(2))], page: { has_more: false } }],
  ]);
  const calls = [];
  const remote = readPositionRemote('person/ada', {
    list: async cursor => { calls.push(cursor); return pages.get(cursor); },
    advance: async parameters => { calls.push(parameters); return { read_position_at: iso(9) }; },
  });
  assert.deepEqual(await remote.list(), [{ conversation: 'agent/a', at: iso(1) }, { conversation: 'session/s', at: iso(2) }]);
  assert.deepEqual(calls, [undefined, 'c1']);
  assert.equal(await remote.advance('agent/a', iso(5)), iso(9));
  assert.deepEqual(calls.at(-1), { owner: 'person/ada', conversation: 'agent/a', at: iso(5) });
  // A cursor that does not move ends the read instead of looping.
  const stuck = readPositionRemote('person/ada', { list: async () => ({ items: [], page: { has_more: true, next_cursor: 'same' } }), advance: async () => ({}) });
  assert.deepEqual(await stuck.list(), []);
  assert.equal(await stuck.advance('agent/a', iso(5)), undefined);
});
