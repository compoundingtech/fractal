import assert from 'node:assert/strict';
import { test } from 'node:test';
import { API_VERSION, ClientError, FractalClient, outcomeUnknown } from '../../clients/typescript/fractal-client/index.ts';
import { resendBoundMessage, sendBoundMessage } from './messageSend.ts';
import { threadReceiptCheck } from './threadReceipt.ts';

const actionId = 'action/ios-thread';
const targetId = 'thread/review';
const messageId = 'message/0123456789abcdef';
const value = {
  kind: 'action-result', status: 'completed', action_id: actionId, operation_id: 'operation/one',
  snapshot_id: 'snapshot/one', affected_ids: [targetId, messageId],
};
const result = payload => ({ api_version: API_VERSION, snapshot: { id: 'snapshot/one' }, value: payload });
const refusalPayload = {
  api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: actionId,
  code: 'forbidden', message: 'the device grant expired', details: {}, retryable: false,
};

test('thread actions require a completed receipt for the exact action, thread and message', async () => {
  const confirm = threadReceiptCheck({ actionId, targetId, messageId });
  await confirm(async () => result(value));
  for (const payload of [
    null,
    { ...value, status: 'accepted' },
    { ...value, action_id: 'action/other' },
    { ...value, affected_ids: ['thread/other', messageId] },
    { ...value, affected_ids: [targetId, 'message/other'] },
    { ...value, operation_id: undefined },
  ]) {
    await assert.rejects(confirm(async () => result(payload)), outcomeUnknown);
  }
  const lifecycle = { ...value, affected_ids: [targetId] };
  await assert.rejects(threadReceiptCheck({ actionId, targetId })(async () => result(lifecycle)), outcomeUnknown);
  await threadReceiptCheck({ actionId, targetId })(async () => result({ ...lifecycle, metadata_revision: 'claim/reopen' }));
});

test('an ambiguous reply keeps the signed action and key until its matching receipt arrives', async () => {
  const received = [];
  const sdk = new FractalClient({
    baseUrl: 'https://paired.invalid',
    fetchImpl: async (_url, options) => {
      if (options.method === 'GET') return Response.json(result({ capabilities: [] }));
      received.push(JSON.parse(options.body));
      if (received.length === 1) return Response.json(result({ ...value, affected_ids: [targetId, 'message/other'] }));
      if (received.length === 2) return new Response('{', { status: 200 });
      if (received.length === 3) return Response.json(refusalPayload, { status: 403 });
      return Response.json(result(value));
    },
  });
  const request = {
    id: actionId, idempotency_key: actionId, fence: { snapshot_id: 'snapshot/old', subject_revisions: {} },
    parameters: { target_id: targetId, content: 'Hello', signature: { nonce: 'original' } },
  };
  const confirm = threadReceiptCheck({ actionId, targetId, messageId });
  const ports = {
    build: async () => request,
    fence: async () => ({ snapshot_id: 'snapshot/new', subject_revisions: {} }),
    send: input => confirm(() => sdk.threadReply(input)),
    wait: async () => {},
  };
  const initial = await sendBoundMessage('member/one', ports);
  assert.equal(initial.how, 'unconfirmed');
  assert.deepEqual(received[0], received[1]);
  const retried = await resendBoundMessage(initial.request);
  assert.equal(retried.how, 'sent');
  assert.deepEqual(received[2], { ...received[0], fence: await ports.fence() });
  assert.deepEqual(received[3], received[2]);
});

test('a definitive daemon refusal still reaches the caller', async () => {
  const refusal = new ClientError(refusalPayload, 403);
  const confirm = threadReceiptCheck({ actionId, targetId, messageId });
  await assert.rejects(confirm(async () => { throw refusal; }), error => error === refusal);
});
