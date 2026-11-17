import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { ConversationAttempts } from './conversationAttempts.ts';
import { ClientError, resendBoundAction, sendBoundAction } from '../../clients/typescript/fractal-client/index.ts';
import { sendStart } from './startDelivery.ts';

const goldens = JSON.parse(readFileSync(new URL('../../docs/st3/client-v0/fixtures/thread-start-refusals.json', import.meta.url), 'utf8'));
for (const context of ['ordinary', 'start', 'reply']) {
  test(`${context}: the controller retains draft, original signed parameters/member and prior unknown dominance`, async () => {
    const draft = Object.freeze({ text: 'Original draft', quoteId: context === 'reply' ? 'message/quote' : undefined });
    const packet = Object.freeze({ id: 'action/original', idempotency_key: 'action/original', fence: {}, parameters: Object.freeze({
      to: 'agent/dotfiles/manager', from: 'person/ada', content: draft.text, signature: 'original-signature',
      ...(draft.quoteId ? { thread_quote_id: draft.quoteId } : {}),
    }) });
    let builds = 0, accepted = false;
    const sent = [];
    const ports = {
      retry: false,
      build: async () => { builds++; return packet; }, fence: async () => ({ snapshot_id: 'snapshot/refreshed' }),
      send: async request => { sent.push(request); if (!accepted) throw sent.length === 1 ? new TypeError('lost reply') : new ClientError(goldens['recipient-unassigned'], 409); },
      wait: async () => assert.fail('only explicit retry may send'),
    };
    let bound;
    const adapt = result => {
      if (result.how !== 'unconfirmed' || context === 'ordinary') return result;
      bound = result.request;
      return { how: 'unconfirmed', retry: async () => adapt(await resendBoundAction(result.request)) };
    };
    const attempts = new ConversationAttempts(result => result.how !== 'unconfirmed' ? undefined
      : context === 'ordinary' ? async () => adapt(await resendBoundAction(result.request)) : result.retry,
      error => ({ how: 'failed', error }));
    const phases = [];
    attempts.subscribe(() => phases.push(attempts.getSnapshot()[0]?.phase));
    const result = await attempts.send('attempt/original', draft, async () => adapt(await (context === 'start' ? sendStart : sendBoundAction)('original-member', ports)));
    assert.equal(result.how, 'unconfirmed');
    const originalBound = context === 'ordinary' ? result.request : bound;
    assert.equal(originalBound.member, 'original-member');
    assert.equal(originalBound.request, packet);
    assert.equal(originalBound.ports.fence, ports.fence);
    assert.equal(originalBound.ports.build, ports.build);
    if (context !== 'start') assert.equal(originalBound.ports, ports);
    const originalRetry = attempts.getSnapshot()[0].retry;
    assert.equal((await originalRetry()).how, 'unconfirmed', 'a later explicit-none refusal cannot settle earlier unknown');
    assert.equal(attempts.getSnapshot()[0].draft, draft);
    assert.equal(builds, 1);
    assert.equal(sent.length, 2);
    assert.equal(sent[1].parameters, packet.parameters);
    assert.equal(sent[1].idempotency_key, packet.idempotency_key);
    await originalRetry();
    assert.equal(sent.length, 2, 'stale retry cannot dispatch the next recovery generation');
    accepted = true;
    assert.equal((await attempts.getSnapshot()[0].retry()).how, 'sent');
    assert.equal(attempts.getSnapshot()[0].phase, 'confirmed');
    assert.ok(phases.includes('sending'));
    assert.ok(phases.includes('unconfirmed'));
    assert.equal(sent[2].parameters, packet.parameters);
    assert.equal(builds, 1);
  });
}

test('Start positive nonapplication releases its slot, with no automatic retry', async () => {
  let sends = 0;
  const attempts = new ConversationAttempts(result => result.retry, error => ({ how: 'failed', error }));
  const result = await attempts.send('attempt/start', { text: 'Kept' }, () => sendStart('original-member', {
    build: async () => ({ fence: {} }), fence: async () => ({}),
    send: async () => { sends++; throw new ClientError(goldens['recipient-unassigned'], 409); },
    wait: async () => assert.fail('no automatic Start retry'),
  }), result => { if (result.how !== 'unconfirmed') attempts.remove('attempt/start'); });
  assert.equal(result.how, 'failed');
  assert.equal(sends, 1);
  assert.equal(attempts.has('attempt/start'), false);
});

test('independent attempts do not steal draft snapshots; removed ownership cannot be recreated by completion', async () => {
  let finish;
  const attempts = new ConversationAttempts(() => undefined, error => ({ how: 'failed', error }));
  const first = attempts.send('attempt/one', { text: 'one' }, () => new Promise(resolve => { finish = resolve; }));
  await attempts.send('attempt/two', { text: 'two' }, async () => ({ how: 'failed', error: 'not sent' }));
  assert.equal(attempts.getSnapshot()[0].draft.text, 'one');
  assert.equal(attempts.getSnapshot()[1].phase, 'failed');
  attempts.remove('attempt/one');
  finish({ how: 'sent' }); await first;
  assert.deepEqual(attempts.getSnapshot().map(attempt => attempt.id), ['attempt/two']);
});

test('refused explicit Send as new leaves the earlier unknown and its original recovery intact', async () => {
  const attempts = new ConversationAttempts(result => result.retry, error => ({ how: 'failed', error }));
  const retry = async () => ({ how: 'sent' });
  await attempts.send('attempt/original', { text: 'kept' }, async () => ({ how: 'unconfirmed', retry }));
  const original = attempts.getSnapshot()[0];
  assert.equal((await attempts.sendAsNew(original.id, async () => ({ how: 'failed', error: 'refused' }))).how, 'failed');
  assert.equal(attempts.getSnapshot()[0], original);
  assert.equal((await original.retry()).how, 'sent');
});
