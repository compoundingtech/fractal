import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { SidebarJournal } from './sidebarJournal.ts';
import { createWriteState, writeReducer } from './sidebarWriteState.ts';
const vectors = JSON.parse(readFileSync(new URL('../../fixtures/clients/sidebar/behavior-v2.json', import.meta.url), 'utf8'));
const scope = { owner: 'owner-a', container: 'container-a', endpoint: 'endpoint-a', host: 'host-a', projection: '1' };
const base = n => ({ scope, snapshot: { id: `snapshot-${n}`, host_id: scope.host, store_index: n, projection_version: scope.projection, created_at: 'created' }, revision: `revision-${n}`, ready: true });
const id = n => ({ actor: 'actor-a', action_id: `action-${n}`, idempotency_key: `key-${n}` });
const fixture = vectors.cases.find(c => c.events.some(e => e.type === 'ProjectAndPlanV2'));
const intent = fixture.events[0].moves[0], memberPlan = fixture.checkpoints[0].moves[0].edit;
const compound = { folders: [{ op: 'folder.rename', id: 'folder-a', name: 'Folder' }], memberships: memberPlan.memberships };
const plan = (edit = memberPlan, fence = base(1)) => ({ type: 'Plan', base: fence, plan: edit, folder: id(1), membership: id(2) });
const event = (type, n = 2) => ({ type, identity: id(n) });
const setup = (initial, storageFailure = false) => {
  const { next_delivery, ...durable } = initial ?? {};
  let raw = initial === undefined ? null : JSON.stringify({ version: 1, state: durable });
  const sent = [], effects = [];
  const storage = { getItem: async () => raw, setItem: async (_key, value) => { if (storageFailure) throw new Error('storage'); raw = value; }, removeItem: async () => { raw = null; } };
  const host = new SidebarJournal(storage, scope, writeReducer, async (effect, state) => { effects.push(effect); if (effect.type === 'Send') sent.push(structuredClone(state.request)); });
  return { host, storage, sent, effects, raw: () => raw };
};
const staged = (edit = memberPlan) => { const state = createWriteState(scope, intent); writeReducer.reduce(state, plan(edit)); return state; };

test('real shared behavior plans cross durable barrier before sending and replay identically', async () => {
  for (const vector of vectors.cases) for (const checkpoint of vector.checkpoints) for (const move of checkpoint.moves) {
    if (move._tag !== 'PlannedV2') continue;
    const state = createWriteState(scope, vector.events[0].moves[0]), env = setup();
    await env.host.dispatch(state, plan(move.edit));
    assert.equal(state.status, 'Sending'); assert.equal(env.sent.length, 1);
    const exact = structuredClone(state.request);
    for (let n = 0; n < 3; n++) { await env.host.dispatch(state, event('Unknown')); await env.host.dispatch(state, { type: 'Replay' }); assert.deepEqual(env.sent.at(-1), exact); }
    await env.host.dispatch(state, event('Ack', 99)); assert.equal(state.status, 'Sending');
    await env.host.dispatch(state, event('Ack')); assert.equal(state.status, 'AwaitingObservation');
    await env.host.dispatch(state, event('Observed')); assert.equal(state.status, 'Observed'); assert.equal(env.raw(), null);
    const count = env.sent.length; await env.host.dispatch(state, event('Observed')); await env.host.dispatch(state, { type: 'Replay' }); assert.equal(env.sent.length, count);
  }
});
test('Rust staged vector requires folder completion persistence and fresh post-folder fence', async () => {
  const state = createWriteState(scope, intent), env = setup(); await env.host.dispatch(state, plan(compound));
  assert.deepEqual(state.request.operations, { Folders: compound.folders });
  await env.host.dispatch(state, event('Ack', 1));
  await env.host.dispatch(state, { type: 'Fresh', base: base(2), operations: compound.memberships }); assert.equal(env.sent.length, 1);
  await env.host.dispatch(state, event('Observed', 1)); assert.equal(state.status, 'AwaitingFreshBase'); assert.equal(state.folder_completed, true);
  for (const stale of [base(1), { ...base(2), revision: base(1).revision }, { ...base(2), ready: false }, { ...base(2), scope: { ...scope, owner: 'other' } }, { ...base(2), snapshot: { ...base(2).snapshot, store_index: 0 } }]) {
    await env.host.dispatch(state, { type: 'Fresh', base: stale, operations: compound.memberships }); assert.equal(state.status, 'AwaitingFreshBase');
  }
  const recovered = setup(JSON.parse(env.raw()).state), restored = await recovered.host.restore(); await recovered.host.resume(restored);
  assert.equal(recovered.sent.length, 0);
  await recovered.host.dispatch(restored.state, { type: 'Fresh', base: base(2), operations: compound.memberships });
  assert.deepEqual(recovered.sent[0].operations, { Memberships: compound.memberships }); assert.deepEqual(recovered.sent[0].fence, base(2));
  await recovered.host.dispatch(restored.state, event('Observed')); assert.equal(restored.state.status, 'Observed');
});
test('partial application refuses deleted destination and retries without reusing identity', async () => {
  const state = staged(compound); writeReducer.reduce(state, event('Persisted', 1)); writeReducer.reduce(state, event('Observed', 1)); writeReducer.reduce(state, event('CompletionPersisted', 1));
  const env = setup(state), restored = await env.host.restore(); await env.host.resume(restored);
  await env.host.dispatch(restored.state, { type: 'Fresh', base: base(2), operations: { error: 'DestinationMissing' } });
  assert.equal(restored.state.status, 'PartiallyApplied'); assert.equal(restored.state.refusal, 'DestinationMissing'); assert.equal(env.sent.length, 0);
  const restart = setup(JSON.parse(env.raw()).state), partial = await restart.host.restore(); await restart.host.resume(partial); assert.equal(restart.sent.length, 0);
  await restart.host.dispatch(partial.state, { type: 'Retry' });
  await restart.host.dispatch(partial.state, plan(compound, base(2))); assert.equal(partial.state.refusal, 'InvalidPlan');
});
test('all Rust journal crash statuses recover with correct persistence or read effects', async () => {
  for (const status of ['Intent', 'AwaitingPersistence', 'Sending', 'UnknownDelivery', 'AwaitingObservation', 'AwaitingCompletionPersistence', 'AwaitingFreshBase', 'Observed', 'Refused', 'PartiallyApplied']) {
    const state = staged(compound); state.status = status;
    const env = setup(state), restored = await env.host.restore(); assert.deepEqual(restored.state.request, state.request); assert.equal(env.sent.length, 0);
    await env.host.resume(restored);
    assert.equal(env.sent.length, ['AwaitingPersistence', 'Sending', 'UnknownDelivery'].includes(status) ? 1 : 0);
    if (status === 'AwaitingCompletionPersistence') assert.equal(restored.state.status, 'AwaitingFreshBase');
    if (status === 'Observed') assert.equal(env.raw(), null);
    if (status === 'Intent') assert.deepEqual(env.effects, ['ReadBase', 'Replan']);
  }
});
test('codec strips nested non-durable fields and rejects invalid durable fields through journal', async () => {
  const original = staged(compound); original.request.credentials = 'secret'; original.request.fence.snapshot.connection = 'secret';
  const encoded = writeReducer.encode(original); assert.equal(JSON.stringify(encoded).includes('secret'), false);
  assert.deepEqual(writeReducer.decode(encoded), encoded);
  const mutations = [
    s => { s.extra = true; }, s => { delete s.used; }, s => { s.used = {}; }, s => { s.used[0].actor = 1; },
    s => { s.scope.owner = false; }, s => { s.status = 'Other'; }, s => { s.refusal = 'Other'; }, s => { s.folder_completed = 0; },
    s => { s.intent.node._tag = 'Other'; }, s => { s.intent.parent = null; }, s => { s.intent.before = false; },
    s => { s.request.identity.action_id = null; }, s => { s.dependent.idempotency_key = []; }, s => { s.request.fence.ready = 1; },
    s => { s.request.fence.revision = false; }, s => { s.request.fence.snapshot.created_at = 0; }, s => { s.request.fence.snapshot.published_at = false; },
    s => { s.request.operations.Folders[0].name = false; }, s => { s.request.operations = { Memberships: [{ op: 'place', member: 1, bucket: null, key: 'a0' }] }; },
    ...[-1, 0.5, Number.MAX_SAFE_INTEGER + 1, '1', null].map(value => s => { s.request.fence.snapshot.store_index = value; }),
  ];
  for (const mutate of mutations) { const state = structuredClone(encoded); mutate(state); const restored = await setup(state).host.restore(); assert.deepEqual(restored, { error: { type: 'InvalidRecord' } }); }
  for (const value of [0, Number.MAX_SAFE_INTEGER]) { const state = structuredClone(encoded); state.request.fence.snapshot.store_index = value; assert.ok((await setup(state).host.restore()).state); }
  for (const value of [null, 'published']) { const state = structuredClone(encoded); state.request.fence.snapshot.published_at = value; const restored = await setup(state).host.restore(); assert.equal(restored.state.request.fence.snapshot.published_at, value); assert.equal(writeReducer.encode(restored.state).request.fence.snapshot.published_at, value); }
  assert.equal('published_at' in writeReducer.encode((await setup(encoded).host.restore()).state).request.fence.snapshot, false);
  for (const field of ['scope', 'request']) { const state = structuredClone(encoded); (field === 'scope' ? state.scope : state.request.fence.scope).owner = 'other'; assert.deepEqual(await setup(state).host.restore(), { error: { type: 'ScopeMismatch' } }); }
});
test('storage failures never send and preserve partial-application classification', async () => {
  for (const completed of [false, true]) { const state = createWriteState(scope, intent); state.folder_completed = completed; const env = setup(undefined, true); await assert.rejects(env.host.dispatch(state, plan()), /storage/); assert.equal(state.status, completed ? 'PartiallyApplied' : 'Refused'); assert.equal(state.refusal, 'Storage'); assert.equal(env.sent.length, 0); }
});
test('initial empty or invalid plans durably refuse without a request or send', async () => {
  for (const edit of [{ folders: [], memberships: [] }, { folders: [{ op: 'retire' }], memberships: [] }]) {
    const state = createWriteState(scope, intent), env = setup();
    await env.host.dispatch(state, plan(edit));
    assert.equal(state.status, 'Refused'); assert.equal(state.refusal, 'InvalidPlan'); assert.equal(state.request, null);
    assert.equal(env.sent.length, 0); assert.deepEqual(env.effects, ['ReadBase']);
    assert.deepEqual(JSON.parse(env.raw()).state, writeReducer.encode(state));
    const restart = setup(JSON.parse(env.raw()).state), restored = await restart.host.restore(); await restart.host.resume(restored);
    assert.equal(restored.state.status, 'Refused'); assert.equal(restart.sent.length, 0);
  }
});
test('invalid runtime durable values fail the persistence barrier before send', async () => {
  const state = createWriteState(scope, intent), env = setup(), invalid = base(1);
  invalid.snapshot.store_index = Number.MAX_SAFE_INTEGER + 1;
  await assert.rejects(env.host.dispatch(state, plan(memberPlan, invalid)), /Invalid sidebar write state/);
  assert.equal(env.sent.length, 0); assert.equal(env.raw(), null); assert.equal(state.refusal, 'Storage');
});
test('storage failures match both current request identity and persistence phase', () => {
  const state = staged(compound), pending = writeReducer.storageFailed(state, 'Pending');
  for (const phase of ['Completion', 'Removal']) assert.deepEqual(writeReducer.reduce(state, writeReducer.storageFailed(state, phase)), []);
  writeReducer.reduce(state, event('Persisted', 1)); writeReducer.reduce(state, event('Ack', 1));
  assert.deepEqual(writeReducer.reduce(state, pending), []); assert.equal(state.status, 'AwaitingObservation');
  writeReducer.reduce(state, event('Observed', 1));
  for (const failure of [pending, writeReducer.storageFailed(state, 'Removal')]) assert.deepEqual(writeReducer.reduce(state, failure), []);
  assert.equal(state.status, 'AwaitingCompletionPersistence');
  const completion = writeReducer.storageFailed(state, 'Completion');
  assert.deepEqual(writeReducer.reduce(state, completion), ['PersistPending', 'ReadBase']); assert.equal(state.status, 'PartiallyApplied');
  const fresh = staged(); assert.deepEqual(writeReducer.reduce(fresh, writeReducer.storageFailed(fresh, 'Pending')), ['PersistPending', 'ReadBase']); assert.equal(fresh.status, 'Refused');
});
test('host captures failed-write identity and phase before awaiting storage', async () => {
  for (const dependent of [false, true]) {
    const state = staged(compound), env = setup();
    env.storage.setItem = async () => {
      writeReducer.reduce(state, event('Persisted', 1)); writeReducer.reduce(state, event('Ack', 1)); writeReducer.reduce(state, event('Observed', 1));
      if (dependent) { writeReducer.reduce(state, event('CompletionPersisted', 1)); writeReducer.reduce(state, { type: 'Fresh', base: base(2), operations: compound.memberships }); }
      throw new Error('late pending failure');
    };
    await assert.rejects(env.host.resume({ state, effects: ['PersistPending'] }), /late pending failure/);
    assert.equal(state.status, dependent ? 'AwaitingPersistence' : 'AwaitingCompletionPersistence');
    assert.equal(state.request.identity.action_id, id(dependent ? 2 : 1).action_id); assert.equal(state.refusal, null); assert.equal(env.sent.length, 0);
  }
});
test('removal failure surfaces without undoing authoritative observation', async () => {
  const state = staged(), env = setup();
  writeReducer.reduce(state, event('Persisted')); writeReducer.reduce(state, event('Observed'));
  env.storage.removeItem = async () => { throw new Error('removal failure'); };
  await assert.rejects(env.host.resume({ state, effects: ['PersistCompletion'] }), /removal failure/);
  assert.equal(state.status, 'Observed'); assert.equal(state.refusal, null); assert.equal(env.sent.length, 0);
});
