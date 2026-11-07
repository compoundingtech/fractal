import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { loadSidebarBehavior } from '../../scripts/sidebar-behavior-fixtures.mjs';
import { planSidebarV2 } from './sidebarEdits.ts';
import { buildTree } from './sidebarPlanning.ts';

const fixtures = loadSidebarBehavior(JSON.parse(readFileSync(new URL('../../fixtures/clients/sidebar/behavior-v2.json', import.meta.url), 'utf8')));
const root = { _tag: 'Root' }, folder = id => ({ _tag: 'Folder', id }), subject = subject => ({ _tag: 'Subject', subject });
for (const fixture of fixtures.cases) test(`native v2 exact operations: ${fixture.name}`, () => {
  const { arrangement, memberships } = fixture.initial;
  for (const [index, intent] of fixture.events[0].moves.entries()) {
    const actual = planSidebarV2(buildTree(arrangement, memberships), intent);
    assert.deepEqual(actual, fixture.checkpoints[0].moves[index].edit);
    assert.ok(actual.memberships.every(operation => operation.op === 'place'));
  }
});
const initial = () => structuredClone(fixtures.cases[0].initial);
const move = (node, parent, before = null) => ({ node, parent, before });
const a = fixtures.cases[0].checkpoints[0].tree.roots[0].id;
const b = fixtures.cases[0].checkpoints[0].tree.nodes[`folder:${a}`].children[0].id;
const d = fixtures.cases[0].checkpoints[0].tree.deletedFolders[0];
test('v2 refusals retain structured causes, cycle paths and anchor location', () => {
  const { arrangement, memberships } = initial();
  for (const [intent, refusal] of [
    [move(folder(a), folder(a)), { _tag: 'Self', node: folder(a) }],
    [move(subject('missing'), root), { _tag: 'NodeMissing', node: subject('missing') }],
    [move(subject('agent/test/a'), subject('agent/test/b')), { _tag: 'ParentNotAllowed', parent: subject('agent/test/b'), cause: 'SubjectParentUnsupported' }],
    [move(subject('agent/test/a'), folder(d)), { _tag: 'ParentMissing', parent: folder(d), cause: 'FolderDeleted' }],
    [move(subject('agent/test/a'), folder('missing')), { _tag: 'ParentMissing', parent: folder('missing'), cause: 'Unknown' }],
    [move(subject('agent/test/a'), root, subject('agent/test/b')), { _tag: 'AnchorMoved', before: subject('agent/test/b'), parent: root, foundUnder: folder(b) }],
    [move(subject('agent/test/b'), folder(b)), { _tag: 'NoChange', node: subject('agent/test/b'), parent: folder(b) }],
    [move(folder(a), folder(b)), { _tag: 'Cycle', node: folder(a), parent: folder(b), path: [folder(a), folder(b), folder(a)].map(node => ({ node, source: 'person' })) }],
  ]) assert.deepEqual(planSidebarV2(buildTree(arrangement, memberships), intent), { error: { _tag: 'Move', refusal } });
});
test('folder drops use exact folder.move and repair a lifted raw parent', () => {
  const { arrangement, memberships } = initial();
  const plan = planSidebarV2(buildTree(arrangement, memberships), move(folder(b), folder(a)));
  assert.equal(plan.folders.length, 1);
  assert.deepEqual(plan.folders[0], { op: 'folder.move', id: b, parent: a, key: 'a0' });
  assert.deepEqual(plan.memberships, []);
});
test('ties rekey only nearest siblings, subject first and forward on equal cost', () => {
  const { arrangement, memberships } = initial();
  const plan = planSidebarV2(buildTree(arrangement, memberships), move(subject('machine/retained'), folder(b), subject('agent/test/b')));
  assert.equal(plan.memberships.length, 2);
  assert.equal(plan.memberships[0].member, 'machine/retained');
  assert.equal(plan.memberships[1].member, 'agent/test/b');
  assert.equal(plan.memberships.some(op => op.member === 'agent/test/a'), false);
});
test('oversized membership payload is refused before dispatch', () => {
  const { arrangement, memberships } = initial();
  const huge = 'agent/' + 'x'.repeat(1048576);
  memberships[2].member = huge;
  const result = planSidebarV2(buildTree(arrangement, memberships), move(subject(huge), folder(b), subject('agent/test/a')));
  assert.equal(result.error._tag, 'TooManyBytes');
  assert.ok(result.error.bytes > 1048576);
});
test('full collision group exceeding one action refuses without dropping rekeys', () => {
  const { arrangement } = initial();
  const memberships = Array.from({ length: 2048 }, (_, i) => ({ container: arrangement.id, member: `agent/test/${String(i).padStart(4, '0')}`, revision: 'claim/test', position: { bucket: b, key: 'a0' } }));
  memberships.push({ container: arrangement.id, member: 'agent/test/moved', revision: 'claim/test', position: { bucket: null, key: 'a0' } });
  assert.deepEqual(planSidebarV2(buildTree(arrangement, memberships), move(subject('agent/test/moved'), folder(b), subject('agent/test/1024'))), { error: { _tag: 'TooManyOperations', count: 1025 } });
});
