import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSidebarBehavior } from '../../scripts/sidebar-behavior-fixtures.mjs';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';
import { sidebarGroups, rememberResources } from './sidebarView.ts';
import { placeResource, moveFolder, validateSidebarMove } from './sidebarEdits.ts';
import { buildTree } from './sidebarPlanning.ts';

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
