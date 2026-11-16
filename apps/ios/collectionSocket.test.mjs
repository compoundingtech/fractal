import assert from 'node:assert/strict';
import { test } from 'node:test';
import { FractalClient, plainError } from '../../clients/typescript/fractal-client/index.ts';
import { collectionSocketFactory } from './collectionSocket.ts';
import { Feed, OPEN_WAIT_MS } from './feed.ts';
import { ForegroundGate } from './foreground.ts';

const flush = async () => { for (let n = 0; n < 5; n++) await Promise.resolve(); };
function bridge() {
  const listeners = new Set(), opened = [], sent = [], closed = [];
  return {
    opened, sent, closed, listeners,
    open: (...args) => opened.push(args), send: (...args) => sent.push(args), close: (...args) => closed.push(args),
    addListener: (_, listener) => { listeners.add(listener); return { remove: () => listeners.delete(listener) }; },
    emit: event => { for (const listener of listeners) listener(event); },
  };
}

test('the native socket forwards authentication, commands, frames and protocol ping without a read', async () => {
  const native = bridge(), socketFactory = collectionSocketFactory(native);
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', credential: async () => 'test-credential' });
  let pings = 0, ended = 0;
  const frames = [];
  const stream = await client.collectionStream({ onFrame: frame => frames.push(frame), onEnd: () => ended++, socket: (...args) => {
    const socket = socketFactory(...args); socket.onping = () => pings++; return socket;
  } });
  stream.subscribe('usage-24', 'usage', 200, { since_ms: 123, until_ms: 456 });
  await flush();
  const [id, url, protocols, headers] = native.opened[0];
  assert.equal(url, 'wss://gateway.invalid/v1/client/collections/stream');
  assert.deepEqual(protocols, ['fractal.client.collections.v0']);
  assert.equal(headers.Authorization, 'Bearer test-credential');
  assert.equal(native.sent.length, 0, 'commands wait for the authenticated open event');
  native.emit({ id, kind: 'open' });
  assert.deepEqual(JSON.parse(native.sent[0][1]), { kind: 'subscribe', id: 'usage-24', collection: 'usage', limit: 200, since_ms: 123, until_ms: 456 });
  native.emit({ id, kind: 'ping' });
  assert.equal(pings, 1); assert.equal(native.sent.length, 1, 'SocketRocket answers pong natively; JS sends nothing');
  native.emit({ id, kind: 'message', data: JSON.stringify({
    kind: 'snapshot', id: 'usage-24', collection: 'usage', items: [], order: [], has_more: false,
    snapshot: { id: 'snapshot/test', host_id: 'host/test', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-09-20T00:00:00Z' },
  }) });
  assert.equal(frames.length, 1);
  stream.close();
  assert.deepEqual(native.closed, [[id, 1000]]);
  assert.equal(native.listeners.size, 0);
  native.emit({ id, kind: 'ping' });
  assert.equal(pings, 1, 'closed sockets release every listener');
  assert.equal(ended, 0, 'intentional close is not a reconnect error');
});

test('native opening failure and remote close end once and release listeners', async () => {
  const native = bridge();
  native.open = async () => { throw new Error('Connection refused'); };
  const socket = collectionSocketFactory(native)('wss://gateway.invalid', [], {});
  let errors = 0;
  socket.onerror = () => errors++;
  await flush();
  assert.equal(errors, 1); assert.equal(native.listeners.size, 0); assert.equal(native.closed.length, 1);
  const next = bridge(), nextSocket = collectionSocketFactory(next)('wss://gateway.invalid', [], {});
  let closes = 0;
  nextSocket.onclose = event => { closes++; assert.equal(event.code, 1001); };
  await flush();
  const id = next.opened[0][0];
  next.emit({ id, kind: 'close', code: 1001 }); next.emit({ id, kind: 'close', code: 1001 });
  assert.equal(closes, 1); assert.equal(next.listeners.size, 0);
});

test('native NSError identity reaches the SDK and plain feed error without localized text', async () => {
  const native = bridge(), ends = [];
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid' });
  await client.collectionStream({ onFrame() {}, onEnd: error => ends.push(error), socket: collectionSocketFactory(native) });
  await flush();
  const id = native.opened[0][0];
  native.emit({ id: id + 1, kind: 'error', domain: 'IgnoredDomain', code: 1 });
  assert.deepEqual(ends, [], 'another socket cannot end this stream');
  const failure = { id, kind: 'error', domain: 'NSURLErrorDomain', code: -1009,
    message: 'private gateway and credential', localizedDescription: 'private localized description', userInfo: { url: 'https://private.invalid' } };
  native.emit(failure);
  native.emit(failure);
  assert.equal(ends.length, 1);
  assert.ok(ends[0] instanceof Error);
  assert.equal(ends[0].message, 'The collections socket failed (NSURLErrorDomain -1009)');
  assert.equal(plainError(ends[0]), ends[0].message, 'the feed plainError path retains domain/code');
  assert.doesNotMatch(JSON.stringify({ issue: plainError(ends[0]) }), /private|localizedDescription|userInfo/);
  assert.equal(native.listeners.size, 0);
});

test('the bridge forwards only NSError domain/code and keeps an untyped failure generic', async () => {
  const native = bridge(), socket = collectionSocketFactory(native)('wss://gateway.invalid', [], {});
  const errors = [];
  socket.onerror = event => errors.push(event);
  await flush();
  native.emit({ id: native.opened[0][0], kind: 'error', domain: 'SRWebSocketErrorDomain', code: 2132,
    message: 'private', localizedDescription: 'private', reason: 'private', data: 'private' });
  assert.deepEqual(errors, [{ domain: 'SRWebSocketErrorDomain', code: 2132 }]);

  const generic = bridge(), ends = [];
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid' });
  await client.collectionStream({ onFrame() {}, onEnd: error => ends.push(error), socket: collectionSocketFactory(generic) });
  await flush();
  generic.emit({ id: generic.opened[0][0], kind: 'error', message: 'private' });
  assert.equal(ends[0].message, 'The collections socket failed');
});

test('native opening and connected failures reach the real feed issue with domain/code only', async t => {
  for (const opened of [false, true]) {
    await t.test(opened ? 'after open' : 'during opening', async t => {
      const native = bridge(), states = [];
      const client = new FractalClient({ baseUrl: 'https://gateway.invalid' });
      const feed = new Feed(client, { onWindow() {}, onConnection: (state, issue) => states.push([state, issue]) },
        new ForegroundGate('active'), () => 'action/test', [1000], undefined, false, undefined, collectionSocketFactory(native));
      t.after(() => feed.close());
      await flush();
      const id = native.opened[0][0];
      if (opened) { native.emit({ id, kind: 'open' }); await flush(); }
      native.emit({ id, kind: 'error', domain: 'NSURLErrorDomain', code: -1009,
        message: 'private credential', localizedDescription: 'private gateway', userInfo: { url: 'https://private.invalid' } });
      await flush();
      assert.deepEqual(states.at(-1), ['reconnecting', 'The collections socket failed (NSURLErrorDomain -1009)']);
      assert.doesNotMatch(JSON.stringify(states), /private|localizedDescription|userInfo/);
      assert.equal(native.listeners.size, 0);
    });
  }
});

test('a real native bridge that never opens times out and releases its listener', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const native = bridge(), states = [];
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid' });
  const feed = new Feed(client, { onWindow() {}, onConnection: (state, issue) => states.push([state, issue]) },
    new ForegroundGate('active'), () => 'action/test', [1000], undefined, false, undefined, collectionSocketFactory(native));
  t.after(() => feed.close());
  await flush();
  assert.equal(native.opened.length, 1);
  t.mock.timers.tick(OPEN_WAIT_MS);
  await flush();
  assert.equal(states.at(-1)[0], 'reconnecting');
  assert.match(states.at(-1)[1], /open|answer|long/i);
  assert.equal(native.listeners.size, 0);
  assert.equal(native.closed.length, 1);
  native.emit({ id: native.opened[0][0], kind: 'open' });
  await flush();
  assert.equal(states.at(-1)[0], 'reconnecting', 'the late native open cannot revive the timed-out stream');
});
