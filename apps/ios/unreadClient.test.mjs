import assert from 'node:assert/strict';
import { test } from 'node:test';
import { API_VERSION, FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { unreadClient, unreadSupported } from './unreadClient.ts';
import { reminderFromSummary } from './unreadVisit.ts';

const snapshot = { id: 'snapshot/1', host_id: 'host/a', store_index: 1, projection_version: 'client-projection.v0' };
const thread = { id: 'thread/a', agent_id: 'agent/a', revision: 'revision/a', title_revision: 'title/a', title: 'A', state: 'open', unread: true, unread_reminder: { anchor_message_id: 'message/a', marks: [{ anchor_message_id: 'message/a', mark_id: 'signed/opaque-mark' }] } };
const bound = { id: 'action/original', idempotency_key: 'retry/original', fence: { snapshot_id: 'snapshot/1', revisions: {} } };
const setup = (response, grants = {}) => {
  const requests = [], deliveries = [], reads = [];
  const client = new FractalClient({ baseUrl: 'https://gateway.invalid', fetchImpl: async (input, init) => {
    const request = { url: new URL(input), body: init?.body ? JSON.parse(init.body) : undefined };
    if (request.url.pathname.endsWith('/capabilities'))
      return Response.json({ api_version: API_VERSION, snapshot, value: { limits: { max_page_items: 100 } } });
    requests.push(request);
    const value = await response(request, requests.length);
    return value instanceof Response ? value : Response.json({ api_version: API_VERSION, snapshot, value });
  } });
  const ports = {
    supported: () => unreadSupported({ capabilities: [{ id: 'threads-v1', version: grants.version ?? 3, state: grants.state ?? 'granted' }] }),
    canRead: () => grants.read !== false, canWrite: () => grants.write !== false,
    read: async (_thread, ids) => { reads.push(ids); return { how: 'sent' }; },
    submit: async (target, kind, send) => { deliveries.push({ target, kind }); await send(client, bound); return { how: 'sent' }; },
  };
  const service = unreadClient(client, ports);
  return { service, api: service.forThread(thread), requests, deliveries, reads };
};
const refused = code => Response.json({ api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: 'request/a', code, message: 'Unavailable', retryable: false, details: {} }, { status: 404 });

test('mark and clear use generated requests and preserve captured opaque IDs, never receipt IDs', async () => {
  const s = setup(() => ({ operation_id: 'action-result/NOT-A-MARK', target_id: thread.id }));
  const observed = ['signed/first', 'signed/second'];
  await s.api.actions.edit({ kind: 'mark', anchor_message: 'message/a', observed_marks: observed });
  await s.api.actions.edit({ kind: 'clear', observed_marks: observed });
  assert.deepEqual(s.requests.map(r => r.body.parameters), [
    { target_id: thread.id, anchor_message_id: 'message/a', observed_marks: observed },
    { target_id: thread.id, observed_marks: observed },
  ]);
  assert.deepEqual(s.requests.map(r => r.body.type), ['thread.mark-unread', 'thread.clear-unread']);
  for (const { body } of s.requests) {
    assert.equal(body.id, bound.id); assert.equal(body.idempotency_key, bound.idempotency_key);
    assert.equal(body.parameters.observed_marks.includes('action-result/NOT-A-MARK'), false);
  }
  assert.deepEqual(reminderFromSummary(thread).marks.map(m => m.mark_id), ['signed/opaque-mark']);
});

test('around uses only the generated seek query and never acknowledges', async () => {
  const s = setup(() => ({ anchor: { message_id: 'message/a', state: 'current' }, items: [], next_cursor: null }));
  const result = await s.api.actions.seek('message/a');
  assert.equal(result.state, 'current');
  assert.equal(s.requests[0].url.searchParams.get('around'), 'message/a');
  assert.equal(s.requests[0].url.searchParams.has('cursor'), false);
  assert.deepEqual(s.deliveries, []); assert.deepEqual(s.reads, []);
});

test('unavailable and explicit pending discard the server body; generic refusal is unavailable', async () => {
  for (const state of ['unavailable', 'pending']) {
    const s = setup(() => ({ anchor: { message_id: 'message/a', state }, items: [], next_cursor: null }));
    const result = await s.api.actions.seek('message/a');
    assert.equal(result.state, state); assert.equal('page' in result, false);
    assert.deepEqual(s.reads, []);
  }
  const s = setup(() => refused('thread-unavailable'));
  assert.deepEqual(await s.api.actions.seek('message/a'), { state: 'unavailable', anchor: { message_id: 'message/a', state: 'unavailable' } });
});

test('seek validates canonical responses and keeps refusal/network errors typed', async () => {
  for (const value of [{ items: [], next_cursor: null }, { anchor: { message_id: 'other', state: 'current' }, items: [], next_cursor: null }, { anchor: { message_id: 'message/a', state: 'maybe' }, items: [], next_cursor: null }]) {
    const s = setup(() => value);
    assert.deepEqual(await s.api.actions.seek('message/a'), { state: 'error', error: { kind: 'invalid-response' } });
  }
  assert.deepEqual((await setup(() => refused('forbidden')).api.actions.seek('message/a')).error, { kind: 'refused', code: 'forbidden' });
  assert.equal((await setup(() => { throw new Error('offline'); }).api.actions.seek('message/a')).error.kind, 'failed');
});

test('marked list drains empty cursor pages with exact generated selector and decodes summaries', async () => {
  const s = setup((_request, n) => n === 1 ? { items: [], next_cursor: 'opaque/next' } : { items: [thread], next_cursor: null });
  assert.deepEqual(await s.service.listMarked(), { kind: 'items', items: [thread] });
  assert.deepEqual(s.requests.map(({ url }) => [url.searchParams.get('marked_unread'), url.searchParams.get('cursor')]), [['true', null], ['true', 'opaque/next']]);
  const malformed = setup(() => ({ items: [{ ...thread, unread_reminder: { marks: [{ mark_id: 3 }], anchor_message_id: 'message/a' } }], next_cursor: null }));
  assert.deepEqual(await malformed.service.listMarked(), { kind: 'error', items: [], error: { kind: 'read-failed', reason: { kind: 'invalid-response' } } });
  assert.equal(reminderFromSummary({ unread_reminder: { marks: [{ mark_id: 'wrong' }], anchor_message_id: 'message/a' } }), undefined);
});

test('list errors never become empty, preserve partial results and enforce paging bound', async () => {
  const s = setup((_request, n) => n === 1 ? { items: [thread], next_cursor: 'next' } : refused('forbidden'));
  assert.deepEqual(await s.service.listMarked(), { kind: 'error', items: [thread], error: { kind: 'read-failed', reason: { kind: 'refused', code: 'forbidden' } } });
  const cycling = setup(() => ({ items: [], next_cursor: 'repeat' }));
  assert.equal((await cycling.service.listMarked()).error.kind, 'repeated-cursor');
  const bounded = setup((_request, n) => ({ items: [], next_cursor: String(n) }));
  assert.equal((await bounded.service.listMarked(2)).error.kind, 'page-bound');
});

test('read-only refuses before any mutation or request and reevaluates revoked grants', async () => {
  const grants = { write: true, read: true }, s = setup(() => assert.fail('must not fetch'), grants);
  // Revoke after the interface was created, as when an already-open menu outlives
  // its grant. Checks must happen at dispatch, not bind time.
  grants.write = false; grants.read = false;
  assert.equal((await s.api.actions.read(['message/a'])).how, 'failed');
  assert.equal((await s.api.actions.edit({ kind: 'mark', anchor_message: 'message/a', observed_marks: [] })).how, 'failed');
  assert.equal((await s.api.actions.edit({ kind: 'clear', observed_marks: [] })).how, 'failed');
  assert.equal((await s.api.actions.seek('message/a')).error.code, 'forbidden');
  assert.equal((await s.service.listMarked()).error.reason.code, 'forbidden');
  assert.deepEqual(s.requests, []); assert.deepEqual(s.deliveries, []); assert.deepEqual(s.reads, []);
});

test('threads-v1 v2 never sends marked_unread, around, mark or clear, including an already-open v3 menu', async () => {
  const grants = { version: 3 }, s = setup(() => assert.fail('v2 must not send new wire operations'), grants);
  const openMenu = s.api;
  grants.version = 2;
  assert.equal(s.service.forThread(thread).state, 'pending-api', 'v2 offers no reminder actions');
  assert.deepEqual(await s.service.listMarked(), {
    kind: 'error', items: [], error: { kind: 'read-failed', reason: { kind: 'refused', code: 'upgrade-required' } },
  });
  assert.deepEqual(await openMenu.actions.seek('message/a'), { state: 'error', error: { kind: 'refused', code: 'upgrade-required' } });
  for (const edit of [{ kind: 'mark', anchor_message: 'message/a', observed_marks: ['opaque/one'] }, { kind: 'clear', observed_marks: ['opaque/one'] }]) {
    const result = await openMenu.actions.edit(edit);
    assert.equal(result.how, 'failed'); assert.match(result.error, /Upgrade/);
  }
  assert.deepEqual(s.requests, []); assert.deepEqual(s.deliveries, []);
});

test('reminder capability negotiation requires a granted threads-v1 version >=3', () => {
  assert.equal(unreadSupported(undefined), false);
  assert.equal(unreadSupported({ capabilities: [] }), false);
  for (const version of [1, 2, 3, 4]) for (const state of ['granted', 'denied']) {
    assert.equal(unreadSupported({ capabilities: [{ id: 'threads-v1', version, state }] }), version >= 3 && state === 'granted');
  }
});
