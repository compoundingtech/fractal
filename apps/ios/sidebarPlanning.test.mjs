import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { buildTree, nodeKey, sameNode, spread } from './sidebarPlanning.ts';

const vectors = JSON.parse(readFileSync(new URL('../../fixtures/clients/sidebar/behavior-v2.json', import.meta.url), 'utf8'));
for (const vector of vectors.cases) {
  const before = structuredClone(vector.initial);
  assert.deepEqual(buildTree(vector.initial.arrangement, vector.initial.memberships, vector.catalog), vector.checkpoints[0].tree, vector.name);
  assert.deepEqual(vector.initial, before, 'projection must not mutate input');
}
const root = { _tag: 'Root' };
const folder = id => ({ _tag: 'Folder', id });
const subject = subject => ({ _tag: 'Subject', subject });
const register = value => ({ value, revision: 'claim/raw' });
const liveFolder = (parent = null) => ({ name: register('Folder'), position: register({ parent, key: 'a0' }), tombstone: null });
const arrangement = () => ({ id: 'arrangement/test', owner: 'person/test', revision: 'claim/tree', deleted: false,
  body: { version: 1, folders: {}, placements: {} }, resolved: { parents: {}, folders: {} } });
const membership = (member, bucket = null) => ({ container: 'arrangement/test', member, revision: 'claim/member', position: { bucket, key: 'a0' } });
assert.equal(nodeKey(folder('same')), 'folder:same');
assert.equal(nodeKey(subject('same')), 'subject:same');
assert.equal(sameNode(root, { _tag: 'Root' }), true);
assert.equal(sameNode(folder('same'), subject('same')), false);
assert.equal(sameNode(subject('x'), subject('x')), true);
const a = arrangement();
a.body.folders.live = liveFolder('deleted');
a.body.folders.deleted = { ...liveFolder(), tombstone: register(true) };
a.resolved.parents.live = null;
a.body.placements.retained = register({ folder: 'deleted', key: 'Zz' });
a.resolved.folders.retained = 'live';
const projected = buildTree(a, undefined, ['fallback', 'retained', '\u{10000}', '\ue000']);
assert.deepEqual(projected.roots, [folder('live'), subject('fallback'), subject('\ue000'), subject('\u{10000}')]);
assert.deepEqual(projected.nodes['folder:live'].parent, folder('deleted'));
assert.deepEqual(projected.nodes['folder:live'].shownUnder, root);
assert.deepEqual(projected.nodes['folder:live'].children, [subject('retained')]);
assert.deepEqual(projected.deletedFolders, ['deleted']);
assert.equal(projected.nodes['subject:fallback'].source, 'fallback');
assert.equal(Object.hasOwn(projected.nodes['subject:fallback'], 'key'), false);
assert.deepEqual(buildTree({ ...a, deleted: true, resolved: null }, []), { error: { _tag: 'ArrangementUnavailable' } });
assert.deepEqual(buildTree(a, []), { error: { _tag: 'Unsupported', required: 2, advertised: 1 } });
const v2 = arrangement(); v2.body = { version: 2, folders: {} };
assert.deepEqual(buildTree(v2), { error: { _tag: 'Unsupported', required: 1, advertised: 2 } });
assert.deepEqual(buildTree({ ...v2, resolved: null }, []), { error: { _tag: 'NotReady' } });
for (const members of [[membership('x'), membership('x')], [{ ...membership('x'), container: 'other' }], [membership('x', 'absent')]]) {
  assert.deepEqual(buildTree(v2, members), { error: { _tag: 'InvalidResolution' } });
}
const invalid = arrangement();
invalid.body.folders.cycle = liveFolder(); invalid.resolved.parents.cycle = 'cycle';
invalid.body.folders.missing = liveFolder();
assert.deepEqual(buildTree(invalid), { error: { _tag: 'NotReady' } });
invalid.resolved.parents.missing = null;
assert.deepEqual(buildTree(invalid), { error: { _tag: 'InvalidResolution' } });
invalid.resolved.parents.cycle = 'absent';
assert.deepEqual(buildTree(invalid), { error: { _tag: 'InvalidResolution' } });
// Iterative ancestry accepts deep resolved chains without recursive stack growth.
const deep = arrangement();
for (let i = 0; i < 12000; i++) { deep.body.folders[i] = liveFolder(); deep.resolved.parents[i] = i === 11999 ? null : String(i + 1); }
assert.deepEqual(buildTree(deep).roots, [folder('11999')]);

assert.deepEqual(spread('a0', null, 2), ['aL', 'af']);
assert.deepEqual(spread(null, null, 2), ['Zf', 'aK']);
assert.deepEqual(spread('a0', 'a1', 1), ['a0V']);
assert.deepEqual(spread(null, 'Zz', 1), ['ZU']);
assert.deepEqual(spread(null, null, 0), []);
for (const [low, high, count] of [['invalid', null, 1], ['a0', 'a0', 0], ['a1', 'a0', 1], ['A' + '0'.repeat(26), null, 1], ['a00', null, 1], [null, null, -1], [null, null, 1.5], [null, null, Number.MAX_VALUE]]) assert.equal(spread(low, high, count), undefined);
const lower = 'a0V' + 'z'.repeat(125), upper = 'a0W' + '0'.repeat(124) + '2';
assert.deepEqual(spread(lower, upper, 2), ['a0W', 'a0W' + '0'.repeat(124) + '1']);
assert.equal(spread(lower, upper, 3), undefined);
const prefix = 'a0' + '0'.repeat(125);
const alphabet = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
const pool = [...alphabet.slice(2, -1)].map(char => prefix + char);
for (const count of [1, 2, 7, 59]) assert.deepEqual(spread(prefix + '1', prefix + 'z', count), Array.from({ length: count }, (_, i) => pool[Math.floor(pool.length * (i + 1) / (count + 1))]));
assert.equal(spread(prefix + '1', prefix + 'z', 60), undefined);
