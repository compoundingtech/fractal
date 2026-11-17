import { describe, expect, it } from 'vitest';
import { ClientError } from './Client.generated.ts';
import { appliedAnswer, notApplied, outcomeUnknown } from './errors.ts';
import { resendAction, resendBoundAction, sendAction, sendBoundAction } from './delivery.ts';

const refusal = (outer: Record<string, unknown>, details: Record<string, unknown>, code = 'stale-fence') => new ClientError({
  api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/certainty',
  code, message: 'refusal', retryable: true, ...outer, details,
}, 409);

const probe = (error?: unknown) => {
  const calls = { builds: 0, posts: 0, fences: 0, waits: 0 };
  const ports = {
    build: async () => ({ id: `action/${++calls.builds}`, fence: 'original', signature: `nonce/${calls.builds}` }),
    fence: async () => { calls.fences++; return 'refreshed'; },
    send: async (_request: { id: string; fence: string; signature: string }) => { calls.posts++; if (error !== undefined) throw error; },
    wait: async (_ms: number) => { calls.waits++; },
  };
  return { calls, ports };
};

describe('delivery certainty', () => {
  it('requires explicit none without unknown or malformed contradictions', () => {
    for (const code of ['stale-fence', 'rate-limited', 'internal', 'forbidden']) {
      for (const [outer, details, answer] of [
        [{}, {}, undefined],
        [{ applied: 'none' }, {}, 'none'],
        [{}, { applied: 'none' }, 'none'],
        [{ applied: 'none' }, { applied: 'none' }, 'none'],
        [{ applied: 'unknown' }, { applied: 'none' }, 'unknown'],
        [{ applied: 'none' }, { applied: 'unknown' }, 'unknown'],
        ...[null, false, 0, '', 'future'].flatMap(applied => [
          [{ applied }, { applied: 'none' }, 'unknown'] as const,
          [{ applied: 'none' }, { applied }, 'unknown'] as const,
        ]),
      ] as const) {
        const error = refusal(outer, details, code);
        const before = JSON.stringify(error.response);
        expect(appliedAnswer(error)).toBe(answer);
        expect(notApplied(error)).toBe(answer === 'none');
        expect(outcomeUnknown(error)).toBe(answer !== 'none');
        expect(JSON.stringify(error.response)).toBe(before);
      }
    }
  });

  it.each([
    refusal({}, {}), refusal({}, {}, 'rate-limited'), refusal({}, {}, 'forbidden'),
    refusal({ applied: 'none' }, { applied: 'unknown' }),
    refusal({ applied: null }, { applied: 'none' }),
    new TypeError('Network request failed'), new Error('malformed reply'),
  ])('never dispatches or refreshes an unknown attempt again (%s)', async error => {
    const { calls, ports } = probe(error);
    const initial = await sendBoundAction('member/original', ports);
    expect(initial.how).toBe('unconfirmed');
    if (initial.how !== 'unconfirmed') throw new Error('Expected retained attempt');
    expect(await resendBoundAction(initial.request)).toEqual(initial);
    expect(await resendAction(ports, initial.request.request)).toEqual({ how: 'unconfirmed', request: initial.request.request });
    expect(calls).toEqual({ builds: 1, posts: 1, fences: 0, waits: 0 });
  });

  it('rebuilds only after explicit nonapplication', async () => {
    const { calls, ports } = probe();
    ports.send = async () => { if (++calls.posts === 1) throw refusal({}, { applied: 'none' }); };
    expect(await sendAction(ports)).toEqual({ how: 'sent' });
    expect(calls).toEqual({ builds: 2, posts: 2, fences: 0, waits: 1 });
  });

  it('retains pre-dispatch build failures as failed', async () => {
    const { calls, ports } = probe();
    const error = new Error('signing unavailable');
    ports.build = async () => { throw error; };
    expect(await sendAction(ports)).toEqual({ how: 'failed', error });
    expect(calls.posts).toBe(0);
  });

  it('reports explicit nonapplication without retry when disabled', async () => {
    const error = refusal({}, { applied: 'none' });
    const { calls, ports } = probe(error);
    expect(await sendAction({ ...ports, retry: false })).toEqual({ how: 'failed', error });
    expect(calls.posts).toBe(1);
  });
});
