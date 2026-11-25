import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { EnvelopeOf, FractalClient, RequestHistoryItem, RequestHistoryPage } from '../../clients/typescript/fractal-client/index.ts';
import { REQUEST_HISTORY_LIMIT, RequestHistoryReader, requestHistoryKey, requestHistoryRelations, requestHistoryRows } from './requestHistory.ts';

// Typed client presentation fixtures, not fabricated live-server observations.
const old: RequestHistoryItem = {
  key: { source_id: 'step-run/review/person', episode: 'generation/review:1:1' }, complete: true,
  payload: { availability: 'available', title: 'Original review', text: 'Approve abc', request: null, references: [], provenance: { kind: 'authored-episode', mission_revision: 'revision/abc', opening_claim_id: 'claim/open-abc' } },
  state: { availability: 'available', status: 'completed', answers: [{ summary: 'Approved abc' }] },
};
const later: RequestHistoryItem = {
  ...old, key: { ...old.key, episode: 'generation/review:2:3' },
  payload: { availability: 'unavailable', reason: 'historical-interpolation-unavailable' },
  state: { availability: 'available', status: 'ready', answers: [] },
};
const page = (items: RequestHistoryItem[] = [old, later]): RequestHistoryPage => ({
  items, boundary_items: [], complete: true, next_cursor: null, source_support: 'person-step',
  source_episodes: [{ source_id: old.key.source_id, episodes: items.map(item => item.key) }], relations: [],
});
const envelope = (value: RequestHistoryPage): EnvelopeOf<RequestHistoryPage> => ({
  api_version: 'fractal.client.v0', request_id: 'request/history-unit', value,
  snapshot: { id: 'snapshot/history-unit', host_id: 'host/unit', created_at: '2026-11-17T00:00:00Z', projection_version: 'client-projection.v0', store_index: 1 },
});

test('history preserves exact identity, independent status, answer and explicit predecessor reason', () => {
  const related = { ...later, key: { source_id: 'step-run/other-ask', episode: 'claim/ask' } };
  const history = page([old, related]);
  history.relations = [{ predecessor: old.key, request: related.key, reason: 'Independent follow-up approval' }];
  assert.deepEqual(requestHistoryRows(history), [old, related]);
  assert.deepEqual(requestHistoryRelations(history), history.relations);
  assert.equal(old.state.availability === 'available' && old.state.status, 'completed');
  assert.equal(related.state.availability === 'available' && related.state.status, 'ready');
});

test('same-source episodes remain separate without manufacturing predecessor relations', () => {
  const history = page();
  assert.notEqual(requestHistoryKey(old.key), requestHistoryKey(later.key));
  assert.equal(requestHistoryRows(history).length, 2);
  assert.deepEqual(requestHistoryRelations(history), []);
  assert.deepEqual(history.source_episodes[0]?.episodes, [old.key, later.key]);
});

test('unavailable original payload never substitutes current or neighboring episode text', () => {
  for (const reason of ['retained-definition-unavailable', 'opening-unavailable', 'historical-interpolation-unavailable'] as const) {
    const unavailable: RequestHistoryItem = { ...later, payload: { availability: 'unavailable', reason } };
    const row = requestHistoryRows(page([unavailable]))[0]!;
    assert.deepEqual(row.payload, { availability: 'unavailable', reason });
    assert.ok(!JSON.stringify(row).includes('Approve abc'));
  }
});

test('privacy hides reasons with absent endpoints and retains authorized boundary navigation', () => {
  const history = page([later]);
  history.relations = [{ predecessor: old.key, request: later.key, reason: 'PRIVATE REASON' }];
  assert.deepEqual(requestHistoryRelations(history), []);
  history.boundary_items = [old];
  assert.deepEqual(requestHistoryRelations(history), history.relations);
  assert.deepEqual(requestHistoryRows(history).map(item => item.key), [old.key, later.key]);
});

test('bounded pagination replaces reauthorized pages and rejects repeated cursors', async () => {
  const first = { ...page([old]), next_cursor: 'opaque-next' };
  const second = { ...page([later]), next_cursor: 'opaque-next' };
  const calls: unknown[] = [];
  const client: Pick<FractalClient, 'threadRequestHistory'> = {
    threadRequestHistory: async (id, options, request) => {
      calls.push([id, options]);
      assert.ok(request?.signal);
      return envelope(calls.length === 1 ? first : second);
    },
  };
  const reader = new RequestHistoryReader(client, 'thread/history');
  await reader.load();
  await reader.more();
  const result = reader.getSnapshot();
  assert.equal(result.kind, 'ready');
  if (result.kind !== 'ready') return;
  assert.deepEqual(result.page.items, [later]);
  assert.deepEqual(calls, [
    ['thread/history', { limit: REQUEST_HISTORY_LIMIT }],
    ['thread/history', { limit: REQUEST_HISTORY_LIMIT, cursor: 'opaque-next' }],
  ]);
  await reader.more();
  assert.equal(calls.length, 2);
  assert.match(JSON.stringify(reader.getSnapshot()), /Refresh history/);
  reader.close();
  assert.deepEqual(reader.getSnapshot(), { kind: 'loading' });
});

test('revoked or failed history reads remove all prior private content', async () => {
  let reads = 0;
  const client: Pick<FractalClient, 'threadRequestHistory'> = {
    threadRequestHistory: async () => {
      if (reads++ === 0) return envelope(page([old]));
      throw new Error('PRIVATE transport detail');
    },
  };
  const reader = new RequestHistoryReader(client, 'thread/history');
  await reader.load();
  const pending = reader.load();
  assert.deepEqual(reader.getSnapshot(), { kind: 'loading' });
  await pending;
  assert.equal(reader.getSnapshot().kind, 'error');
  assert.ok(!JSON.stringify(reader.getSnapshot()).includes('Original review'));
  assert.ok(!JSON.stringify(reader.getSnapshot()).includes('PRIVATE'));
});

test('released reader cannot publish a delayed authorized response', async () => {
  const { promise, resolve } = Promise.withResolvers<EnvelopeOf<RequestHistoryPage>>();
  const client: Pick<FractalClient, 'threadRequestHistory'> = {
    threadRequestHistory: () => promise,
  };
  const reader = new RequestHistoryReader(client, 'thread/history');
  const pending = reader.load();
  reader.close();
  resolve(envelope(page()));
  await pending;
  assert.deepEqual(reader.getSnapshot(), { kind: 'loading' });
});

test('refresh supersedes an in-flight page without resurrecting older private rows', async () => {
  const first = Promise.withResolvers<EnvelopeOf<RequestHistoryPage>>();
  const second = Promise.withResolvers<EnvelopeOf<RequestHistoryPage>>();
  let reads = 0;
  const client: Pick<FractalClient, 'threadRequestHistory'> = {
    threadRequestHistory: () => reads++ === 0 ? first.promise : second.promise,
  };
  const reader = new RequestHistoryReader(client, 'thread/history');
  const pendingFirst = reader.load();
  const pendingSecond = reader.load();
  second.resolve(envelope(page([])));
  await pendingSecond;
  first.resolve(envelope(page([old])));
  await pendingFirst;
  const result = reader.getSnapshot();
  assert.equal(result.kind, 'ready');
  if (result.kind === 'ready') assert.deepEqual(result.page.items, []);
});
