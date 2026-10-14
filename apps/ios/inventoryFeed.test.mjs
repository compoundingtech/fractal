import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Feed, HEARTBEAT_WAIT_MS } from './feed.ts';
import { InventoryFeed } from './inventoryFeed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';

const at = index => ({ id: `snapshot/host/${index}`, host_id: 'host/one', store_index: index, projection_version: 'client-projection.v0', created_at: '2026-10-12T00:00:00Z' });
const snapshot = (id, collection, items, index = 1, has_more = false) => ({ kind: 'snapshot', id, collection, items, order: items.map(row => row.id), snapshot: at(index), has_more });
const flush = async () => { for (let n = 0; n < 5; n++) await Promise.resolve(); };
const handlers = { onWindow() {}, onConnection() {} };
const row = (kind, id) => ({ kind, id, revision: `${id}@1` });

test('idle feed with focused inventories makes no request for longer than all old intervals', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient();
  let reads = 0, transport;
  client.capabilities = async () => { reads++; throw new Error('Unexpected capabilities probe'); };
  const states = [];
  const feed = new Feed(client, { ...handlers, onConnection: (state, issue) => states.push([state, issue]) }, new ForegroundGate('active'), () => 'action/test', [1000], undefined, false, undefined, () => (transport = { onping: null, close() {}, send() {} }));
  await flush();
  feed.setInventoryGranted(true);
  const seen = new Map();
  for (const [name, kind] of Object.entries({ clients: 'connected-client', usage: 'usage', machines: 'machine', devices: 'device', sessions: 'session', terminals: 'runtime' })) {
    feed.watchInventory(name, { onRows: rows => seen.set(name, rows), onIssue() {} }, name === 'usage' ? 24 : undefined);
    sockets[0].frame(snapshot(name === 'usage' ? 'usage-24' : name, name, [row(kind, `${kind}/one`)]));
  }
  sockets[0].frame(snapshot('attention', 'attention', []));
  const sent = [...sockets[0].sent];
  // Server protocol pings keep a quiet stream live; no client probe or inventory read occurs.
  for (let n = 0; n < 9; n++) { t.mock.timers.tick(8000); transport.onping(); }
  assert.equal(Date.now(), 1_800_000_072_000, '72 s exceeds the old 10 s client/probe and 60 s usage intervals');
  assert.equal(reads, 0);
  assert.equal(sockets.length, 1);
  assert.deepEqual(sockets[0].sent, sent);
  assert.equal(seen.size, 6);
  t.mock.timers.tick(HEARTBEAT_WAIT_MS);
  assert.equal(sockets[0].closed, true, 'a blackholed socket is closed when protocol heartbeats stop');
  assert.deepEqual(states.at(-1), ['reconnecting', 'st stopped answering']);
  const before = sockets[0].sent.length;
  transport.onping();
  assert.equal(sockets[0].sent.length, before, 'a closed socket cannot revive itself');
  feed.close();
});

test('screen windows share subscriptions, fold changes and leave after grace; pulls request snapshots', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient();
  const stream = await client.collectionStream({ onFrame() {}, onEnd() {} });
  const inventory = new InventoryFeed(30_000);
  inventory.open(stream);
  const updates = [], issues = [];
  const listener = { onRows: (rows, hasMore, at) => updates.push([rows, hasMore, at.store_index]), onIssue: issue => issues.push(issue) };
  const stop = inventory.watch('sessions', listener);
  assert.deepEqual(stream.sent, [], 'an older or not-yet-advertised daemon receives no optional command');
  inventory.setGranted(true);
  const native = { ...row('session', 'session/native'), managed: false, native_session_id: 'native', process: { pid: 42, exact_session: true } };
  inventory.frame(snapshot('sessions', 'sessions', [native], 1, true));
  const stopSecond = inventory.watch('sessions', listener);
  assert.equal(stream.sent.length, 1, 'another screen shares one socket slot');
  const changed = { ...native, revision: 'new', state: 'exited' };
  inventory.frame({ kind: 'changes', id: 'sessions', collection: 'sessions', upserts: [changed], removes: [], order: [native.id], snapshot: at(2), has_more: false });
  assert.deepEqual(updates.at(-1), [[changed], false, 2]);
  inventory.frame({ kind: 'resync', id: 'sessions', code: 'remote-unavailable', message: 'owner away', retryable: true });
  assert.match(issues.at(-1), /trying again/);
  t.mock.timers.tick(1000);
  assert.equal(stream.sent.length, 1, 'the retained server subscription retries; the phone sends no competing read');
  inventory.refresh(['sessions']);
  assert.equal(stream.sent.length, 2);
  inventory.frame(snapshot('sessions', 'sessions', [], 3));
  assert.deepEqual(updates.at(-1), [[], false, 3], 'explicit pull replaces with a fresh snapshot');
  stop(); t.mock.timers.tick(30_000);
  assert.equal(stream.sent.length, 2, 'the other screen keeps the window alive');
  stopSecond(); t.mock.timers.tick(29_999);
  assert.equal(stream.sent.length, 2);
  t.mock.timers.tick(1);
  assert.deepEqual(stream.sent.at(-1), { kind: 'unsubscribe', id: 'sessions' });
  assert.equal(inventory.frame(snapshot('sessions', 'sessions', [native], 4)), false, 'late frames cannot restore a left window');
  inventory.close();
});

test('each selected usage period shares one slot; reconnect snapshots replace the old rows', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const { client, sockets } = fakeClient();
  const gate = new ForegroundGate('active');
  const feed = new Feed(client, handlers, gate, () => 'action/test', [1000]);
  await flush(); feed.setInventoryGranted(true);
  const seen = [];
  const listen = { onRows: rows => seen.push(rows), onIssue() {} };
  feed.watchInventory('usage', listen, 24);
  feed.watchInventory('usage', { ...listen }, 24);
  feed.watchInventory('usage', listen, 168);
  assert.equal(sockets[0].sent.filter(command => command.collection === 'usage').length, 2);
  assert.equal(sockets[0].sent.find(command => command.id === 'usage-168').since_ms, Date.now() - 168 * 3_600_000);
  sockets[0].frame(snapshot('usage-24', 'usage', [row('usage', 'usage/old')]));
  sockets[0].drop(new Error('network lost'));
  t.mock.timers.tick(1000); await flush(); feed.setInventoryGranted(true);
  assert.equal(sockets.length, 2);
  sockets[1].frame(snapshot('usage-24', 'usage', [row('usage', 'usage/new')], 2));
  assert.deepEqual(seen.at(-1), [row('usage', 'usage/new')]);
  const count = seen.length;
  sockets[0].frame(snapshot('usage-24', 'usage', [row('usage', 'usage/ghost')], 3));
  assert.equal(seen.length, count, 'old socket frames are ignored');
  gate.update('background'); t.mock.timers.tick(120_000); await flush();
  assert.equal(sockets.length, 2, 'the background performs no reads or reconnects');
  gate.update('active'); await flush(); feed.setInventoryGranted(true);
  assert.equal(sockets[2].sent.filter(command => command.collection === 'usage').length, 2);
  feed.close();
});
