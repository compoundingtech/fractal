import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SidebarJournal } from './sidebarJournal.ts';
const scope = { owner: 'owner-a', container: 'container-a', endpoint: 'gateway-a', host: 'host-a', projection: '1' };
const state = (status = 'AwaitingPersistence', dependent = false) => ({ scope, status, folder_completed: false, dependent, intent: { member: 'member-a', bucket: 'folder-a' }, request: { identity: { actor: 'actor-a', action_id: 'action-a', idempotency_key: 'key-a' }, fence: { scope, revision: 'revision-a', snapshot: 'snapshot-a' }, operations: [{ type: 'place', member: 'member-a', bucket: 'folder-a', key: 'a0' }] } });
// Deliberately small test reducer mirrors Rust edits.rs lifecycle stages, not a planner.
const reducer = {
  encode: s => ({ scope: s.scope, status: s.status, folder_completed: s.folder_completed, dependent: s.dependent, intent: s.intent, request: s.request }),
  decode: s => s && typeof s === 'object' && typeof s.status === 'string' && s.scope && s.request?.identity && s.request.fence?.scope && typeof s.folder_completed === 'boolean' ? s : undefined,
  persisted: (s, completion) => ({ type: completion ? 'CompletionPersisted' : 'Persisted', identity: structuredClone(s.request.identity) }),
  storageFailed: () => ({ type: 'StorageFailed' }),
  reduce(s, e) {
    if (e.type === 'StorageFailed') { s.status = s.folder_completed ? 'PartiallyApplied' : 'Refused'; return ['PersistPending', 'ReadBase']; }
    if (e.identity && JSON.stringify(e.identity) !== JSON.stringify(s.request.identity)) return [];
    if (e.type === 'Stage') { s.status = 'AwaitingPersistence'; return ['PersistPending']; }
    if (e.type === 'Persisted' && s.status === 'AwaitingPersistence') { s.status = 'Sending'; return [{ type: 'Send', delivery: 'Replay' }]; }
    if (e.type === 'Replay' && s.status === 'UnknownDelivery') { s.status = 'Sending'; return [{ type: 'Send', delivery: 'Replay' }]; }
    if (e.type === 'Ack' && s.status === 'Sending') { s.status = 'AwaitingObservation'; return ['PersistPending', 'ReadBase']; }
    if (e.type === 'Observed' && ['Sending', 'UnknownDelivery', 'AwaitingObservation'].includes(s.status)) { s.folder_completed = s.dependent; s.status = 'AwaitingCompletionPersistence'; return ['PersistCompletion']; }
    if (e.type === 'CompletionPersisted' && s.status === 'AwaitingCompletionPersistence') { s.status = s.dependent ? 'AwaitingFreshBase' : 'Observed'; return s.dependent ? ['ReadBase'] : ['PersistRemoval']; }
    return [];
  },
};
const setup = (values = new Map()) => {
  const storage = { getItem: async key => values.get(key) ?? null, setItem: async (key, value) => { values.set(key, value); }, removeItem: async key => { values.delete(key); } };
  const effects = [], sent = [];
  const host = () => new SidebarJournal(storage, scope, reducer, async (effect, s) => { effects.push(effect); if (effect.type === 'Send') sent.push(structuredClone(s.request)); });
  return { values, storage, effects, sent, host };
};
const seed = (env, s) => env.values.set('record', JSON.stringify({ version: 1, state: s }));
const seedHost = env => { env.storage.getItem = async () => [...env.values.values()][0] ?? null; return env.host(); };

test('all crash boundaries restore exact immutable request with a new persistence barrier', async () => {
  for (const status of ['AwaitingPersistence', 'Sending', 'UnknownDelivery', 'AwaitingObservation']) {
    const env = setup(), original = state(status); seed(env, original);
    const host = seedHost(env), restored = await host.restore();
    assert.deepEqual(restored.state.request, original.request); assert.equal(env.sent.length, 0);
    await host.resume(restored);
    assert.deepEqual(env.sent, status === 'AwaitingObservation' ? [] : [original.request]);
    assert.equal(restored.state.status, status === 'AwaitingObservation' ? status : 'Sending');
    assert.deepEqual(env.effects, status === 'AwaitingObservation' ? ['ReadBase'] : [{ type: 'Send', delivery: 'Replay' }]);
  }
});
test('pending and completion writes block their acknowledgements until durable', async () => {
  const env = setup(); let release;
  env.storage.setItem = () => new Promise(resolve => { release = resolve; });
  const host = env.host(), s = state(); const work = host.dispatch(s, { type: 'Stage' });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(s.status, 'AwaitingPersistence'); assert.equal(env.sent.length, 0);
  release(); await work; assert.equal(env.sent.length, 1);
  const completion = host.dispatch(s, { type: 'Observed' });
  await new Promise(resolve => setImmediate(resolve)); assert.equal(s.status, 'AwaitingCompletionPersistence');
  release(); await completion; assert.equal(s.status, 'Observed'); assert.equal(env.sent.length, 1);
});
test('folder completion and dependent membership pending survive restart without folder replay', async () => {
  for (const status of ['AwaitingCompletionPersistence', 'AwaitingFreshBase', 'PartiallyApplied']) {
    const env = setup(), s = state(status, true); s.folder_completed = true; seed(env, s);
    const host = seedHost(env), restored = await host.restore(); await host.resume(restored);
    assert.equal(restored.state.folder_completed, true); assert.equal(env.sent.length, 0);
    assert.equal(restored.state.status, status === 'AwaitingCompletionPersistence' ? 'AwaitingFreshBase' : status);
  }
  const env = setup(), s = state('AwaitingPersistence'); s.folder_completed = true; s.request.identity.action_id = 'membership-action'; seed(env, s);
  const host = seedHost(env), restored = await host.restore(); await host.resume(restored);
  assert.deepEqual(env.sent, [s.request]); assert.equal(restored.state.folder_completed, true);
});
test('completed and observed requests are never resent even when removal fails', async () => {
  for (const status of ['AwaitingCompletionPersistence', 'Observed']) {
    const env = setup(); seed(env, state(status)); const host = seedHost(env), restored = await host.restore();
    env.storage.removeItem = async () => { throw new Error('disk'); };
    await assert.rejects(host.resume(restored), /disk/); assert.equal(env.sent.length, 0);
    const again = await host.restore(); assert.ok(['PersistCompletion', 'PersistRemoval'].includes(again.effects[0]));
    assert.equal(env.sent.length, 0);
  }
});
test('write failures reject and stop send or dependent completion, without retry loops', async () => {
  for (const completion of [false, true]) {
    const env = setup(), s = state(completion ? 'Sending' : 'Intent', completion);
    env.storage.setItem = async () => { throw new Error('disk'); };
    await assert.rejects(env.host().dispatch(s, { type: completion ? 'Observed' : 'Stage' }), /disk/);
    assert.equal(s.status, completion ? 'PartiallyApplied' : 'Refused'); assert.equal(env.sent.length, 0);
  }
});
test('unknown versions refuse before decode; corrupt and foreign records are explicit errors', async () => {
  const env = setup(), host = seedHost(env); let decoded = 0;
  const checking = new SidebarJournal(env.storage, scope, { ...reducer, decode: s => { decoded++; return reducer.decode(s); } }, async () => {});
  for (const version of [0, 2, 4294967295]) {
    env.values.set('record', JSON.stringify({ version, state: { future: true } }));
    assert.deepEqual(await checking.restore(), { error: { type: 'Version', version } });
  }
  assert.equal(decoded, 0);
  for (const raw of ['not json', '{}', '{"version":1,"state":{}}']) { env.values.set('record', raw); assert.deepEqual(await host.restore(), { error: { type: 'InvalidRecord' } }); }
  for (const field of Object.keys(scope)) for (const request of [false, true]) {
    const s = structuredClone(state()); if (request) s.request.fence.scope[field] += '-other'; else s.scope[field] += '-other';
    seed(env, s); assert.deepEqual(await host.restore(), { error: { type: 'ScopeMismatch' } });
  }
});
test('serialized edits retain each exact write and exclude runtime credentials', async () => {
  const env = setup(), s = state('Intent'); s.transport = { bearer: 'test-private-token' };
  const writes = [], original = env.storage.setItem;
  env.storage.setItem = async (key, raw) => { writes.push(raw); await original(key, raw); };
  const host = env.host(); await Promise.all([host.dispatch(s, { type: 'Stage' }), host.dispatch(s, { type: 'Ack' })]);
  assert.deepEqual(writes.map(raw => JSON.parse(raw).state.status), ['AwaitingPersistence', 'AwaitingObservation']);
  assert.ok(writes.every(raw => !raw.includes('test-private-token') && !raw.includes('transport')));
  assert.equal(env.sent.length, 1); assert.deepEqual(await setup().host().restore(), { empty: true });
});
test('switching host scope refuses all restored effects and unknown-delivery replay', async () => {
  for (const field of Object.keys(scope)) for (const requestOnly of [false, true]) {
    const env = setup(); seed(env, state('UnknownDelivery'));
    const restored = await seedHost(env).restore(), calls = [];
    const selected = { ...scope, [field]: `${scope[field]}-other` };
    const host = new SidebarJournal(env.storage, selected, reducer, async effect => { calls.push(effect); });
    if (requestOnly) restored.state.scope = selected;
    for (const effect of [{ type: 'Send', delivery: 'Replay' }, 'ReadBase', 'Replan', 'PersistPending', 'PersistCompletion', 'PersistRemoval']) {
      await assert.rejects(host.resume({ state: restored.state, effects: [effect] }), /scope mismatch/);
    }
    restored.state.status = 'UnknownDelivery';
    await assert.rejects(host.dispatch(restored.state, { type: 'Replay' }), /scope mismatch/);
    assert.deepEqual(calls, []);
    assert.equal(env.values.size, 1);
  }
});
