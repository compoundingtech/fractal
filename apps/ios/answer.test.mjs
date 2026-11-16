import assert from 'node:assert/strict';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { submitAnswer } from './answer.ts';

const refused = (code, details = {}) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message: 'm', retryable: false, details }, 409);

// A scripted Fractal: each send answers the next step; `waiting` is what stillWaiting reports.
function world(steps, waiting) {
  const log = { sent: [], built: 0, waited: [], asked: 0 };
  const ports = {
    async build() { log.built++; return { id: `action/${log.built}` }; },
    async send(request) {
      log.sent.push(request.id);
      const step = steps[Math.min(log.sent.length - 1, steps.length - 1)];
      if (step instanceof Error) throw step;
      return step === 'ok' ? { value: { kind: 'action-result', status: 'completed', action_id: request.id } } : step;
    },
    async stillWaiting() { log.asked++; const value = typeof waiting === 'function' ? waiting(log) : waiting; if (value instanceof Error) throw value; return value; },
    async wait(ms) { log.waited.push(ms); },
  };
  return { ports, log };
}

// An answer that lands is sent once.
{
  const { ports, log } = world(['ok'], true);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'sent' });
  assert.deepEqual(log.sent, ['action/1']);
  assert.equal(log.asked, 0, 'a success asks nothing');
}

// Cos, 2026-10-10: the answer landed but the reply was lost. The identical request is sent again
// (the same key), and Fractal's first answer comes back: a success, no second record.
{
  const { ports, log } = world([new TypeError('Network request failed'), 'ok'], true);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'sent' });
  assert.deepEqual(log.sent, ['action/1', 'action/1'], 'the same request, not a rebuilt one');
  assert.equal(log.built, 1);
}

// Fractal said it may still complete: the same.
{
  const { ports, log } = world([refused('unavailable', { applied: 'unknown' }), 'ok'], true);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'sent' });
  assert.deepEqual(log.sent, ['action/1', 'action/1']);
}

// The reply is lost twice, but the step is no longer waiting: it landed. Answered, not failed.
{
  const { ports } = world([new TypeError('Network request failed')], false);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'already' });
}

// A repeat submit after the answer landed: Fractal refuses the new key as stale. The step no longer
// waits, so it is a success.
{
  const { ports, log } = world([refused('stale-fence')], false);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'already' });
  assert.deepEqual(log.sent, ['action/1'], 'no pointless retries once it is answered');
}

// A stale fence while it still waits is a race: a fresh request each time, then the refusal.
{
  const { ports, log } = world([refused('stale-fence'), 'ok'], true);
  assert.deepEqual(await submitAnswer(ports), { ok: true, how: 'sent' });
  assert.deepEqual(log.sent, ['action/1', 'action/2']);
  const stuck = world([refused('stale-fence')], true);
  const outcome = await submitAnswer(stuck.ports);
  assert.equal(outcome.ok, false);
  assert.equal(stuck.log.sent.length, 8, 'a stuck race gives up after eight requests');
}

// A refusal while it still waits is shown as it is, once.
{
  const { ports, log } = world([refused('forbidden')], true);
  const outcome = await submitAnswer(ports);
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.response.code, 'forbidden');
  assert.deepEqual(log.sent, ['action/1']);
}

// If Fractal cannot say whether it still waits, the failure stands (nothing is claimed).
{
  const { ports } = world([new TypeError('Network request failed')], new Error('offline'));
  assert.equal((await submitAnswer(ports)).ok, false);
  const unsure = world([refused('forbidden')], null);
  assert.equal((await submitAnswer(unsure.ports)).ok, false);
}

// A request that cannot even be built is reported, not sent.
{
  const { ports, log } = world(['ok'], true);
  ports.build = async () => { throw new Error('Still connecting; try again in a moment.'); };
  const outcome = await submitAnswer(ports);
  assert.equal(outcome.ok, false);
  assert.equal(log.sent.length, 0);
}
console.log('answer ok');

// Explicit unknown metadata overrides stale/rate-limited shorthand. Defensive unsafe metadata
// is not a new protocol contract, but cannot justify inventing another action identity.
for (const details of [{applied:'unknown'},{applied:'none',replay_unsafe:true}]) {
 const {ports,log}=world([refused('stale-fence',details)],true);
 const result=await submitAnswer(ports);assert.equal(result.how,'unconfirmed');assert.equal(log.built,1);
 const checked=await result.check();assert.equal(checked.how,'unconfirmed');assert.equal(log.built,1);
 assert(log.sent.every(id=>id==='action/1'));
}

// A resolved transport promise is not a matching Completed receipt. Explicit checks keep
// the original identity even when a later replay is refused, then accept only its receipt.
for (const receipt of [undefined, {}, {value:{kind:'action-result',status:'accepted',action_id:'action/1'}}, {value:{kind:'action-result',status:'completed',action_id:'action/other'}}]) {
  const {ports,log}=world([receipt,refused('stale-fence',{applied:'none'}),'ok'],true);
  const first=await submitAnswer(ports);
  assert.equal(first.how,'unconfirmed');
  assert.deepEqual(log.sent,['action/1'],'non-completed result is not polled automatically');
  const second=await first.check();
  assert.equal(second.how,'unconfirmed','later refusal cannot erase prior ambiguity');
  assert.equal(log.built,1);
  assert.deepEqual(await second.check(),{ok:true,how:'sent'});
  assert.deepEqual(log.sent,['action/1','action/1','action/1']);
}
