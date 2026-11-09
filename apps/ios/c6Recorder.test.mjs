import assert from 'node:assert/strict';
import { test } from 'node:test';
import { C6Recorder, C6_LIMIT, nearestRank } from './c6Recorder.ts';

const fixture = (timeoutMs = 20) => {
  let time = 0, id = 0;
  const context = { route: 'tailscale', path: 'unknown' };
  const recorder = new C6Recorder({ now: () => time, id: () => `nonce-${++id}`, context: () => context, version: '1.0/source-sha', changed: () => {}, timeoutMs });
  return { recorder, context, advance: ms => { time += ms; } };
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
  assert.deepEqual(recorder.rows[0].start, { route: 'tailscale', path: 'unknown' });
  assert.equal(recorder.rows[0].status, 'invalid');
  assert.equal(recorder.rows[0].errorCode, 'route-changed');
  assert.equal(recorder.rows[1].status, 'ok');
  assert.equal(recorder.rows[0].ackMs, 4);
  const pathOnly = fixture();
  pathOnly.context.route = 'fabric'; pathOnly.context.path = 'direct';
  await pathOnly.recorder.run('status', async () => { pathOnly.context.path = 'relay'; });
  assert.equal(pathOnly.recorder.rows[0].errorCode, 'route-changed');
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
});

test('keeps ACK when echo is missing and never exports raw errors or message content', async () => {
  const { recorder } = fixture(1);
  await recorder.run('send-echo', async (_, hooks) => { hooks.ack(); });
  assert(recorder.rows.every(row => row.status === 'timeout' && row.ackMs === 0 && row.echoMs === null));
  await recorder.run('status', async () => { throw new Error('secret https://credential@private.invalid message body'); });
  const exported = JSON.parse(recorder.export());
  assert.equal(exported.schema, 'ios-c6.v1');
  assert.equal(exported.rows.length, 20);
  assert.deepEqual(Object.keys(exported.rows[0]).sort(), ['runId', 'blockId', 'index', 'operation', 'version', 'start', 'end', 'requestId', 'messageId', 'ackMs', 'echoMs', 'status', 'errorCode'].sort());
  assert(!/secret|credential|private.invalid|message body|nonce-3/.test(recorder.export()));
  assert(exported.rows.slice(10).every(row => row.status === 'failed' && row.errorCode === 'request-failed'));
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
