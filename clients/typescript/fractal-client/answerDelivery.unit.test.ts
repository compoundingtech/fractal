import { describe, expect, it } from 'vitest';
import { ClientError } from './Client.generated.ts';
import { submitAnswer } from '../../../apps/ios/answerDelivery.ts';
import { submitAnswer as submitLegacyAnswer } from '../../../apps/ios/answer.ts';

const refusal = (details: Record<string, unknown> = {}) => new ClientError({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/answer',
  code: 'stale-fence', message: 'snapshot changed', retryable: true, details,
}, 409);

for (const submit of [submitAnswer, submitLegacyAnswer]) {
  describe(submit === submitAnswer ? 'answer delivery' : 'legacy answer delivery', () => {
    it.each([refusal(), refusal({ applied: 'unknown' }), refusal({ applied: null }), new TypeError('Network request failed')])('does not replay unknown delivery (%s)', async error => {
      let builds = 0, posts = 0, reads = 0, waits = 0;
      const ports = {
        build: async () => ({ id: `action/${++builds}` }),
        send: async () => { posts++; throw error; },
        stillWaiting: async () => { reads++; return true; },
        wait: async () => { waits++; },
      };
      const outcome = await submit(ports);
      expect(outcome.ok).toBe(false);
      if ('check' in outcome) {
        await outcome.check();
        await outcome.check();
        expect(reads).toBe(3);
      }
      expect({ builds, posts, waits }).toEqual({ builds: 1, posts: 1, waits: 0 });
    });

    it('rebuilds only after explicit nonapplication', async () => {
      let builds = 0, posts = 0;
      expect(await submit({
        build: async () => ({ id: `action/${++builds}` }),
        send: async request => {
          if (++posts === 1) throw refusal({ applied: 'none' });
          return { value: { kind: 'action-result', status: 'completed', action_id: request.id } };
        },
        stillWaiting: async () => true,
        wait: async () => {},
      })).toEqual({ ok: true, how: 'sent' });
      expect({ builds, posts }).toEqual({ builds: 2, posts: 2 });
    });
  });
}

it('checks malformed receipts with reads only, never a second POST', async () => {
  let posts = 0, waiting = true;
  const outcome = await submitAnswer({
    build: async () => ({ id: 'action/original' }),
    send: async () => { posts++; return { value: { kind: 'action-result', status: 'accepted', action_id: 'action/original' } }; },
    stillWaiting: async () => waiting,
    wait: async () => {},
  });
  expect(outcome.ok).toBe(false);
  if (!outcome.ok && outcome.how === 'unconfirmed') {
    expect((await outcome.check()).ok).toBe(false);
    waiting = false;
    expect(await outcome.check()).toEqual({ ok: true, how: 'already' });
  } else throw new Error('Expected unknown delivery');
  expect(posts).toBe(1);
});
