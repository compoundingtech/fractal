import assert from 'node:assert/strict';
import test from 'node:test';
import { FractalClient } from '../../fractal-client/index.ts';
import { followQuickAccess } from '../quickAccessFollow.ts';
import { quickAccessSelection } from '../quickAccess.ts';

const { owner, subject } = quickAccessSelection('person/example/session/native');
const snapshot = { id: 'snapshot/test/7/proof', host_id: 'host/test', store_index: 7, projection_version: 'client-projection.v0', created_at: '2026-01-01T00:00:00Z' };
const identity = { id: 'action/pin', idempotency_key: 'pins-edit-000001', fence: { snapshot_id: snapshot.id, subject_revisions: {} } };
function setup({ absent = false, corrupt = false, actionStatus = 'completed', actionReply }  = {}) {
  let exists = !absent, frame, ended;
  const actions = [], states = [], subscriptions = [], reads = [], streams = [];
  const row = (member, key) => ({ id: member, container: subject, member, position: { bucket: null, key }, revision: 'claim/place' });
  const client = new FractalClient({ baseUrl: 'http://fixture.invalid', fetchImpl: async (input, init) => {
    const url = new URL(String(input)); reads.push(url.pathname); let value;
    if (url.pathname.endsWith('/capabilities')) value = { session_actor: owner + '/session/native', limits: { max_page_items: 200 }, capabilities: [{ id: 'arrangements', version: 2, state: 'granted' }, { id: 'control.arrangements', version: 1, state: 'granted' }] };
    else if (url.pathname.endsWith('/actions')) { const action = JSON.parse(String(init.body)); actions.push(action); exists = true; const reply = actionReply?.(action, actions.length); if (reply instanceof Response) return reply; if (reply instanceof Error) throw reply; value = { kind: 'action-result', action_id: action.id, status: reply ?? actionStatus }; }
    else if (!exists) return Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'not-found', message: 'absent', retryable: false }, { status: 404 });
    else if (url.pathname.endsWith('/memberships')) value = { kind: 'page', collection: 'ordered-memberships', items: [row(url.searchParams.has('cursor') && !corrupt ? 'mission/example' : 'agent/example', url.searchParams.has('cursor') ? 'a1' : 'a0')], page: { has_more: !url.searchParams.has('cursor'), next_cursor: url.searchParams.has('cursor') ? null : 'second' } };
    else value = { id: subject, owner, kind: 'arrangement', deleted: false, revision: 'claim/create', body: { version: 2, name: { value: 'Pins', revision: 'claim/create' }, folders: {} } };
    return Response.json({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot, value });
  } });
  client.collectionStream = async options => {
    frame = options.onFrame; ended = options.onEnd; streams.push(options);
    return { subscribeArrangements: (...args) => subscriptions.push(args), subscribeOrderedMemberships: (...args) => subscriptions.push(args), close: () => {} };
  };
  return { client, actions, states, subscriptions, reads, streams, onState: state => states.push(state), retire: () => { exists = false; }, end: () => ended?.(), changed: () => frame?.({ kind: 'changes', id: 'pins-members' }) };
}

test('native-compatible follow reads all pages and follows outside-window changes', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  assert.deepEqual(h.states.at(-1).members.map(item => item.member), ['agent/example', 'mission/example']);
  assert.equal(h.subscriptions.length, 2);
  const before = h.reads.length; h.changed();
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(h.reads.length >= before + 3, 'membership invalidation rereads arrangement and every page');
  await follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]);
  assert.equal(h.actions[0].type, 'arrangement.membership.edit');
  assert.ok(h.states.filter(state => state.phase === 'pending').every(state => state.editable === false));
  assert.equal(h.states.at(-1).editable, true);
  follow.close();
});

test('collection creation needs explicit reader readiness and distinct action identities', async () => {
  const h = setup({ absent: true });
  let follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  await assert.rejects(follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }]));
  assert.equal(h.actions.length, 0); follow.close();
  follow = await followQuickAccess({ gateway: h.client, onState: h.onState, v2ReadersReady: true });
  await assert.rejects(follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], identity));
  follow.close();
  follow = await followQuickAccess({ gateway: h.client, onState: h.onState, v2ReadersReady: true });
  await follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' });
  assert.deepEqual(h.actions.map(item => item.type), ['arrangement.edit', 'arrangement.membership.edit']);
  follow.close();
});

test('a disconnected or retired collection never becomes silently writable', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  h.end();
  await assert.rejects(follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]));
  assert.equal(h.states.at(-1).phase, 'unavailable'); follow.close();
  const r = setup(), retired = await followQuickAccess({ gateway: r.client, onState: r.onState });
  r.retire(); retired.refresh();
  await new Promise(resolve => setImmediate(resolve));
  assert.match(r.states.at(-1).detail, /Restoring removed Pins/);
  await assert.rejects(retired.edit(identity, [{ op: 'remove', member: 'agent/example' }])); retired.close();
});

test('duplicate page members are refused rather than rendered twice', async () => {
  const h = setup({ corrupt: true }), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  assert.equal(h.states.at(-1).phase, 'unavailable');
  assert.match(h.states.at(-1).detail, /Duplicate Pins membership/); follow.close();
});

// React Native has AbortSignal.aborted but need not implement throwIfAborted.
test('accepts the native AbortSignal surface and aborts without opening a stream', async () => {
  const h = setup();
  const controller = new AbortController();
  controller.signal.throwIfAborted = undefined;
  const follow = await followQuickAccess({ gateway: h.client, onState: h.onState, signal: controller.signal });
  follow.close(); controller.abort();
  await assert.rejects(followQuickAccess({ gateway: h.client, onState: h.onState, signal: controller.signal }));
});

test('an accepted but incomplete action is not reported as a completed edit', async () => {
  const h = setup({ actionReply: (_, count) => count === 1 ? 'accepted' : 'completed' });
  const follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  await assert.rejects(follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]), /not confirmed/);
  assert.equal(h.states.at(-1).phase, 'unconfirmed');
  assert.equal(h.states.at(-1).editable, false);
  assert.deepEqual(h.states.at(-1).members.map(item => item.member), ['agent/example', 'mission/example']);
  await follow.refresh(); assert.equal(h.states.at(-1).phase, 'unconfirmed');
  await follow.checkEdit(); assert.deepEqual(h.actions[0], h.actions[1]);
  assert.equal(h.states.at(-1).editable, true); follow.close();
});


test('explicit refresh after stream loss reconnects without replaying an edit', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  h.end();
  assert.equal(h.states.at(-1).phase, 'unavailable');
  assert.equal(h.streams.length, 1, 'a stream end does not automatically reconnect');
  await assert.rejects(follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]));
  await follow.refresh();
  assert.equal(h.streams.length, 2);
  h.streams[1].onFrame({ kind: 'snapshot', id: 'pins-members' });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(h.states.at(-1).phase, 'synced');
  assert.equal(h.states.at(-1).editable, true);
  assert.deepEqual(h.states.at(-1).members.map(item => item.member), ['agent/example', 'mission/example']);
  assert.deepEqual(h.subscriptions.slice(-2).map(args => args[0]), ['pins-layout', 'pins-members']);
  assert.equal(h.actions.length, 0, 'reconnecting never replays an edit');
  h.streams[0].onEnd(new Error('late event from the old stream'));
  assert.equal(h.states.at(-1).phase, 'synced', 'old stream cannot invalidate the replacement');
  await follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]);
  assert.equal(h.actions.length, 1); follow.close();
});

test('a late interrupted HTTP read cannot overwrite a refreshed session', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  const original = h.client.arrangementsGet.bind(h.client), started = Promise.withResolvers(), release = Promise.withResolvers();
  h.client.arrangementsGet = async (...args) => { started.resolve(); await release.promise; return original(...args); };
  const oldRead = follow.refresh(); await started.promise;
  h.end(); h.client.arrangementsGet = original;
  await follow.refresh();
  release.resolve(); await oldRead;
  assert.equal(h.states.at(-1).phase, 'synced');
  assert.equal(h.states.at(-1).editable, true);
  assert.equal(h.streams.length, 2); follow.close();
});

test('reopening a broken follow does not forget that the Pins arrangement was removed', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState, v2ReadersReady: true });
  h.end(); h.retire(); await follow.refresh();
  assert.equal(h.states.at(-1).phase, 'unavailable');
  assert.equal(h.states.at(-1).editable, false);
  assert.match(h.states.at(-1).detail, /Restoring removed Pins/);
  await assert.rejects(follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'create-after-loss' }));
  assert.equal(h.actions.length, 0); follow.close();
});


test('refresh never replaces the session while an edit still has an unresolved outcome', async () => {
  const h = setup(), follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  const started = Promise.withResolvers(), release = Promise.withResolvers(), original = h.client.arrangementMembershipEdit.bind(h.client);
  h.client.arrangementMembershipEdit = async (...args) => { started.resolve(); await release.promise; return original(...args); };
  const editing = follow.edit(identity, [{ op: 'remove', member: 'agent/example' }]); await started.promise;
  h.end(); await follow.refresh();
  assert.equal(h.streams.length, 1);
  await assert.rejects(follow.edit({ ...identity, id: 'action/second' }, [{ op: 'remove', member: 'mission/example' }]));
  release.resolve(); await editing; await follow.refresh();
  assert.equal(h.streams.length, 2);
  assert.equal(h.states.at(-1).editable, true);
  assert.equal(h.actions.length, 1); follow.close();
});


test('lost membership receipt stays locked across reads and reconnect; explicit check reuses immutable request', async () => {
  const h = setup({ actionReply: (_, count) => count === 1 ? new TypeError('response lost after commit') : 'completed' });
  const follow = await followQuickAccess({ gateway: h.client, onState: h.onState });
  const key = structuredClone(identity), ops = [{ op: 'remove', member: 'agent/example' }];
  await assert.rejects(follow.edit(key, ops), /response lost/);
  const original = structuredClone(h.actions[0]);
  const readMembers = h.client.arrangementsMemberships.bind(h.client);
  h.client.arrangementsMemberships = async (...args) => {
    const response = await readMembers(...args);
    response.value.items = response.value.items.filter(row => row.member !== 'agent/example');
    return response;
  };
  key.id = 'action/changed'; key.fence.subject_revisions.changed = 'claim/changed'; ops[0].member = 'mission/changed';
  assert.equal(h.states.at(-1).phase, 'unconfirmed');
  await follow.refresh(); h.end(); await follow.refresh();
  assert.deepEqual(h.states.at(-1).members.map(row => row.member), ['mission/example'], 'observed applied membership is not a delivery receipt');
  assert.equal(h.states.at(-1).phase, 'unconfirmed');
  assert.equal(h.states.at(-1).editable, false);
  assert.equal(h.actions.length, 1, 'reads and reconnect never resend');
  await assert.rejects(follow.edit(identity, [{ op: 'remove', member: 'mission/example' }]));
  await follow.checkEdit();
  assert.deepEqual(h.actions[1], original);
  assert.equal(h.states.at(-1).phase, 'synced');
  assert.equal(h.states.at(-1).editable, true); follow.close();
});

test('an explicit retired refusal remains terminal even if this client never saw the collection', async () => {
  const h = setup({ absent: true, actionReply: () => Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'arrangement-retired', message: 'retired', retryable: false, details: { applied: 'none' } }, { status: 409 }) });
  const follow = await followQuickAccess({ gateway: h.client, onState: h.onState, v2ReadersReady: true });
  await assert.rejects(follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' }));
  h.retire(); h.end(); await follow.refresh();
  assert.equal(h.states.at(-1).phase, 'unavailable');
  assert.equal(h.states.at(-1).editable, false);
  assert.match(h.states.at(-1).detail, /Restoring removed Pins/); follow.close();
});


test('create-race validation recovery is explicit GET-only and truthfully described', async () => {
  const h = setup({ absent: true, actionReply: action => action.type === 'arrangement.edit'
    ? Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'arrangement-exists', message: 'already exists', retryable: false }, { status: 409 }) : 'completed' });
  const follow = await followQuickAccess({ gateway: h.client, onState: h.onState, v2ReadersReady: true });
  const original = h.client.arrangementsGet.bind(h.client); let lost = false;
  h.client.arrangementsGet = async (...args) => { if (!lost) { lost = true; throw new TypeError('adoption GET lost'); } return original(...args); };
  await assert.rejects(follow.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' }));
  assert.equal(h.states.at(-1).phase, 'unconfirmed'); assert.equal(h.states.at(-1).editable, false);
  assert.match(h.states.at(-1).detail, /create request was not applied/);
  await follow.refresh(); assert.equal(h.actions.length, 1, 'Refresh never sends the retained member');
  assert.equal(h.states.at(-1).phase, 'unconfirmed');
  await follow.checkEdit();
  assert.deepEqual(h.actions.map(a => a.type), ['arrangement.edit', 'arrangement.membership.edit']);
  assert.equal(h.actions[1].id, identity.id); assert.equal(h.states.at(-1).editable, true); follow.close();
});
