import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError, resendBoundAction, sendBoundAction, resendAction, sendAction } from './index.ts';

const refusal = (details = {}, code = 'stale-fence') => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message: 'm', retryable: true, details }, 409);
const world = (error) => {
  const log = { built: 0, sent: [], fences: 0, waits: [] };
  const ports = {
    build: async () => ({ id: `action/${++log.built}`, fence: 'original', signature: `nonce/${log.built}` }),
    fence: async () => { log.fences++; return 'refreshed'; },
    send: async request => { log.sent.push(request); if (error) throw error; },
    wait: async ms => { log.waits.push(ms); },
  };
  return { ports, log };
};

test('a confirmed send dispatches once', async () => {
  const { ports, log } = world();
  assert.deepEqual(await sendAction(ports), { how: 'sent' });
  assert.equal(log.sent.length, 1);
});

for (const error of [refusal(), refusal({}, 'rate-limited'), refusal({ applied: 'unknown' }), refusal({ applied: null }), new TypeError('Network request failed'), new Error('malformed reply')]) {
  test(`unknown delivery never dispatches again: ${error.message} ${JSON.stringify(error.response?.details)}`, async () => {
    const { ports, log } = world(error);
    const outcome = await sendAction(ports);
    assert.deepEqual(outcome, { how: 'unconfirmed', request: log.sent[0] });
    assert.deepEqual(await resendAction(ports, outcome.request), outcome);
    assert.equal(log.sent.length, 1);
    assert.equal(log.built, 1);
    assert.equal(log.fences, 0);
    assert.deepEqual(log.waits, []);
  });
}

test('bound unknown attempts retain original transport without refreshing or posting', async () => {
  const { ports, log } = world(refusal());
  const outcome = await sendBoundAction('member/original', ports);
  assert.equal(outcome.how, 'unconfirmed');
  assert.deepEqual(await resendBoundAction(outcome.request), outcome);
  assert.equal(log.sent.length, 1);
  assert.equal(log.fences, 0);
});

test('only explicit nonapplication permits rebuilding', async () => {
  const { ports, log } = world(refusal({ applied: 'none' }));
  const send = ports.send;
  ports.send = async request => log.sent.length === 0 ? send(request) : log.sent.push(request);
  assert.deepEqual(await sendAction(ports), { how: 'sent' });
  assert.equal(log.built, 2);
  assert.notEqual(log.sent[0].signature, log.sent[1].signature);
});

test('disabled retries report proven nonapplication as failed', async () => {
  const error = refusal({ applied: 'none' });
  const { ports, log } = world(error);
  assert.deepEqual(await sendAction({ ...ports, retry: false }), { how: 'failed', error });
  assert.equal(log.sent.length, 1);
});

test('pre-dispatch build failures remain failed', async () => {
  const { ports, log } = world();
  const error = new Error('cannot sign');
  ports.build = async () => { throw error; };
  assert.deepEqual(await sendAction(ports), { how: 'failed', error });
  assert.equal(log.sent.length, 0);
});
