import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/st3-client/index.ts';
import { resendMessage, sendMessage } from './messageSend.ts';

const refused = (code, status, details = {}, retryable = false) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message: 'm', retryable, details }, status);
// st could not confirm the send in time; it may still complete.
const unconfirmed = () => refused('message-send-unconfirmed', 503, { applied: 'unknown', action_id: 'action/1', idempotency_key: 'action/1' });
const rateLimited = () => refused('rate-limited', 429, { applied: 'none' }, true);

// A scripted st: each send answers the next step. Each built request is new: its own id, key and
// signature nonce, as the app signs one.
function world(steps) {
  const log = { sent: [], built: 0, fences: 0, waited: [] };
  const ports = {
    async build() {
      log.built++;
      const id = `action/ios-${log.built}`;
      return { id, idempotency_key: id, fence: { snapshot_id: `snapshot/${log.built}`, subject_revisions: {} }, parameters: { to: 'agent/example', content: 'hello', signature: { nonce: `nonce-${log.built}`, signature: `signed-${log.built}` } } };
    },
    async fence() { log.fences++; return { snapshot_id: `snapshot/fresh-${log.fences}`, subject_revisions: {} }; },
    async send(request) {
      log.sent.push(structuredClone(request));
      const step = steps[Math.min(log.sent.length - 1, steps.length - 1)];
      if (step instanceof Error) throw step;
      return step;
    },
    async wait(ms) { log.waited.push(ms); },
  };
  return { ports, log };
}

test('a send st confirms at once is sent, once', async () => {
  const { ports, log } = world(['ok']);
  assert.deepEqual(await sendMessage(ports), { how: 'sent' });
  assert.equal(log.sent.length, 1);
  assert.equal(log.built, 1);
});

test('a send st leaves unconfirmed twice is unconfirmed and carries the exact request it sent', async () => {
  const { ports, log } = world([unconfirmed(), unconfirmed()]);
  const outcome = await sendMessage(ports);
  assert.equal(outcome.how, 'unconfirmed');
  assert.equal(log.built, 1, 'never rebuilt');
  assert.equal(log.sent.length, 2);
  assert.deepEqual(log.sent[1], log.sent[0], 'the repeat is the identical request');
  assert.deepEqual(outcome.request, log.sent[0], 'the outcome carries that request, not an error');
  assert.deepEqual(log.waited, [1000]);
});

test('an unconfirmed send whose repeat st confirms is sent', async () => {
  const { ports, log } = world([unconfirmed(), 'ok']);
  assert.deepEqual(await sendMessage(ports), { how: 'sent' });
  assert.equal(log.built, 1);
  assert.equal(log.sent.length, 2);
});

test('sending again repeats the unconfirmed request with only its fence read anew', async () => {
  const first = world([unconfirmed(), unconfirmed()]);
  const outcome = await sendMessage(first.ports);
  assert.equal(outcome.how, 'unconfirmed');
  const again = world(['ok']);
  assert.deepEqual(await resendMessage(again.ports, outcome.request), { how: 'sent' });
  assert.equal(again.log.built, 0, 'nothing is rebuilt or re-signed');
  assert.equal(again.log.sent.length, 1);
  const [resent] = again.log.sent;
  const { fence, ...rest } = resent;
  const { fence: oldFence, ...original } = first.log.sent[0];
  assert.deepEqual(rest, original, 'the same id, key and parameters, signature and nonce included');
  assert.notDeepEqual(fence, oldFence);
  assert.equal(fence.snapshot_id, 'snapshot/fresh-1');
});

test('sending again that st still does not confirm stays unconfirmed with the same request', async () => {
  const { ports, log } = world([unconfirmed()]);
  const request = (await world([]).ports.build());
  const outcome = await resendMessage(ports, request);
  assert.deepEqual(outcome, { how: 'unconfirmed', request });
  assert.ok(log.sent.every(sent => sent.id === request.id && sent.idempotency_key === request.idempotency_key));
  // A refusal that applied nothing: the same request again under a fresh fence, never a new id.
  const limited = world([rateLimited()]);
  assert.deepEqual(await resendMessage(limited.ports, request), { how: 'unconfirmed', request });
  assert.equal(limited.log.built, 0);
  assert.ok(limited.log.sent.length > 1 && limited.log.sent.every(sent => sent.id === request.id));
});

test('only a refusal that applied nothing builds a request with a fresh id', async () => {
  // rate-limited, applied: none: tried again afresh.
  const limited = world([rateLimited(), 'ok']);
  assert.deepEqual(await sendMessage(limited.ports), { how: 'sent' });
  assert.deepEqual(limited.log.sent.map(sent => sent.id), ['action/ios-1', 'action/ios-2']);
  assert.notEqual(limited.log.sent[1].parameters.signature.nonce, limited.log.sent[0].parameters.signature.nonce);
  // applied: none under another code: the same.
  const none = world([refused('unavailable', 503, { applied: 'none' }), 'ok']);
  assert.deepEqual(await sendMessage(none.ports), { how: 'sent' });
  assert.equal(none.log.built, 2);
  // A lost reply is repeated, never rebuilt.
  const lost = world([new TypeError('Network request failed'), 'ok']);
  assert.deepEqual(await sendMessage(lost.ports), { how: 'sent' });
  assert.equal(lost.log.built, 1);
  // After an unknown outcome, a repeat refused as rate-limited stays that request: the first may still land.
  const raced = world([unconfirmed(), rateLimited()]);
  const outcome = await sendMessage(raced.ports);
  assert.equal(outcome.how, 'unconfirmed');
  assert.equal(raced.log.built, 1);
  // A refusal is shown as it is, once.
  const forbidden = world([refused('forbidden', 403)]);
  const failed = await sendMessage(forbidden.ports);
  assert.equal(failed.how, 'failed');
  assert.equal(failed.error.response.code, 'forbidden');
  assert.equal(forbidden.log.built, 1);
  assert.equal(forbidden.log.sent.length, 1);
});
