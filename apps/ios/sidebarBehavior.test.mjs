import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSidebarBehavior } from '../../scripts/sidebar-behavior-fixtures.mjs';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';
import { sidebarGroups, rememberResources } from './sidebarView.ts';
import { placeResource, moveFolder, validateSidebarMove } from './sidebarEdits.ts';
import { buildTree } from './sidebarPlanning.ts';
import { hostname, arch, platform } from 'node:os';
import { SidebarMemberships } from './sidebarMemberships.ts';
import { readSidebarMemberships, sidebarMembershipFrame, sidebarMembershipFailure } from './feed.ts';
import { planSidebarV2 } from './sidebarEdits.ts';
import { createWriteState, writeReducer } from './sidebarWriteState.ts';
import { sidebarWriteRequest } from './sidebarDispatch.ts';
import { SidebarProjectionCache } from './sidebarProjectionCache.ts';

const fixtures = loadSidebarBehavior();
const nested = 'nested-and-dormant-non-agent-placements';
const lifted = 'multi-tombstone-lift-and-unknown-root';
const cycle = 'authoritative-cycle-cut-not-revision-order';
const unicode = 'separate-domains-and-unicode-scalar-ties';
const uuid = suffix => `00000000-0000-7000-8000-00000000000${suffix}`;
// Explicit native-v1 differences, not exemptions: both the shared expectation and native
// outcome are asserted. Remove an entry when the corresponding production behavior changes.
const divergences = {
  [`${nested}/tree`]: { reason: 'Native v1 displays nested folders as peer groups.', flattened: [uuid('b')] },
  [`${lifted}/tree`]: { reason: 'Native v1 displays nested folders as peer groups.', flattened: [uuid('b')] },
  [`${cycle}/tree`]: { reason: 'Native v1 displays resolved nested folders as peer groups.', flattened: [uuid('a'), uuid('b')] },
  [`${unicode}/tree`]: { reason: 'Native ties use UTF-16, not Unicode scalar ordering.', roots: ['agent/test/😀', 'agent/test/\ue000', 'agent/test/unplaced'] },
  [`${nested}/0`]: { reason: 'Flat folder reorder has no self-parent validation.', shared: 'Self', native: [{ op: 'folder.move', id: uuid('a'), parent: null, key: 'a2' }] },
  [`${nested}/1`]: { reason: 'Flat folder reorder has no ancestry-cycle validation.', shared: 'Cycle', native: [{ op: 'folder.move', id: uuid('a'), parent: null, key: 'a2' }] },
  [`${nested}/2`]: { reason: 'Native destination is a folder ID, not a typed node.', shared: 'ParentNotAllowed', native: [{ op: 'subject.place', subject: 'agent/test/one', folder: 'agent/test/dormant', key: 'a0' }] },
  [`${nested}/3`]: { reason: 'Native planner trusts the destination supplied by its UI.', shared: 'ParentMissing', native: [{ op: 'subject.place', subject: 'agent/test/one', folder: uuid('e'), key: 'a0' }] },
  [`${nested}/4`]: { reason: 'Native planner permits new resource placements.', shared: 'NodeMissing', native: [{ op: 'subject.place', subject: 'agent/test/missing', folder: null, key: 'a0' }] },
  [`${nested}/5`]: { reason: 'Native anchor error is unstructured.', shared: 'AnchorMoved', error: 'The destination changed; choose it again.' },
  [`${nested}/6`]: { reason: 'Native planner emits an operation for a no-op drop.', shared: 'NoChange', native: [{ op: 'subject.place', subject: 'agent/test/one', folder: uuid('b'), key: 'a0' }] },
  [`${lifted}/1`]: { reason: 'Native planner does not reject a tombstoned destination.', shared: 'ParentMissing', native: [{ op: 'subject.place', subject: 'agent/test/lifted', folder: uuid('c'), key: 'a0' }] },
};
const used = new Set();
const difference = id => { const value = divergences[id]; if (value) { assert.ok(value.reason); used.add(id); } return value; };
const snapshot = { id: 'snapshot/host/test/1', host_id: 'host/test', store_index: 1, projection_version: 'client-projection.v0', created_at: '2026-10-15T00:00:00Z' };
const resourcesFor = catalog => rememberResources(new Map(), catalog.map(id => ({ id, title: id, target: id, kind: 'Resources' })));
const subjectIds = nodes => nodes.filter(node => node._tag === 'Subject').map(node => node.subject);
const plan = (arrangement, intent) => intent.node._tag === 'Folder'
  ? moveFolder(arrangement, intent.node.id, intent.before?.id)
  : placeResource(arrangement, intent.node.subject, intent.parent._tag === 'Root' ? null : intent.parent.id ?? intent.parent.subject, intent.before?.subject ?? intent.before?.id);
// Apply the returned wire operations to an authoritative test frame, not to a copied planner.
const acknowledged = (arrangement, operations) => {
  const next = structuredClone(arrangement);
  for (const operation of operations) {
    assert.ok(operation.key.length > 0 && operation.key.length <= 128);
    if (operation.op === 'subject.place') {
      next.body.placements[operation.subject] = { value: { folder: operation.folder, key: operation.key }, revision: 'claim/edit' };
      next.resolved.folders[operation.subject] = operation.folder;
    } else {
      assert.equal(operation.op, 'folder.move');
      next.body.folders[operation.id].position = { value: { parent: operation.parent, key: operation.key }, revision: 'claim/edit' };
    }
  }
  return next;
};

test('schema-invalid behavior is rejected before replay', () => {
  for (const mutate of [value => { value.schemaVersion = 2; }, value => { value.cases[0].events[0].unknown = true; }]) {
    const invalid = structuredClone(fixtures); mutate(invalid);
    assert.throws(() => loadSidebarBehavior(invalid));
  }
});

for (const fixture of fixtures.cases) test(`native sidebar behavior: ${fixture.name}`, async () => {
  const { initial: { arrangement }, catalog, events: [event], checkpoints: [checkpoint] } = fixture;
  assert.equal(event.type, 'ProjectAndPlanV1');
  const { client, sockets } = fakeClient();
  let delivered;
  const feed = new Feed(client, { onWindow() {}, onConnection() {}, onWindowError() {} }, new ForegroundGate('active'), () => 'action/test');
  feed.followArrangements(arrangement.owner, { onArrangements: value => { delivered = value; }, onIssue: issue => assert.equal(issue, '') });
  try {
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(sockets.length, 1);
    const send = (item, changes = false) => sockets[0].frame({ kind: changes ? 'changes' : 'snapshot', id: 'arrangements', collection: 'arrangements', snapshot, ...(changes ? { upserts: [item], removes: [] } : { items: [item] }), order: [item.id], has_more: false });
    send(arrangement);
    assert.deepEqual(delivered, [arrangement]);
    const resources = resourcesFor(catalog);
    const view = sidebarGroups(delivered, resources);
    const nodes = Object.values(checkpoint.tree.nodes);
    const folders = nodes.filter(value => value.node._tag === 'Folder').sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : a.node.id < b.node.id ? -1 : 1);
    const delta = difference(`${fixture.name}/tree`);
    const nestedIds = nodes.filter(value => value.node._tag === 'Folder' && value.parent._tag === 'Folder').map(value => value.node.id);
    assert.deepEqual(nestedIds, delta?.flattened ?? []);
    const actualFolders = view.groups.filter(group => group.folder);
    assert.deepEqual(actualFolders.map(group => ({ id: group.id, subjects: group.resources.map(row => row.id) })), folders.map(value => ({ id: `folder/${value.node.id}`, subjects: subjectIds(value.children) })));
    const roots = subjectIds(checkpoint.tree.roots);
    if (delta?.roots) assert.notDeepEqual(delta.roots, roots);
    assert.deepEqual(view.groups.filter(group => !group.folder).flatMap(group => group.resources.map(row => row.id)), delta?.roots ?? roots);
    for (const group of view.groups) for (const row of group.resources) assert.equal(row.missing, !catalog.includes(row.id));
    assert.equal(view.unfiledCount, roots.length);
    assert.deepEqual(checkpoint.effects, []);
    assert.deepEqual(checkpoint.wireActions, []);
    assert.equal(event.moves.length, checkpoint.moves.length);
    for (const [index, intent] of event.moves.entries()) {
      const expected = checkpoint.moves[index];
      const refusal = validateSidebarMove(buildTree(arrangement, undefined, catalog), intent);
      if (expected._tag === 'Refused') {
        assert.deepEqual(refusal, expected.refusal);
        // The validator is unwired: retain the live editor/Feed contract until activation.
        const delta = difference(`${fixture.name}/${index}`);
        assert.ok(delta, 'Every live native planning difference must remain explicit.');
        assert.equal(expected.refusal._tag, delta.shared);
        if (delta.error) { assert.throws(() => plan(arrangement, intent), { message: delta.error }); continue; }
        const operations = plan(arrangement, intent);
        assert.deepEqual(operations, delta.native);
        const next = acknowledged(arrangement, operations);
        send(next, true);
        assert.deepEqual(delivered, [next]);
        continue;
      }
      assert.equal(refusal, undefined);
      assert.deepEqual(expected.intent, intent);
      const operations = plan(arrangement, intent);
      assert.deepEqual(operations.map(({ key, ...operation }) => operation), [{ op: 'subject.place', subject: intent.node.subject, folder: intent.parent.id ?? null }]);
      send(acknowledged(arrangement, operations), true);
      const groups = sidebarGroups(delivered, resources).groups;
      assert.ok(groups.find(group => group.id === `folder/${intent.parent.id}`).resources.some(row => row.id === intent.node.subject));
    }
  } finally { feed.close(); }
});

test('all documented native differences were exercised', () => {
  assert.deepEqual([...used].sort(), Object.keys(divergences).sort());
});

// Opt in: SIDEBAR_IOS_PERF=1 nix develop .#web --command pnpm --dir apps/ios test
// Node measurements are pre-activation CPU evidence, not native release/frame evidence.
const scaleScope = { owner: 'person/test', container: 'arrangement/scale', endpoint: 'local', host: snapshot.host_id, projection: snapshot.projection_version };
const subject = subject => ({ _tag: 'Subject', subject });
const folder = id => ({ _tag: 'Folder', id });
const scaleFixture = (count, depth = 1) => {
  const arrangement = { id: scaleScope.container, owner: scaleScope.owner, revision: 'claim/scale',
    body: { version: 2, name: { value: 'Sidebar', revision: 'claim/scale' }, folders: {} }, resolved: { parents: {} } };
  for (let i = 0; i < depth; i++) {
    const id = `00000000-0000-7000-8000-${String(i).padStart(12, '0')}`, parent = i ? Object.keys(arrangement.body.folders).at(-1) : null;
    arrangement.body.folders[id] = { name: { value: `Folder ${i}`, revision: 'claim/folder' }, position: { value: { parent, key: 'a0' }, revision: 'claim/folder' } };
    arrangement.resolved.parents[id] = parent;
  }
  const destination = Object.keys(arrangement.body.folders).at(-1);
  const memberships = Array.from({ length: count }, (_, i) => ({ id: `membership/${i}`, member: `agent/scale/${String(i).padStart(4, '0')}`, container: arrangement.id, revision: 'claim/member', position: { bucket: null, key: 'a0' } }));
  const catalog = memberships.map(row => row.member), resources = resourcesFor(catalog);
  const intent = { node: subject(catalog[0]), parent: folder(destination), before: null };
  const v1 = { ...arrangement, body: { ...arrangement.body, version: 1, placements: Object.fromEntries(memberships.map(row => [row.member, { value: { folder: row.position.bucket, key: row.position.key }, revision: row.revision }])) }, resolved: { ...arrangement.resolved, folders: Object.fromEntries(catalog.map(id => [id, null])) } };
  return { arrangement, memberships, catalog, resources, intent, v1 };
};
const cutFor = count => ({ epoch: 'scale-epoch', changed_index: 1, live_count: count });
const scaleRead = async (memberships, onPage = () => {}) => {
  const state = new SidebarMemberships(), cut = cutFor(memberships.length);
  let effects = state.reduce({ type: 'Refresh', scope: scaleScope, generation: 1 }), requests = 0, active = 0;
  while (effects.length && effects[0].type !== 'PersistBase') {
    assert.equal(effects.length, 1);
    const effect = effects[0];
    const event = await readSidebarMemberships(effect, async () => {}, async (_token, cursor) => {
      assert.equal(++active, 1, 'one page in flight');
      const offset = cursor ? Number(cursor) : 0, end = Math.min(offset + 128, memberships.length);
      // Model unrelated daemon writes: global snapshot advances, collection cut does not.
      const page = { frontier: cut, snapshot: { ...snapshot, id: `snapshot/scale/${requests}`, store_index: ++requests }, members: memberships.slice(offset, end), has_more: end < memberships.length, ...(end < memberships.length ? { next: String(end) } : {}) };
      onPage(state, effect.token, page); active--;
      return page;
    });
    effects = state.reduce(event);
    if (state.status === 'Paging') assert.equal(state.complete, undefined);
  }
  assert.equal(state.status, 'Ready'); assert.equal(state.stale, false);
  assert.deepEqual(state.complete.members, memberships);
  assert.equal(requests, Math.max(1, Math.ceil(memberships.length / 128)));
  return state;
};

for (const count of [1001, 4096]) test(`native real feed/projection/planning at ${count} tied live members`, async () => {
  const f = scaleFixture(count);
  const state = await scaleRead(f.memberships, (state, token, page) => {
    const event = sidebarMembershipFrame(token, 'scale', { kind: 'changes', id: 'scale', collection: 'ordered-memberships', membership: { container: scaleScope.container, changed_index: 1, live_count: count }, snapshot: page.snapshot, upserts: [], removes: [], order: [], has_more: false }, 'scale-epoch');
    assert.deepEqual(state.reduce(event), []);
  });
  const tree = buildTree(f.arrangement, state.complete.members, f.catalog);
  assert.equal(Object.keys(tree.nodes).length, count + 1);
  assert.deepEqual(tree.roots.filter(node => node._tag === 'Subject').map(node => node.subject), f.catalog);
  const edit = planSidebarV2(tree, f.intent);
  assert.equal(edit.memberships.length, 1);
  const write = createWriteState(scaleScope, f.intent);
  assert.deepEqual(writeReducer.reduce(write, { type: 'Plan', base: { scope: scaleScope, snapshot, revision: f.arrangement.revision, ready: true }, plan: edit, folder: { actor: 'actor/scale', action_id: 'action/folder', idempotency_key: 'key/folder' }, membership: { actor: 'actor/scale', action_id: 'action/member', idempotency_key: 'key/member' } }), ['PersistPending']);
  assert.equal(write.status, 'AwaitingPersistence', 'pending state exists before any send');
  assert.deepEqual(sidebarWriteRequest(write.request).parameters.operations, edit.memberships);
  const view = sidebarGroups([f.v1], f.resources);
  assert.equal(view.unfiledCount, count);
  assert.equal(view.groups.flatMap(group => group.resources).length, count);
});

test('native admitted 1,024-folder chain projects and plans without recursive traversal', () => {
  const f = scaleFixture(1001, 1024), tree = buildTree(f.arrangement, f.memberships);
  assert.equal(Object.keys(tree.nodes).length, 2025);
  let node = tree.roots.find(node => node._tag === 'Folder'), depth = 0;
  while (node) { depth++; node = tree.nodes[`folder:${node.id}`].children.find(node => node._tag === 'Folder'); }
  assert.equal(depth, 1024);
  assert.equal(planSidebarV2(tree, f.intent).memberships.length, 1);
});

test('atomic 1,025-operation rekey and >1 MiB operations never reach dispatch', () => {
  const f = scaleFixture(2049), destination = f.intent.parent.id;
  for (const row of f.memberships.slice(0, 2048)) row.position.bucket = destination;
  const intent = { node: subject(f.catalog[2048]), parent: folder(destination), before: subject(f.catalog[1024]) };
  let dispatches = 0;
  const dispatchIfPlanned = result => { if (!('error' in result)) { dispatches++; return sidebarWriteRequest(result); } return result.error; };
  const before = structuredClone(f.memberships);
  assert.deepEqual(dispatchIfPlanned(planSidebarV2(buildTree(f.arrangement, f.memberships), intent)), { _tag: 'TooManyOperations', count: 1025 });
  assert.deepEqual(f.memberships, before, 'no partial rekey or truncation');
  const huge = scaleFixture(1), id = 'agent/' + 'x'.repeat(1048576);
  huge.memberships[0].member = id; huge.intent.node = subject(id);
  const refused = dispatchIfPlanned(planSidebarV2(buildTree(huge.arrangement, huge.memberships), huge.intent));
  assert.equal(refused._tag, 'TooManyBytes'); assert.ok(refused.bytes > 1048576);
  assert.equal(dispatches, 0);
});

test('scale refresh faults retain stale complete rows and isolate obsolete attempts', async () => {
  const f = scaleFixture(4096), original = await scaleRead(f.memberships), base = original.complete;
  for (const fault of ['disconnect', 'unauthorized', 'expired', 'repeated', 'edited']) {
    const state = new SidebarMemberships(base);
    const token = state.reduce({ type: 'Refresh', scope: scaleScope, generation: 2 })[0].token;
    const page = { frontier: cutFor(4096), snapshot, members: f.memberships.slice(0, 128), has_more: true, next: 'cursor' };
    state.reduce({ type: 'Page', token, page });
    assert.equal(state.status, 'Paging'); assert.equal(state.complete, base); assert.equal(state.stale, true);
    if (fault === 'repeated') state.reduce({ type: 'Page', token, page: { ...page, members: f.memberships.slice(128, 256) } });
    else if (fault === 'edited') state.reduce({ type: 'Metadata', token, frontier: { ...cutFor(4096), changed_index: 2 } });
    else state.reduce(sidebarMembershipFailure(token, fault === 'disconnect' ? {} : { status: fault === 'expired' ? 409 : 403, response: { code: fault === 'expired' ? 'membership-collection-changed' : 'forbidden' } }));
    assert.equal(state.status, ({ disconnect: 'Disconnected', unauthorized: 'Refused', expired: 'Paging', repeated: 'Refused', edited: 'Paging' })[fault]);
    assert.equal(state.complete, base); assert.equal(state.stale, true);
    if (fault === 'expired' || fault === 'edited') assert.deepEqual(state.reduce({ type: 'Page', token, page }), []);
  }
  const empty = await scaleRead([]);
  assert.equal(empty.status, 'Ready'); assert.equal(empty.complete.members.length, 0);
  assert.notEqual(new SidebarMemberships().status, empty.status);
});

const projectionInput = f => ({ arrangement: f.arrangement, complete: { scope: scaleScope, frontier: cutFor(f.memberships.length), snapshots: [snapshot], members: f.memberships }, modelRevision: 'model/1', catalogRevision: 'catalog/1', presentationRevision: 'presentation/1' });
test('projection cache reuses revisions, invalidates every relevant identity and scope', () => {
  const f = scaleFixture(1001), input = projectionInput(f), cache = new SidebarProjectionCache(), tree = cache.get(input);
  const equivalent = structuredClone(input);
  equivalent.complete.snapshots = [{ ...snapshot, id: 'snapshot/unrelated', store_index: 99999 }];
  assert.equal(cache.get(equivalent), tree, 'new object and unrelated global writes reuse projection');
  for (const mutate of [
    x => { x.arrangement.revision += '/next'; }, x => { x.arrangement.deleted = true; },
    x => { x.modelRevision += '/resolved'; }, x => { x.catalogRevision += '/next'; },
    x => { x.presentationRevision += '/next'; },
    ...['epoch', 'changed_index', 'live_count'].map(key => x => { x.complete.frontier[key] = typeof x.complete.frontier[key] === 'number' ? x.complete.frontier[key] + 1 : 'next-epoch'; }),
    ...['endpoint', 'host', 'projection'].map(key => x => { x.complete.scope[key] += '/other'; }),
    x => { x.complete.scope.owner = x.arrangement.owner = 'person/other'; },
    x => { x.complete.scope.container = x.arrangement.id = 'arrangement/other'; x.complete.members = x.complete.members.map(row => ({ ...row, container: x.arrangement.id })); },
  ]) {
    const changed = structuredClone(input); mutate(changed);
    assert.notEqual(cache.get(changed), tree);
    assert.notEqual(cache.get(input), tree, 'single entry never reuses an evicted scope/revision');
  }
  const fresh = cache.get(input); cache.clear(); assert.notEqual(cache.get(input), fresh);
  const mismatch = structuredClone(input); mismatch.complete.scope.container = 'arrangement/other';
  assert.deepEqual(cache.get(mismatch), { error: { _tag: 'InvalidResolution' } });
  const changed = structuredClone(input); changed.complete.frontier.changed_index++;
  changed.complete.members[0].position.bucket = f.intent.parent.id;
  assert.equal(cache.get(changed).nodes[`subject:${f.catalog[0]}`].shownUnder.id, f.intent.parent.id);
  assert.deepEqual(planSidebarV2({ error: { _tag: 'NotReady' } }, f.intent), { error: { _tag: 'Projection', error: { _tag: 'NotReady' } } });
});
test('same-revision resolution recovery never caches failures or scans members on warm hits', () => {
  const f = scaleFixture(4096, 2), input = projectionInput(f), cache = new SidebarProjectionCache();
  const [root, child] = Object.keys(f.arrangement.body.folders), parents = f.arrangement.resolved.parents;
  delete parents[child];
  assert.deepEqual(cache.get(input), { error: { _tag: 'NotReady' } });
  parents[child] = root;
  const complete = cache.get(input); assert.ok(!('error' in complete));
  delete parents[root];
  assert.deepEqual(cache.get(input), { error: { _tag: 'NotReady' } });
  parents[root] = null;
  const recovered = cache.get(input); assert.ok(!('error' in recovered)); assert.notEqual(recovered, complete);
  input.complete.members = new Proxy(f.memberships, { get() { assert.fail('warm cache must not inspect members'); } });
  assert.equal(cache.get(input), recovered);
  input.complete.members = f.memberships;
  parents[root] = child;
  assert.deepEqual(cache.get(input), { error: { _tag: 'InvalidResolution' } }, 'same-revision parent change creates a cycle');
  parents[root] = null;
  const valid = cache.get(input); assert.ok(!('error' in valid)); assert.notEqual(valid, recovered);
  parents[child] = null;
  assert.equal(cache.get(input).nodes[`folder:${child}`].shownUnder._tag, 'Root');
  const invalid = projectionInput(scaleFixture(1)), badCache = new SidebarProjectionCache();
  invalid.complete.members[0].container = 'arrangement/other';
  assert.deepEqual(badCache.get(invalid), { error: { _tag: 'InvalidResolution' } });
  invalid.complete.members[0].container = invalid.arrangement.id;
  assert.ok(!('error' in badCache.get(invalid)), 'non-readiness errors are not cached either');
});
test('cache invalidation preserves authoritative projection error precedence', () => {
  const f = scaleFixture(2), input = projectionInput(f), cache = new SidebarProjectionCache();
  cache.get(input);
  const legacy = { ...input, arrangement: { ...f.v1, resolved: undefined } };
  assert.deepEqual(cache.get(legacy), { error: { _tag: 'Unsupported', required: 2, advertised: 1 } });
  for (const corrupt of [
    members => { members.push(structuredClone(members[0])); },
    members => { members[0].container = 'arrangement/other'; },
  ]) {
    cache.get(input);
    const invalid = structuredClone(input);
    delete invalid.arrangement.resolved.parents[f.intent.parent.id];
    corrupt(invalid.complete.members);
    assert.deepEqual(cache.get(invalid), { error: { _tag: 'InvalidResolution' } });
    assert.deepEqual(cache.get(invalid), buildTree(invalid.arrangement, invalid.complete.members));
  }
});
const pendingEdit = (f, edit) => {
  const state = createWriteState(scaleScope, f.intent);
  writeReducer.reduce(state, { type: 'Plan', base: { scope: scaleScope, snapshot, revision: f.arrangement.revision, ready: true }, plan: edit, folder: { actor: 'actor/test', action_id: 'action/folder', idempotency_key: 'key/folder' }, membership: { actor: 'actor/test', action_id: 'action/member', idempotency_key: 'key/member' } });
  return state;
};
const summary = values => { const sorted = values.sort((a, b) => a - b); return { p50: sorted[Math.ceil(sorted.length * .5) - 1], p95: sorted[Math.ceil(sorted.length * .95) - 1] }; };
test('opt-in native cached keystroke CPU latency and equivalent v1 baseline', { skip: process.env.SIDEBAR_IOS_PERF !== '1' && 'Set SIDEBAR_IOS_PERF=1; timing gates are not default CI assertions' }, async t => {
  const results = [];
  for (const count of [1001, 4096]) {
    const f = scaleFixture(count), input = projectionInput(f), cache = new SidebarProjectionCache(), coldCache = new SidebarProjectionCache();
    const tree = cache.get(input), v1Tree = buildTree(f.v1, undefined, f.catalog);
    const times = { v2: [], v1: [], coldProjection: [], v1View: [], inMemoryInitialLoad: [] };
    for (let i = 0; i < 120; i++) {
      let started = performance.now();
      const current = cache.get(input), edit = planSidebarV2(current, f.intent);
      const pending = pendingEdit(f, edit);
      const v2 = performance.now() - started;
      assert.equal(current, tree); assert.equal(pending.status, 'AwaitingPersistence');
      assert.deepEqual(pending.request.operations, { Memberships: edit.memberships });
      started = performance.now();
      // Same revision: reuse the v1 projection too. Normalize legacy Place payload
      // solely to charge both baselines the same real pending-reducer transition.
      const operations = placeResource(f.v1, f.catalog[0], f.intent.parent.id);
      const v1Pending = pendingEdit(f, { folders: [], memberships: operations.map(op => ({ op: 'place', member: op.subject, bucket: op.folder, key: op.key })) });
      const v1 = performance.now() - started;
      assert.equal(v1Tree.roots.length, count + 1); assert.equal(v1Pending.status, pending.status);
      started = performance.now(); sidebarGroups([f.v1], f.resources, {}, 'agent');
      const view = performance.now() - started;
      coldCache.clear(); started = performance.now(); const cold = coldCache.get(input);
      const coldProjection = performance.now() - started;
      assert.equal(cold.roots.length, count + 1);
      if (i >= 20) { times.v2.push(v2); times.v1.push(v1); times.v1View.push(view); times.coldProjection.push(coldProjection); }
    }
    for (let i = 0; i < 20; i++) { const started = performance.now(); await scaleRead(f.memberships); times.inMemoryInitialLoad.push(performance.now() - started); }
    results.push({ count, ms: Object.fromEntries(Object.entries(times).map(([key, values]) => [key, summary(values)])) });
  }
  console.log(JSON.stringify({ runner: { host: hostname(), platform: platform(), arch: arch(), node: process.version }, samples: 100, warmup: 20, results, limitations: 'Cached CPU projection+planning+pending only, not native overlay painting. Cold projection reported separately, ungated. v1 uses equivalent pending normalization, not an activated legacy journal. D socket and PR25 native release frame evidence remain required.' }));
  for (const { count, ms } of results) await t.test(`${count} cached keystroke <=4ms and <=2x v1`, () => {
    assert.ok(ms.v2.p95 <= 4, `v2 p95 ${ms.v2.p95}ms >4ms`);
    assert.ok(ms.v2.p95 <= 2 * ms.v1.p95, `v2/v1 ${ms.v2.p95 / ms.v1.p95} >2`);
  });
});

test('D: local-socket full initial load p95 <=1s and sustained unrelated daemon writes', { skip: '#4739 D not delivered: real collection-scoped paging daemon required; in-memory page/cut invariance above is not socket evidence' }, () => {});
test('D: real server rejects 4,097th live-member edit', { skip: '#4739 real daemon admission fixture required; do not substitute client-side refusal for the server limit' }, () => {});
test('PR25: native release render p95 <=8ms, next-frame pending overlay, <=2x v1', { skip: 'v2 view/frame host is unwired before PR25; Node sidebarGroups is v1 only, not native frame/render evidence' }, () => {});
