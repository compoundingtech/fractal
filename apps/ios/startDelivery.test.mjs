import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { appliedAnswer, ClientError, resendBoundAction } from '../../clients/typescript/fractal-client/index.ts';
import { sendStart } from './startDelivery.ts';
import { Schema } from 'effect';
import { decodeUnknownSync, ErrorEnvelope } from '../../clients/typescript/fractal-client/Schema.generated.ts';

const goldens = JSON.parse(readFileSync(new URL('../../docs/st3/client-v0/fixtures/thread-start-refusals.json', import.meta.url), 'utf8'));
test('exact canonical golden envelopes decode and round-trip without request ID hydration', () => {
  for (const envelope of Object.values(goldens)) {
    const decoded = decodeUnknownSync(ErrorEnvelope, 'strict')(envelope);
    assert.deepEqual(Schema.encodeSync(ErrorEnvelope)(decoded), envelope);
    assert.equal(decoded.request_id, envelope.request_id);
  }
});

for (const key of ['stale-snapshot']) {
  test(`golden POST ${key} without applied proof keeps the original signed packet, even after a later definitive refusal`, async () => {
    const packet = Object.freeze({ fence: { snapshot_id: 'snapshot/original' }, id: 'action/original', idempotency_key: 'action/original', parameters: { target_id: 'thread/original', agent_id: 'agent/original', signature: 'original-signature', content: 'Original draft' } });
    let builds = 0;
    const sent = [];
    const error = new ClientError(goldens[key], 409);
    assert.equal(appliedAnswer(error), undefined, 'retryable=false is not nonapplication proof');
    const result = await sendStart('fixture', {
      build: async () => { builds++; return packet; },
      fence: async () => ({ snapshot_id: 'snapshot/refreshed' }),
      send: async request => { sent.push(request); throw sent.length === 1 ? error : new ClientError(goldens['recipient-unassigned'], 409); },
      wait: async () => { assert.fail('Start admission must not auto-resend'); },
    });
    assert.equal(result.how, 'unconfirmed');
    assert.equal(result.request.request, packet);
    assert.equal(builds, 1);
    assert.equal(sent.length, 1);
    const retried = await resendBoundAction(result.request);
    assert.equal(retried.how, 'unconfirmed', 'a later refusal cannot settle an earlier unknown delivery');
    assert.equal(retried.request, result.request);
    assert.equal(sent.length, 2);
    assert.equal(sent[1].parameters, packet.parameters);
    assert.equal(sent[1].idempotency_key, packet.idempotency_key);
    assert.equal(builds, 1, 'never re-sign or rebuild an unknown Start');
  });
}

for (const key of ['recipient-unassigned', 'recipient-retired']) {
test(`golden POST ${key} proves applied:none and is a definitive failure`, async () => {
  const error = new ClientError(goldens[key], 409);
  assert.equal(appliedAnswer(error), 'none');
  let sends = 0;
  const result = await sendStart('fixture', {
    build: async () => ({ fence: {} }), fence: async () => ({}),
    send: async () => { sends++; throw error; },
    wait: async () => { assert.fail('a refusal needs explicit person action'); },
  });
  assert.deepEqual(result, { how: 'failed', error });
  assert.equal(sends, 1);
});
}
