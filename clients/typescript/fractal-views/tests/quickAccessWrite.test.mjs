import assert from 'node:assert/strict';
import test from 'node:test';
import { FractalClient } from '../../fractal-client/index.ts';
import { QuickAccessWrite } from '../quickAccessWrite.ts';
import { quickAccessSelection } from '../quickAccess.ts';

const selection = quickAccessSelection('person/example/session/native');
const snapshot = { id: 'snapshot/test/7/proof', host_id: 'host/test', store_index: 7, projection_version: 'client-projection.v0', created_at: '2026-01-01T00:00:00Z' };
const identity = { id: 'action/pin', idempotency_key: 'pins-edit-000001', fence: { snapshot_id: snapshot.id, subject_revisions: {} } };
const create = { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' };
const operations = [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }];
const refused = details => Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'stale-fence', message: 'refused', retryable: false, details }, { status: 409 });
function setup(reply, readReply) {
  const actions = [], reads = [];
  const client = new FractalClient({ baseUrl: 'http://fixture.invalid', fetchImpl: async (input, init) => {
    let value;
    const path = new URL(String(input)).pathname;
    if (path.endsWith('/capabilities')) value = { session_actor: selection.owner + '/session/native', capabilities: [], limits: { max_page_items: 200 } };
    else if (path.endsWith('/actions')) {
      const action = JSON.parse(String(init.body)); actions.push(action);
      const result = reply(action, actions.length);
      if (result instanceof Error) throw result;
      if (result instanceof Response) return result;
      value = { kind: 'action-result', action_id: action.id, status: result ?? 'completed' };
    } else {
      reads.push(String(input)); const result = readReply?.(reads.length);
      if (result instanceof Error) throw result;
      value = result ?? { id: selection.subject, owner: selection.owner, deleted: false, body: { version: 2 } };
    }
    return Response.json({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot, value });
  } });
  return { actions, reads, client };
}

test('unconfirmed membership retains original deep-copied identity, fence and operations', async () => {
  const h = setup((_, count) => count === 1 ? new TypeError('response lost after commit') : 'completed');
  const key = structuredClone(identity), ops = structuredClone(operations);
  const write = new QuickAccessWrite(h.client, selection, key, ops);
  await assert.rejects(write.send()); const original = structuredClone(h.actions[0]);
  assert.equal(write.unconfirmed, true); assert.equal(write.refused, false);
  key.id = 'action/changed'; key.fence.subject_revisions.changed = 'claim/changed'; ops[0].member = 'mission/changed';
  await write.send(); assert.deepEqual(h.actions[1], original);
  assert.equal(write.unconfirmed, false);
});

for (const lostStage of ['arrangement.edit', 'arrangement.membership.edit']) {
  test(`compound edit retains exact ${lostStage} stage after response loss`, async () => {
    let lost = false;
    const h = setup(action => { if (!lost && action.type === lostStage) { lost = true; return new TypeError('lost receipt'); } });
    const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
    await assert.rejects(write.send());
    assert.equal(write.stage, lostStage === 'arrangement.edit' ? 'create' : 'membership');
    const original = structuredClone(h.actions.at(-1)), before = h.actions.length;
    await write.send(); assert.deepEqual(h.actions[before], original);
    assert.deepEqual(h.actions.map(a => a.type), lostStage === 'arrangement.edit'
      ? ['arrangement.edit', 'arrangement.edit', 'arrangement.membership.edit']
      : ['arrangement.edit', 'arrangement.membership.edit', 'arrangement.membership.edit']);
  });
}

for (const details of [{ applied: 'none' }, { applied: 'unknown' }, { applied: 'none', replay_unsafe: true }]) {
  test(`only guaranteed first-attempt nonapplication refuses: ${JSON.stringify(details)}`, async () => {
    const h = setup(() => refused(details)), write = new QuickAccessWrite(h.client, selection, identity, operations);
    await assert.rejects(write.send());
    const safe = details.applied === 'none' && !details.replay_unsafe;
    assert.equal(write.refused, safe); assert.equal(write.unconfirmed, !safe);
  });
}

test('a refused explicit check cannot prove the original ambiguous write was unapplied', async () => {
  const h = setup((_, count) => count === 1 ? new TypeError('lost original receipt') : refused({ applied: 'none' }));
  const write = new QuickAccessWrite(h.client, selection, identity, operations);
  await assert.rejects(write.send()); await assert.rejects(write.send());
  assert.equal(write.unconfirmed, true); assert.equal(write.refused, false);
  assert.deepEqual(h.actions[0], h.actions[1]);
});

for (const status of ['accepted', 'rejected']) {
  test(`matching ${status} is not a canonical completed arrangement receipt`, async () => {
    const h = setup(() => status), write = new QuickAccessWrite(h.client, selection, identity, operations);
    await assert.rejects(write.send()); assert.equal(write.unconfirmed, true);
  });
}

test('a completed result for another action cannot settle the retained edit', async () => {
  const h = setup(() => Response.json({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot, value: { kind: 'action-result', action_id: 'action/other', status: 'completed' } }));
  const write = new QuickAccessWrite(h.client, selection, identity, operations);
  await assert.rejects(write.send()); await assert.rejects(write.send());
  assert.equal(write.unconfirmed, true); assert.deepEqual(h.actions[0], h.actions[1]);
});

test('confirmed create replay makes membership a new first attempt', async () => {
  const h = setup((action, count) => count === 1 ? new TypeError('create receipt lost')
    : action.type === 'arrangement.edit' ? 'completed' : refused({ applied: 'none' }));
  const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
  await assert.rejects(write.send()); await assert.rejects(write.send());
  assert.equal(write.stage, 'membership'); assert.equal(write.refused, true); assert.equal(write.unconfirmed, false);
  assert.deepEqual(h.actions[0], h.actions[1]); assert.equal(h.actions[2].type, 'arrangement.membership.edit');
});


const exists = details => Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'arrangement-exists', message: 'already exists', retryable: false, ...(details ? { details } : {}) }, { status: 409 });
test('real bare create-race refusal adopts the exact live v2 collection before membership', async () => {
  const h = setup(action => action.type === 'arrangement.edit' ? exists() : 'completed');
  const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
  await write.send();
  assert.equal(h.reads.length, 1); assert.equal(write.needsAdoption, false);
  assert.deepEqual(h.actions.map(a => a.type), ['arrangement.edit', 'arrangement.membership.edit']);
});

test('lost adoption GET explicitly resumes validation without resending CREATE', async () => {
  const h = setup(action => action.type === 'arrangement.edit' ? exists() : 'completed', count => count === 1 ? new TypeError('GET response lost') : undefined);
  const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
  await assert.rejects(write.send());
  assert.equal(write.needsAdoption, true); assert.equal(write.unconfirmed, true);
  assert.equal(h.actions.length, 1);
  await write.send();
  assert.equal(h.reads.length, 2); assert.equal(write.needsAdoption, false);
  assert.deepEqual(h.actions.map(a => a.type), ['arrangement.edit', 'arrangement.membership.edit']);
  assert.equal(h.actions[1].id, identity.id);
});

for (const invalid of [{ deleted: true }, { deleted: null }, { owner: 'person/other' }, { id: 'arrangement/person/example/other' }, { body: { version: 1 } }]) {
  test(`adoption refuses incompatible collection ${JSON.stringify(invalid)}`, async () => {
    const h = setup(() => exists(), () => ({ id: selection.subject, owner: selection.owner, deleted: false, body: { version: 2 }, ...invalid }));
    const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
    await assert.rejects(write.send(), /cannot be adopted/); await assert.rejects(write.send(), /cannot be adopted/);
    assert.equal(write.needsAdoption, true); assert.match(write.adoptionIssue, /cannot be adopted/);
    assert.equal(h.actions.length, 1); assert.equal(h.reads.length, 2);
  });
}

for (const details of [{ applied: 'unknown' }, { replay_unsafe: true }]) {
  test(`ambiguous create replay refused as exists cannot adopt ${JSON.stringify(details)}`, async () => {
    const h = setup((_, count) => count === 1 ? new TypeError('create receipt lost') : exists(details));
    const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
    await assert.rejects(write.send()); await assert.rejects(write.send());
    assert.equal(write.unconfirmed, true); assert.equal(write.needsAdoption, false);
    assert.equal(h.reads.length, 0); assert.deepEqual(h.actions[0], h.actions[1]);
  });
}

for (const details of [{ applied: 'unknown' }, { replay_unsafe: true }]) {
  test(`unsafe first create-race response cannot adopt ${JSON.stringify(details)}`, async () => {
    const h = setup(() => exists(details)), write = new QuickAccessWrite(h.client, selection, identity, operations, create);
    await assert.rejects(write.send()); assert.equal(write.unconfirmed, true);
    assert.equal(write.needsAdoption, false); assert.equal(h.reads.length, 0);
  });
}


test('safe create conflict after ambiguous replay ensures the container then sends one original membership', async () => {
  const h = setup((action, count) => count === 1 ? new TypeError('create response lost') : action.type === 'arrangement.edit' ? exists() : 'completed');
  const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
  await assert.rejects(write.send()); await write.send();
  assert.equal(h.reads.length, 1); assert.equal(write.unconfirmed, false);
  assert.deepEqual(h.actions[0], h.actions[1]);
  assert.deepEqual(h.actions.map(a => a.type), ['arrangement.edit', 'arrangement.edit', 'arrangement.membership.edit']);
  assert.equal(h.actions[2].id, identity.id);
});

test('safe conflict after ambiguous replay retains GET-only validation when that read is lost', async () => {
  const h = setup((action, count) => count === 1 ? new TypeError('create response lost') : action.type === 'arrangement.edit' ? exists() : 'completed', count => count === 1 ? new TypeError('GET lost') : undefined);
  const write = new QuickAccessWrite(h.client, selection, identity, operations, create);
  await assert.rejects(write.send()); await assert.rejects(write.send());
  assert.equal(write.needsAdoption, true); assert.equal(h.actions.length, 2);
  await write.send();
  assert.equal(h.reads.length, 2); assert.equal(write.unconfirmed, false);
  assert.deepEqual(h.actions[0], h.actions[1]);
  assert.deepEqual(h.actions.map(a => a.type), ['arrangement.edit', 'arrangement.edit', 'arrangement.membership.edit']);
  assert.equal(h.actions[2].id, identity.id);
});
