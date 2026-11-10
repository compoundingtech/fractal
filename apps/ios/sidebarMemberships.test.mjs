import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SidebarMemberships } from './sidebarMemberships.ts';
import { sidebarMembershipFrame, sidebarMembershipFailure, readSidebarMemberships } from './feed.ts';
const scope = { owner: 'person/test', container: 'arrangement/test', endpoint: 'https://gateway.invalid', host: 'host/test', projection: 'v1' };
const frontier = (changed_index = 1, live_count = 1, epoch = 'epoch-a') => ({ epoch, changed_index, live_count });
const page = (ids = ['a'], next, cut = frontier()) => ({ frontier: cut, snapshot: { id: 'snapshot/test', host_id: scope.host, store_index: 1, projection_version: scope.projection, created_at: 'created', expires_at: 'expires' }, members: ids.map(id => ({ id, member: id, container: scope.container, revision: 'claim/test', position: { bucket: null, key: 'a0' } })), has_more: next !== undefined, next });
const refresh = (state, generation = 1, selected = scope) => { const effects = state.reduce({ type: 'Refresh', scope: selected, generation }); assert.equal(effects[0].type, 'SubscribeAndRead'); return effects[0].token; };
const deliver = (state, token, value) => state.reduce({ type: 'Page', token, page: value });
const metadata = (state, token, value) => state.reduce({ type: 'Metadata', token, frontier: value });
const ready = () => { const state = new SidebarMemberships(); const token = refresh(state); deliver(state, token, page()); return { state, token }; };

test('all page partitions publish only complete sets despite unrelated snapshot writes', () => {
  for (let count = 0; count < 12; count++) for (let width = 1; width <= 12; width++) {
    const state = new SidebarMemberships(), token = refresh(state), cut = frontier(1, count);
    metadata(state, token, cut);
    for (let offset = 0; offset < Math.max(count, 1); offset += width) {
      const ids = Array.from({ length: Math.min(width, count - offset) }, (_, i) => `member-${offset + i}`);
      const next = offset + width < count ? `cursor-${offset}` : undefined;
      const input = page(ids, next, cut); input.snapshot.store_index += offset; input.snapshot.id += offset;
      const effects = deliver(state, token, input);
      if (next) { assert.equal(state.complete, undefined); assert.deepEqual(effects, [{ type: 'Read', token, cursor: next }]); }
      else assert.deepEqual(effects, [{ type: 'PersistBase' }]);
    }
    assert.equal(state.status, 'Ready'); assert.equal(state.stale, false); assert.equal(state.complete.members.length, count);
    metadata(state, token, frontier(0, 0)); assert.equal(state.stale, false);
  }
});
test('malformed pages refuse and retain the last complete base', () => {
  for (const mutate of [p => { p.next = undefined; }, p => { p.has_more = false; }, p => { p.next = ''; }, p => { p.members = []; }, p => { p.members.push(p.members[0]); }, p => { p.members[0].container = 'other'; }, p => { p.frontier.live_count = 0; }, p => { p.frontier.live_count = -1; }, p => { p.frontier.changed_index = NaN; }, p => { p.frontier.epoch = ''; }]) {
    const { state } = ready(), base = state.complete, token = refresh(state), input = page(['b'], 'cursor', frontier(2, 2));
    mutate(input); deliver(state, token, input); assert.equal(state.status, 'Refused'); assert.equal(state.complete, base); assert.equal(state.stale, true);
  }
  for (const mode of ['repeat', 'duplicate', 'short', 'count-conflict']) {
    const { state } = ready(), token = refresh(state); deliver(state, token, page(['b'], 'cursor', frontier(2, 3)));
    deliver(state, token, page([mode === 'duplicate' ? 'b' : 'c'], mode === 'repeat' ? 'cursor' : undefined, frontier(2, mode === 'count-conflict' ? 4 : 3)));
    assert.equal(state.status, 'Refused'); assert.equal(state.complete.members[0].member, 'a');
  }
});
test('supersession, incomparable epochs and host changes discard whole attempts', () => {
  for (const mode of ['index', 'epoch', 'host', 'projection', 'metadata']) {
    const state = new SidebarMemberships(), token = refresh(state); deliver(state, token, page(['a'], 'cursor', frontier(2, 2)));
    const input = page(['b'], undefined, frontier(2, 2));
    if (mode === 'index') input.frontier.changed_index++;
    if (mode === 'epoch') input.frontier.epoch = 'epoch-b';
    if (mode === 'host') input.snapshot.host_id = 'other';
    if (mode === 'projection') input.snapshot.projection_version = 'other';
    const effects = mode === 'metadata' ? metadata(state, token, frontier(3, 2)) : deliver(state, token, input);
    assert.equal(effects[0].type, 'SubscribeAndRead'); assert.equal(state.complete, undefined); assert.ok(effects[0].token.attempt > token.attempt);
    assert.deepEqual(deliver(state, token, page()), []);
  }
  const state = new SidebarMemberships(), token = refresh(state); metadata(state, token, frontier(5, 1));
  const next = deliver(state, token, page([], undefined, frontier(1, 0, 'epoch-b')))[0].token;
  deliver(state, next, page([], undefined, frontier(1, 0, 'epoch-b'))); assert.equal(state.status, 'Ready');
});
test('metadata joins: newer before first page, older during paging, conflicts paging and ready', () => {
  const state = new SidebarMemberships(), token = refresh(state); metadata(state, token, frontier(5, 1));
  assert.equal(deliver(state, token, page(['a'], undefined, frontier(4, 1)))[0].type, 'SubscribeAndRead');
  for (const paging of [false, true]) {
    const { state, token } = ready();
    if (paging) { const current = refresh(state); deliver(state, current, page(['b'], 'cursor', frontier(1, 2))); metadata(state, current, frontier(1, 3)); }
    else metadata(state, token, frontier(1, 3));
    assert.equal(state.status, 'Refused'); assert.equal(state.complete.members[0].member, 'a');
  }
  const fresh = new SidebarMemberships(), current = refresh(fresh); deliver(fresh, current, page(['a'], 'cursor', frontier(3, 2)));
  assert.deepEqual(metadata(fresh, current, frontier(2, 1)), []); deliver(fresh, current, page(['b'], undefined, frontier(3, 2))); assert.equal(fresh.status, 'Ready');
  assert.equal(metadata(fresh, current, frontier(4, 2))[0].type, 'SubscribeAndRead'); assert.equal(fresh.stale, true);
});
test('scope/generation/attempt isolation, restored scope, disconnect and bounded resets', () => {
  const { state, token: old } = ready(), base = state.complete, token = refresh(state, 2);
  for (const field of ['owner', 'container', 'endpoint', 'host', 'projection', 'generation', 'attempt']) {
    const wrong = structuredClone(token); if (field in wrong.scope) wrong.scope[field] += '-other'; else wrong[field]++;
    for (const event of [{ type: 'Page', page: page() }, { type: 'Metadata', frontier: frontier() }, { type: 'Expired' }, { type: 'Refused' }, { type: 'Disconnect' }]) assert.deepEqual(state.reduce({ ...event, token: wrong }), []);
  }
  assert.deepEqual(deliver(state, old, page()), []); state.reduce({ type: 'Disconnect' }); assert.equal(state.complete, base); assert.equal(state.status, 'Disconnected');
  const restored = new SidebarMemberships(base); assert.equal(restored.stale, true); let next = refresh(restored);
  for (let attempt = 0; attempt < 3; attempt++) { const effects = restored.reduce(sidebarMembershipFailure(next, { status: 409, response: { code: 'membership-frontier-reset' } })); if (effects.length) next = effects[0].token; }
  assert.equal(restored.status, 'Exhausted'); assert.equal(restored.complete, base); refresh(restored); assert.equal(restored.status, 'Paging');
  refresh(restored, 3, { ...scope, owner: 'person/other' }); assert.equal(restored.complete, undefined);
});
test('unwired frame conversion preserves metadata-only changes, requires epoch, maps failures', () => {
  const state = new SidebarMemberships(), token = refresh(state);
  const frame = { kind: 'changes', id: 'sidebar', collection: 'ordered-memberships', membership: { container: scope.container, changed_index: 7, live_count: 8 }, snapshot: page().snapshot, upserts: [], removes: [], order: [], has_more: false };
  assert.deepEqual(sidebarMembershipFrame(token, 'sidebar', frame, 'epoch-a'), { type: 'Metadata', token, frontier: frontier(7, 8) });
  assert.deepEqual(sidebarMembershipFrame(token, 'sidebar', { ...frame, kind: 'snapshot', items: [] }, 'epoch-a').frontier, frontier(7, 8));
  for (const [kind, retryable, expected] of [['error', false, 'Refused'], ['error', true, 'Expired'], ['resync', true, 'Expired']]) assert.equal(sidebarMembershipFrame(token, 'sidebar', { kind, id: 'sidebar', code: 'unavailable', message: 'unavailable', retryable }, 'epoch-a').type, expected);
  assert.equal(sidebarMembershipFrame(token, 'other', frame, 'epoch-a'), undefined);
  assert.equal(sidebarMembershipFrame(token, 'sidebar', frame).type, 'Refused');
  assert.equal(sidebarMembershipFrame(token, 'sidebar', { ...frame, membership: { ...frame.membership, container: 'other' } }, 'epoch-a').type, 'Refused');
  assert.equal(sidebarMembershipFrame(token, 'sidebar', { ...frame, snapshot: { ...frame.snapshot, host_id: 'other' } }, 'epoch-a').type, 'Expired');
  for (const status of [401, 403, 409, 500]) assert.equal(sidebarMembershipFailure(token, { status, response: { code: 'other' } }).type, 'Refused');
  assert.deepEqual(sidebarMembershipFailure(token, {}), { type: 'Disconnect', token });
});
test('unsupported and terminal collection-v1 failures retain stale base without retry loops', () => {
  for (const [status, code, type] of [[404, 'not-found', 'Unsupported'], [422, 'unsupported-membership-paging-contract', 'Unsupported'], [422, 'invalid-membership-cursor', 'InvalidCursor'], [500, 'membership-frontier-inconsistent', 'FrontierInconsistent']]) {
    const { state, token: old } = ready(), base = state.complete, token = refresh(state, 2);
    const failure = sidebarMembershipFailure(token, { status, response: { code } });
    assert.deepEqual(failure, { type, token });
    assert.deepEqual(state.reduce({ ...failure, token: old }), []); assert.equal(state.status, 'Paging');
    assert.deepEqual(state.reduce(failure), []);
    assert.equal(state.status, type); assert.equal(state.stale, true); assert.equal(state.complete, base);
    assert.deepEqual(deliver(state, token, page()), []); assert.deepEqual(state.reduce(failure), []); assert.equal(state.status, type);
  }
});
test('both collection-v1 409 pairs restart whole attempts, never continuations', () => {
  for (const [status, code] of [[409, 'membership-frontier-reset'], [409, 'membership-collection-changed']]) {
    const { state } = ready(), base = state.complete, token = refresh(state);
    deliver(state, token, page(['b'], 'cursor', frontier(2, 2)));
    const event = sidebarMembershipFailure(token, { status, response: { code } });
    assert.deepEqual(event, { type: 'Expired', token });
    const [effect] = state.reduce(event); assert.equal(effect.type, 'SubscribeAndRead'); assert.ok(effect.token.attempt > token.attempt);
    assert.equal(state.complete, base); assert.deepEqual(deliver(state, token, page()), []);
    deliver(state, effect.token, page(['c'], undefined, frontier(3, 1))); assert.deepEqual(state.complete.members.map(row => row.member), ['c']);
  }
  for (const [status, code] of [[410, 'membership-frontier-reset'], [410, 'membership-collection-changed'], [409, 'page-cursor-expired'], [410, 'page-cursor-expired'], [409, 'invalid-membership-cursor'], [422, 'membership-frontier-inconsistent'], [500, 'unsupported-membership-paging-contract'], [422, 'other']]) assert.equal(sidebarMembershipFailure(refresh(new SidebarMemberships()), { status, response: { code } }).type, 'Refused');
});
test('first reads await subscription; continuations do not resubscribe', async () => {
  const state = new SidebarMemberships(), token = refresh(state), calls = [];
  let joined;
  const subscribe = async () => { calls.push('subscribe'); await new Promise(resolve => { joined = resolve; }); calls.push('joined'); };
  const read = async (received, cursor) => { assert.deepEqual(received, token); calls.push(cursor ?? 'first'); return page(); };
  const pending = readSidebarMemberships({ type: 'SubscribeAndRead', token }, subscribe, read);
  assert.deepEqual(calls, ['subscribe']); joined(); state.reduce(await pending);
  assert.deepEqual(calls, ['subscribe', 'joined', 'first']); assert.equal(state.status, 'Ready');
  await readSidebarMemberships({ type: 'Read', token, cursor: 'cursor-a' }, subscribe, read);
  assert.deepEqual(calls, ['subscribe', 'joined', 'first', 'cursor-a']);
  await assert.rejects(readSidebarMemberships({ type: 'SubscribeAndRead', token }, async () => { throw new Error('unavailable'); }, read), /unavailable/);
  assert.equal(calls.length, 4);
});
