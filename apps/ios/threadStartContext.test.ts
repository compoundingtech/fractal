import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { EnvelopeOf, FractalClient, SourceEpisode, ThreadStartContext, ThreadStartRecipient } from '../../clients/typescript/fractal-client/index.ts';
import { ThreadStartContextReader } from './threadStartContext.ts';

const origin: SourceEpisode = { source_id: 'notice/system-typed', episode: 'episode/exact' };
// Generated DTO-typed presentation inputs, not live-server success observations.
const context = (recipient: ThreadStartRecipient): ThreadStartContext => ({ kind: 'thread-start-context', ...origin, start_recipient: recipient });
const envelope = (value: ThreadStartContext): EnvelopeOf<ThreadStartContext> => ({
  api_version: 'fractal.client.v0', request_id: 'request/unit', value,
  snapshot: { id: 'snapshot/unit', host_id: 'host/unit', created_at: '2026-11-17T00:00:00Z', projection_version: 'client-projection.v0', store_index: 1 },
});

test('typed system notice without a source agent or roster resolves through the canonical exact-key client method', async () => {
  const calls: Array<Parameters<FractalClient['threadStartContext']>> = [];
  const selected = context({ agent_id: 'agent/configured', basis: { _tag: 'configured' } });
  const client: Pick<FractalClient, 'threadStartContext'> = { threadStartContext: async (...args) => { calls.push(args); return envelope(selected); } };
  const reader = new ThreadStartContextReader(client, origin);
  await reader.load();
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0]?.slice(0, 2), [origin.source_id, origin.episode]);
  assert.deepEqual(reader.getSnapshot(), { kind: 'ready', context: selected, changed: false });
});

test('offline original responsibility stays unchanged; only a canonical terminal fallback changes the recipient', async () => {
  const selected = context({ agent_id: 'agent/offline-original', basis: { _tag: 'original' } });
  const reader = new ThreadStartContextReader({ threadStartContext: async () => envelope(selected) }, origin);
  await reader.load();
  assert.deepEqual(reader.getSnapshot(), { kind: 'ready', context: selected, changed: false });
});

test('A to B then B to C recipient refresh clears prior selection and requires visible fresh-send review without sending', async () => {
  const recipients = ['agent/A', 'agent/B', 'agent/C'];
  let reads = 0;
  const client: Pick<FractalClient, 'threadStartContext'> = {
    threadStartContext: async () => envelope(context({ agent_id: recipients[reads++]!, basis: { _tag: 'fallback', original_agent_id: 'agent/original', retirement: { claim_id: 'claim/retired', kind: 'agent.retired', state: 'retired' } } })),
  };
  const reader = new ThreadStartContextReader(client, origin);
  await reader.load();
  for (const expected of recipients.slice(1)) {
    const pending = reader.load(true);
    assert.deepEqual(reader.getSnapshot(), { kind: 'loading' }, 'old recipient cannot remain sendable during refresh');
    await pending;
    const state = reader.getSnapshot();
    assert.equal(state.kind, 'ready');
    if (state.kind !== 'ready') return;
    assert.equal(state.context.start_recipient.agent_id, expected);
    assert.equal(state.changed, true);
    assert.equal(state.context.start_recipient.basis._tag, 'fallback');
  }
  assert.equal(reads, 3, 'the reader performs only the requested preview reads, never a send');
});

test('refusal followed by unavailable context removes the old target instead of inventing a fallback', async () => {
  let reads = 0;
  const reader = new ThreadStartContextReader({ threadStartContext: async () => {
    if (reads++ === 0) return envelope(context({ agent_id: 'agent/A', basis: { _tag: 'original' } }));
    throw new Error('private authority detail');
  } }, origin);
  await reader.load();
  await reader.load(true);
  assert.equal(reader.getSnapshot().kind, 'error');
  assert.ok(!JSON.stringify(reader.getSnapshot()).includes('agent/A'));
  assert.ok(!JSON.stringify(reader.getSnapshot()).includes('private authority detail'));
});

test('released or superseded context requests cannot resurrect a previous recipient', async () => {
  const first = Promise.withResolvers<EnvelopeOf<ThreadStartContext>>();
  const second = Promise.withResolvers<EnvelopeOf<ThreadStartContext>>();
  let reads = 0;
  const reader = new ThreadStartContextReader({ threadStartContext: () => reads++ === 0 ? first.promise : second.promise }, origin);
  const older = reader.load();
  const newer = reader.load(true);
  second.resolve(envelope(context({ agent_id: 'agent/B', basis: { _tag: 'configured' } })));
  await newer;
  first.resolve(envelope(context({ agent_id: 'agent/A', basis: { _tag: 'original' } })));
  await older;
  const selected = reader.getSnapshot();
  assert.equal(selected.kind === 'ready' && selected.context.start_recipient.agent_id, 'agent/B');
  reader.close();
  assert.deepEqual(reader.getSnapshot(), { kind: 'loading' });
});
