import assert from 'node:assert/strict';
import { test, beforeEach, afterEach } from 'node:test';
import { C6Recorder, C6_LIMIT, nearestRank, checkC6Aborted } from './c6Recorder.ts';
import { ForegroundGate } from './foreground.ts';
import { createRequire } from 'node:module';
import { FractalClient } from '../../clients/typescript/fractal-client/Client.generated.ts';

const require = createRequire(import.meta.url);
const { AbortController: RNAbortController } = require('abort-controller/dist/abort-controller');
const capabilities = require('../../docs/st3/client-v0/fixtures/capabilities.json');
const nativeAbortController = globalThis.AbortController;
beforeEach(() => { globalThis.AbortController = RNAbortController; });
afterEach(() => { globalThis.AbortController = nativeAbortController; });

test('C6 send cancellation accepts the React Native AbortSignal polyfill', () => {
  const controller = new RNAbortController();
  assert.equal(controller.signal.throwIfAborted, undefined);
  assert.doesNotThrow(() => checkC6Aborted(undefined));
  assert.doesNotThrow(() => checkC6Aborted(controller.signal));
  controller.abort();
  assert.throws(() => checkC6Aborted(controller.signal), error => error.name === 'AbortError' && error.message === 'This operation was aborted');
});

test('C6 send cancellation preserves abort reasons without requiring DOMException', t => {
  for (const reason of [new TypeError('cancelled'), 'cancelled', 0, false]) {
    assert.throws(() => checkC6Aborted({ aborted: true, reason }), error => error === reason);
  }
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'DOMException');
  t.after(() => Object.defineProperty(globalThis, 'DOMException', descriptor));
  globalThis.DOMException = undefined;
  const controller = new RNAbortController();
  controller.abort();
  assert.throws(() => checkC6Aborted(controller.signal), error => error instanceof Error && error.name === 'AbortError' && error.message === 'This operation was aborted');
});

test('C6 status under the React Native AbortSignal polyfill', async t => {
  const original = globalThis.AbortController;
  globalThis.AbortController = RNAbortController;
  t.after(() => { globalThis.AbortController = original; });
  let fetches = 0, id = 0;
  const errors = [];
  const identity = new FractalClient({
    baseUrl: 'http://100.101.102.103:8790',
    credential: () => 'test-credential',
    fetchImpl: async () => {
      fetches++;
      return new Response(JSON.stringify({ ...capabilities, request_id: `request/c6-${fetches}` }), { headers: { 'Content-Type': 'application/json' } });
    },
  });
  const recorder = new C6Recorder({
    now: () => 0, id: () => `c6-${++id}`, version: 'test', changed: () => {},
    snapshot: () => ({ identity, context: { route: 'tailscale', path: 'unknown' } }),
    readFabricPath: async () => assert.fail('Fabric is off'),
  });
  await recorder.run('status', async (_, hooks, sample) => {
    try {
      if (!sample.identity) throw new Error('C6 disconnected');
      const reply = await sample.identity.capabilities({ signal: hooks.signal });
      hooks.request(reply.request_id);
      hooks.ack();
    } catch (error) {
      errors.push(error);
      throw error;
    }
  });
  t.diagnostic(JSON.stringify({
    rows: recorder.rows.map(({ status, errorCode, errorName, requestId, start, end }) => ({ status, errorCode, errorName, requestId, start, end })),
    fetches, errors: errors.map(error => ({ name: error.name, message: error.message })),
  }));
  assert.equal(recorder.rows.length, 10);
  if (errors.length) {
    assert.equal(fetches, 0);
    assert.equal(errors.length, 10);
    assert(errors.every(error => error instanceof TypeError && /throwIfAborted/.test(error.message)));
    assert(recorder.rows.every(row => row.status === 'failed' && row.errorCode === 'request-failed' && row.requestId === null && row.start.route === 'tailscale' && row.end.route === 'tailscale'));
    assert(recorder.rows.every(row => row.errorName === 'TypeError'));
  }
  assert.deepEqual(recorder.rows.map(row => row.status), Array(10).fill('ok'));
  assert.deepEqual(recorder.rows.map(row => row.requestId), Array.from({ length: 10 }, (_, index) => `request/c6-${index + 1}`));
  assert.equal(fetches, 10);
  assert(recorder.rows.every(row => row.errorName === null));
});

const fixture = (timeoutMs = 20) => {
  let time = 0, id = 0;
  const context = { route: 'tailscale', path: 'unknown' };
  const sample = { identity: {}, context };
  const recorder = new C6Recorder({ now: () => time, id: () => `nonce-${++id}`, snapshot: () => sample, readFabricPath: async () => sample.context.path, version: '1.0/source-sha', changed: () => {}, timeoutMs });
  return { recorder, context, sample, advance: ms => { time += ms; } };
};

test('freezes start context, invalidates a changed route, and does not refill failures', async () => {
  const { recorder, context, advance } = fixture();
  let calls = 0;
  await recorder.run('status', async (_, hooks) => {
    calls++;
    advance(4); hooks.request(`request/${calls}`); hooks.ack();
    context.route = 'fabric'; context.path = 'relay';
  });
  assert.equal(calls, 10);
  assert.deepEqual(recorder.rows[0].start, { route: 'tailscale', observedPath: 'unknown' });
  assert.equal(recorder.rows[0].status, 'invalid');
  assert.equal(recorder.rows[0].errorCode, 'route-changed');
  assert.equal(recorder.rows[1].status, 'ok');
  assert.equal(recorder.rows[0].ackMs, 4);
  const pathOnly = fixture();
  pathOnly.context.route = 'fabric'; pathOnly.context.path = 'direct';
  await pathOnly.recorder.run('status', async () => { pathOnly.context.path = 'relay'; });
  assert.equal(pathOnly.recorder.rows[0].status, 'ok');
  assert.equal(pathOnly.recorder.rows[0].pathFlag, 'changed');
});

test('correlates own nonce only, including echo before ACK, and records separate durations', async () => {
  const { recorder, advance } = fixture();
  await recorder.run('send-echo', async (nonce, hooks) => {
    hooks.request('action/123');
    advance(2);
    recorder.observe('wrong-author', `[C6:${nonce}]`, false);
    recorder.observe('wrong-nonce', '[C6:other]', true);
    advance(3);
    recorder.observe('message/exact', `prefix [C6:${nonce}] suffix`, true);
    advance(7); hooks.ack();
  });
  assert.equal(recorder.rows.length, 10);
  for (const row of recorder.rows) {
    assert.equal(row.status, 'ok'); assert.equal(row.echoMs, 5); assert.equal(row.ackMs, 12);
    assert.equal(row.messageId, 'message/exact'); assert.equal(row.requestId, 'action/123');
  }
});

test('timeouts are bounded rows, abort requests, and ignore late events', async () => {
  const { recorder } = fixture(1);
  let calls = 0, aborted = 0;
  const late = [];
  await recorder.run('send-echo', async (nonce, hooks) => {
    calls++;
    hooks.signal.addEventListener('abort', () => aborted++);
    late.push(() => { hooks.request('late'); hooks.ack(); recorder.observe('late', `[C6:${nonce}]`, true); });
  });
  late.forEach(event => event());
  assert.equal(calls, 10); assert.equal(aborted, 10);
  assert(recorder.rows.every(row => row.status === 'timeout' && row.errorCode === 'timeout' && row.ackMs === null && row.echoMs === null && row.requestId === null));
  assert(recorder.rows.every(row => row.errorName === null));
});

test('keeps ACK when echo is missing and never exports raw errors or message content', async () => {
  const { recorder } = fixture(1);
  await recorder.run('send-echo', async (_, hooks) => { hooks.ack(); });
  assert(recorder.rows.every(row => row.status === 'timeout' && row.ackMs === 0 && row.echoMs === null));
  await recorder.run('status', async () => { throw new Error('secret https://credential@private.invalid message body'); });
  const exported = JSON.parse(recorder.export());
  assert.equal(exported.schema, 'ios-c6.v2');
  assert.equal(exported.rows.length, 20);
  assert.deepEqual(Object.keys(exported.rows[0]).sort(), ['runId', 'blockId', 'index', 'operation', 'version', 'start', 'end', 'pathScope', 'pathFlag', 'requestId', 'messageId', 'ackMs', 'echoMs', 'status', 'errorCode', 'errorName'].sort());
  assert.deepEqual(Object.keys(exported.rows[0].start).sort(), ['observedPath', 'route']);
  assert.deepEqual(Object.keys(exported.rows[0].end).sort(), ['observedPath', 'route']);
  assert.equal(exported.rows[0].pathScope, 'unknown');
  assert(!/secret|credential|private.invalid|message body|nonce-3/.test(recorder.export()));
  assert(exported.rows.slice(10).every(row => row.status === 'failed' && row.errorCode === 'request-failed'));
  assert(exported.rows.slice(10).every(row => row.errorName === 'Error'));
});

test('runs sequentially, refuses concurrent blocks and bounds retained rows', async () => {
  const { recorder } = fixture();
  let active = 0, max = 0;
  for (let block = 0; block < 21; block++) {
    await recorder.run('status', async () => {
      active++; max = Math.max(max, active);
      await recorder.run('status', async () => assert.fail('concurrent block'));
      await Promise.resolve(); active--;
    });
  }
  assert.equal(max, 1); assert.equal(recorder.rows.length, C6_LIMIT);
  assert.equal(new Set(recorder.rows.map(row => row.blockId)).size, 20);
  recorder.clear(); assert.equal(recorder.rows.length, 0);
});

test('nearest-rank percentiles are sorted and not interpolated', () => {
  const input = [10, 3, 8, 1, 2, 9, 6, 4, 7, 5];
  assert.equal(nearestRank(input, 0.5), 5);
  assert.equal(nearestRank(input, 0.95), 10);
  assert.equal(nearestRank([], 0.95), null);
  assert.equal(input[0], 10);
});

test('each sample binds its transport and context, and client changes invalidate even an unchanged route', async () => {
  const { recorder, sample } = fixture();
  const original = sample.identity, replacement = {};
  const used = [];
  await recorder.run('status', async (_, hooks, captured) => {
    used.push(captured.identity);
    if (used.length === 1) sample.identity = replacement;
    hooks.ack();
  });
  assert.equal(used[0], original);
  assert(used.slice(1).every(client => client === replacement));
  assert.equal(recorder.rows[0].status, 'invalid');
  assert(recorder.rows.slice(1).every(row => row.status === 'ok' && row.start.route === 'tailscale'));
  const switched = fixture();
  let count = 0;
  await switched.recorder.run('status', async (_, hooks, captured) => {
    assert.equal(captured.identity, switched.sample.identity);
    if (++count === 1) {
      switched.sample.identity = {};
      switched.sample.context = { route: 'fabric', path: 'direct' };
    }
    hooks.ack();
  });
  assert.equal(switched.recorder.rows[0].status, 'invalid');
  assert.equal(switched.recorder.rows[0].start.route, 'tailscale');
  assert(switched.recorder.rows.slice(1).every(row => row.status === 'ok' && row.start.route === 'fabric'));
});

for (const reason of ['blur', 'background', 'subscription-replaced']) {
  test(`cancel on ${reason} aborts the active sample and retains cancelled rows without more sends`, async () => {
    const { recorder } = fixture(1000);
    let sends = 0, closed = 0, aborted = false;
    await recorder.run('send-echo', async (_, hooks) => {
      sends++;
      hooks.cleanup(() => closed++);
      hooks.signal.addEventListener('abort', () => { aborted = true; });
      hooks.ack();
      recorder.cancel();
    });
    assert.equal(sends, 1);
    assert.equal(closed, 1);
    assert.equal(aborted, true);
    assert.equal(recorder.rows.length, 10);
    assert(recorder.rows.every(row => row.status === 'cancelled' && row.errorCode === 'cancelled'));
    assert(recorder.rows.every(row => row.errorName === null));
    assert(recorder.rows.slice(1).every(row => row.requestId === null && row.ackMs === null));
  });
}

test('the app foreground gate cancels a running recorder on background', async () => {
  const { recorder } = fixture(1000);
  const gate = new ForegroundGate('active');
  const unsubscribe = gate.subscribe(active => { if (!active) recorder.cancel(); });
  let sends = 0;
  await recorder.run('send-echo', async () => {
    sends++;
    gate.update('background');
  });
  unsubscribe();
  assert.equal(sends, 1);
  assert(recorder.rows.every(row => row.errorCode === 'cancelled'));
});

test('Fabric reads fresh paths twice per sample, ignores cached paths, and excludes reader time', async () => {
  let time = 0, id = 0, reads = 0;
  const sample = { identity: {}, context: { route: 'fabric', path: 'direct' } };
  const recorder = new C6Recorder({
    now: () => time, id: () => `${++id}`, snapshot: () => sample, version: 'source', changed: () => {},
    readFabricPath: async () => { reads++; time += 100; return reads % 2 === 1 ? 'relay' : 'direct'; },
  });
  await recorder.run('send-echo', async (nonce, hooks) => {
    time += 3; hooks.ack();
    time += 4; recorder.observe('message/exact', `[C6:${nonce}]`, true);
  });
  assert.equal(reads, 20, 'two uncached native path reads for each of ten samples');
  for (const row of recorder.rows) {
    assert.equal(row.start.observedPath, 'relay', 'cached direct hint must not label this observation');
    assert.equal(row.end.observedPath, 'direct');
    assert.equal(row.pathScope, 'module-global'); assert.equal(row.pathFlag, 'changed');
    assert.equal(row.status, 'ok'); assert.equal(row.errorCode, null);
    assert.equal(row.ackMs, 3); assert.equal(row.echoMs, 7);
  }
  assert.equal(nearestRank(recorder.rows.filter(row => row.status === 'ok').map(row => row.ackMs), 0.5), 3, 'flagged successes remain in route percentiles');
  const exported = JSON.parse(recorder.export());
  assert(exported.rows.every(row => row.pathScope === 'module-global' && row.start.observedPath === 'relay' && !('path' in row.start)));
});

test('unknown at either Fabric boundary flags a valid row; Tailscale never reads Fabric paths', async () => {
  for (const unknownAt of ['start', 'end']) {
    let id = 0, reads = 0;
    const recorder = new C6Recorder({
      now: () => 0, id: () => `${++id}`, version: 'source', changed: () => {},
      snapshot: () => ({ identity: null, context: { route: 'fabric', path: 'direct' } }),
      readFabricPath: async () => { reads++; return (reads % 2 === 1) === (unknownAt === 'start') ? 'unknown' : 'direct'; },
    });
    await recorder.run('status', async (_, hooks) => hooks.ack());
    assert.equal(reads, 20);
    assert(recorder.rows.every(row => row.status === 'ok' && row.errorCode === null && row.pathFlag === 'unknown' && row.pathScope === 'module-global'));
  }
  let reads = 0, id = 0;
  const recorder = new C6Recorder({
    now: () => 0, id: () => `${++id}`, version: 'source', changed: () => {},
    snapshot: () => ({ identity: null, context: { route: 'tailscale', path: 'unknown' } }),
    readFabricPath: async () => { reads++; return 'direct'; },
  });
  await recorder.run('status', async (_, hooks) => hooks.ack());
  assert.equal(reads, 0);
  assert(recorder.rows.every(row => row.status === 'ok' && row.start.observedPath === 'unknown' && row.end.observedPath === 'unknown' && row.pathScope === 'unknown' && row.pathFlag === null));
});
