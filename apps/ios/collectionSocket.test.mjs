import assert from 'node:assert/strict';
import { test } from 'node:test';
import { FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { collectionSocketFactory } from './collectionSocket.ts';

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
  assert.deepEqual(protocols, ['st3.client.collections.v0']);
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
