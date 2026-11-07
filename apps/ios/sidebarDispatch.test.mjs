import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { SidebarJournal } from './sidebarJournal.ts';
import { createWriteState, writeReducer } from './sidebarWriteState.ts';
import { sidebarWriteRequest, sidebarWriteResult, sendSidebarWrite } from './sidebarDispatch.ts';
const scope = { owner: 'person/test', container: 'arrangement/test', endpoint: 'endpoint/test', host: 'host/test', projection: '1' };
const identity = n => ({ actor: 'actor/test', action_id: `action/${n}`, idempotency_key: `key/${n}` });
const base = n => ({ scope, snapshot: { id: `snapshot/${n}`, host_id: scope.host, store_index: n, projection_version: '1', created_at: 'created' }, revision: `revision/${n}`, ready: true });
const intent = { node: { _tag: 'Subject', subject: 'agent/test' }, parent: { _tag: 'Root' }, before: null };
const members = [{ op: 'place', member: 'agent/test', bucket: null, key: 'a0' }];
const plan = { type: 'Plan', base: base(1), plan: { folders: [], memberships: members }, folder: identity(1), membership: identity(2) };
const state = () => createWriteState(scope, intent);
const request = () => { const s = state(); writeReducer.reduce(s, plan); return s.request; };
const receipt = (id = 'action/2', status = 'completed') => ({ value: { action_id: id, status } });
const failure = (code, applied) => new ClientError({ api_version: 'fractal.client.v0', error_version: 'fractal.client.error.v0', request_id: 'request/test', code, message: 'refusal', retryable: true, details: applied ? { applied } : {} }, 409);
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const setup = (initial = null, run = async () => {}, limits) => {
  let raw = initial;
  const storage = { getItem: async () => raw, setItem: async (_key, value) => { raw = value; }, removeItem: async () => { raw = null; } };
  return { storage, host: new SidebarJournal(storage, scope, writeReducer, run, limits), raw: () => raw };
};
test('native wire payload preserves every operation, identity and both fences', () => {
  const r = request(), wire = sidebarWriteRequest(r);
  assert.deepEqual(wire, { api_version: 'fractal.client.v0', id: 'action/2', idempotency_key: 'key/2', type: 'arrangement.membership.edit', fence: { snapshot_id: 'snapshot/1', subject_revisions: { 'arrangement/test': 'revision/1' } }, parameters: { subject: 'arrangement/test', owner: 'person/test', operations: members } });
  r.operations = { Folders: [{ op: 'folder.create', id: 'folder/test', name: 'Test', parent: null, key: 'a0' }, { op: 'folder.move', id: 'folder/test', parent: 'folder/other', key: 'a1' }] };
  const folders = sidebarWriteRequest(r); assert.equal(folders.type, 'arrangement.edit'); assert.deepEqual(folders.parameters.operations, r.operations.Folders);
  folders.parameters.operations[0].name = 'Changed'; assert.equal(r.operations.Folders[0].name, 'Test'); assert.equal('actor' in wire, false);
});
test('delivery allowlist never turns ambiguous failures into safe refusal', () => {
  const r = request();
  for (const delivery of ['First', 'Replay']) for (const status of ['accepted', 'completed']) assert.deepEqual(sidebarWriteResult(r, delivery, receipt('action/2', status)), { type: 'Ack', identity: r.identity });
  assert.equal(sidebarWriteResult(r, 'First', receipt('action/2', 'rejected')).reason, 'Conflict');
  for (const [code, reason] of [['stale-fence', 'StaleFence'], ['forbidden', 'Unauthorized'], ['arrangement-owner-forbidden', 'Unauthorized'], ['membership-owner-forbidden', 'Unauthorized'], ['arrangement-folder-deleted', 'DestinationMissing'], ['arrangement-retired', 'DestinationMissing'], ['not-found', 'DestinationMissing'], ['invalid-membership-operations', 'InvalidPlan']]) assert.equal(sidebarWriteResult(r, 'First', { error: failure(code) }).reason, reason);
  for (const result of [receipt('action/other'), receipt('action/2', 'future'), { error: new Error('timeout') }, { error: failure('internal') }, { error: failure('idempotency-conflict') }, { error: failure('stale-fence', 'unknown') }]) assert.equal(sidebarWriteResult(r, 'First', result).type, 'Unknown');
  assert.equal(sidebarWriteResult(r, 'First', { error: failure('idempotency-conflict', 'none') }).reason, 'Conflict');
  assert.equal(sidebarWriteResult(r, 'First', { error: failure('internal', 'none') }).reason, 'InvalidPlan');
  for (const code of ['forbidden', 'stale-fence', 'invalid-membership-operations', 'internal', 'idempotency-conflict']) for (const applied of [undefined, 'none', 'unknown']) assert.deepEqual(sidebarWriteResult(r, 'Replay', { error: failure(code, applied) }), { type: 'Unknown', identity: r.identity });
});
test('real journal dispatch replays production idempotency and refreshes compound fences', async () => {
  const calls = [], client = { submitAction: async (wire, options) => { calls.push(structuredClone(wire)); assert.ok(options.signal); return receipt(wire.id); } };
  const env = setup(null, async (effect, s, signal) => { if (effect.type === 'Send') await sendSidebarWrite(client, s.request, effect.delivery, 'actor/test', signal); });
  const s = state(); await env.host.dispatch(s, { ...plan, plan: { folders: [{ op: 'folder.rename', id: 'folder/test', name: 'Test' }], memberships: members } });
  await env.host.dispatch(s, { type: 'Unknown', identity: identity(1) });
  const restarted = setup(env.raw(), async (effect, current, signal) => { if (effect.type === 'Send') await sendSidebarWrite(client, current.request, effect.delivery, 'actor/test', signal); });
  const restored = await restarted.host.restore(); await restarted.host.resume(restored);
  assert.deepEqual(calls[1], calls[0]);
  await restarted.host.dispatch(restored.state, { type: 'Observed', identity: identity(1) });
  await restarted.host.dispatch(restored.state, { type: 'Fresh', base: base(2), operations: members });
  assert.equal(calls[2].id, 'action/2'); assert.equal(calls[2].fence.snapshot_id, 'snapshot/2'); assert.deepEqual(calls[2].fence.subject_revisions, { 'arrangement/test': 'revision/2' });
  let invoked = false; const refusal = await sendSidebarWrite({ submitAction: async () => { invoked = true; } }, request(), 'First', 'actor/other', new AbortController().signal); assert.equal(invoked, false); assert.equal(refusal.reason, 'Unauthorized');
});
test('replay refusals preserve compound progress and never resend automatically', async () => {
  for (const restored of [false, true]) {
    const responses = [], deliveries = [], requests = [];
    const run = async (effect, current, signal) => {
      if (effect.type !== 'Send') return;
      deliveries.push(effect.delivery); requests.push(structuredClone(current.request));
      responses.push(await sendSidebarWrite({ submitAction: async () => ({ error: failure('forbidden', 'none') }) }, current.request, effect.delivery, 'actor/test', signal));
    };
    let env = setup(null, run), s = state();
    await env.host.dispatch(s, { ...plan, plan: { folders: [{ op: 'folder.rename', id: 'folder/test', name: 'Test' }], memberships: members } });
    assert.equal(deliveries[0], 'First'); assert.equal(responses.pop().type, 'Refused');
    // The first response was lost; the caller can only record uncertain delivery.
    await env.host.dispatch(s, { type: 'Unknown', identity: identity(1) });
    const exact = structuredClone(s.request);
    assert.equal(requests.length, 1);
    assert.equal('next_delivery' in JSON.parse(env.raw()).state, false);
    if (restored) {
      await env.host.close(); env = setup(env.raw(), run);
      const recovery = await env.host.restore(); s = recovery.state; await env.host.resume(recovery);
    } else await env.host.dispatch(s, { type: 'Replay' });
    assert.equal(deliveries.at(-1), 'Replay'); assert.deepEqual(requests.at(-1), exact);
    const unresolved = responses.pop(); assert.deepEqual(unresolved, { type: 'Unknown', identity: exact.identity });
    await env.host.dispatch(s, unresolved);
    assert.equal(s.status, 'UnknownDelivery'); assert.equal(s.refusal, null); assert.deepEqual(s.request, exact);
    assert.equal(requests.length, 2); assert.deepEqual(s.dependent, identity(2));
    await env.host.dispatch(s, { type: 'Observed', identity: identity(1) });
    assert.equal(s.folder_completed, true);
    await env.host.dispatch(s, { type: 'Fresh', base: base(2), operations: members });
    assert.equal(deliveries.at(-1), 'First'); assert.deepEqual(s.request.identity, identity(2));
  }
});
test('connection close cancels in-flight transport and bars late storage send', async () => {
  const entered = deferred(), blocked = deferred(); let sends = 0, signal;
  const env = setup(null, async (_effect, _s, selected) => { signal = selected; entered.resolve(); await blocked.promise; });
  const task = env.host.dispatch(state(), plan); await entered.promise; await env.host.close(); await assert.rejects(task, /closed/); assert.equal(signal.aborted, true); blocked.resolve();
  await assert.rejects(env.host.dispatch(state(), plan), /closed/);
  for (const phase of ['Pending', 'Completion', 'Removal']) {
    const disk = setup(null, async () => { sends++; }), writing = deferred(), release = deferred(), current = state();
    if (phase !== 'Pending') {
      writeReducer.reduce(current, phase === 'Completion' ? { ...plan, plan: { folders: [{ op: 'folder.rename', id: 'folder/test', name: 'Test' }], memberships: members } } : plan);
      const identity = current.request.identity;
      writeReducer.reduce(current, { type: 'Persisted', identity }); writeReducer.reduce(current, { type: 'Observed', identity });
      if (phase === 'Removal') writeReducer.reduce(current, { type: 'CompletionPersisted', identity });
    }
    const method = phase === 'Removal' ? 'removeItem' : 'setItem', persist = disk.storage[method];
    disk.storage[method] = async (...args) => { writing.resolve(); await release.promise; await persist(...args); };
    const pending = phase === 'Pending' ? disk.host.dispatch(current, plan) : disk.host.resume({ state: current, effects: [phase === 'Completion' ? 'PersistCompletion' : 'PersistRemoval'] });
    await writing.promise;
    const unchanged = structuredClone(current); let closed = false;
    const closing = disk.host.close().then(() => { closed = true; });
    await assert.rejects(pending, /closed/); assert.equal(closed, false);
    release.resolve(); await closing;
    assert.deepEqual(current, unchanged); assert.equal(sends, 0);
    disk.storage[method] = persist;
    const replacement = new SidebarJournal(disk.storage, scope, writeReducer, async () => {});
    await replacement.dispatch(state(), { ...plan, membership: identity(3) });
    const ownRecord = disk.raw(); assert.equal(JSON.parse(ownRecord).state.request.identity.action_id, 'action/3');
    await disk.host.close(); assert.equal(disk.raw(), ownRecord); await replacement.close();
  }
});
test('queue bounds and deadline stop wedged host without starting dependent work', async () => {
  const entered = deferred(), blocked = deferred();
  const env = setup(null, async () => { entered.resolve(); await blocked.promise; }, { maxQueued: 1, timeoutMs: 100 });
  const pending = env.host.dispatch(state(), plan); await entered.promise;
  await assert.rejects(env.host.dispatch(state(), plan), /queue full/); await assert.rejects(pending, /closed/); blocked.resolve();
  await assert.rejects(env.host.restore(), /closed/);
});
test('restore blocks edit events until exact recovered state resumes; u32 envelope validated', async () => {
  const s = state(); writeReducer.reduce(s, plan);
  const env = setup(JSON.stringify({ version: 1, state: writeReducer.encode(s) })), restored = await env.host.restore();
  await assert.rejects(env.host.dispatch(state(), plan), /Resume/);
  await assert.rejects(env.host.resume({ state: structuredClone(restored.state), effects: restored.effects }), /identity mismatch/);
  await env.host.resume(restored); await env.host.dispatch(restored.state, { type: 'Ack', identity: identity(2) });
  for (const version of [-1, 0.5, 4294967296, '1', null]) assert.deepEqual(await setup(JSON.stringify({ version, state: s })).host.restore(), { error: { type: 'InvalidRecord' } });
});
