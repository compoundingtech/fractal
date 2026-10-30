import assert from 'node:assert/strict';
import { resourceRow, readSidebarResources, subjectToFile } from './sidebarResources.ts';
import { sidebarGroups, rememberResources } from './sidebarView.ts';
const observation = (id, kind, facts = {}) => ({ id, kind, facts, observed_at: '2026-10-11T00:00:00Z', opened_by: null, opened_by_run: null });
const pull = observation('resource/github/acme/garden/issue/9', 'vcs.pull-request', { title: 'Build garden', number: 9 });
const repo = observation('resource/github/acme/garden', 'vcs.repository', { url: 'https://github.com/acme/garden' });
const custom = observation('resource/notes/one', 'custom.acme.note');
assert.equal(resourceRow(pull).title, '#9 Build garden');
assert.equal(resourceRow(repo).kind, 'Repositories');
assert.equal(resourceRow(custom).kind, 'Resources');
assert.equal(resourceRow(custom).title, custom.id);
const calls = [];
const found = await readSidebarResources(async (filters, options) => {
  calls.push([filters, options]);
  return { value: { items: filters.kind === 'vcs.pull-request' ? [pull] : filters.kind === 'vcs.repository' ? [repo] : [custom, pull], page: { has_more: !filters.kind } } };
});
assert.deepEqual(calls.map(([filter]) => filter.kind), [undefined, 'vcs.pull-request', 'vcs.repository']);
assert.ok(calls.every(([, options]) => options.limit === 200));
assert.equal(found.rows.length, 3); assert.equal(found.truncated, true);
const seen = rememberResources(new Map(), found.rows.map(resourceRow));
const groups = sidebarGroups([], seen).groups;
assert.ok(groups.some(g => g.title === 'Pull requests'));
assert.ok(groups.some(g => g.title === 'Repositories'));
assert.ok(groups.some(g => g.title === 'Resources'));
const missing = rememberResources(seen, []);
assert.equal(missing.get(pull.id).missing, true);
assert.equal(subjectToFile('resource/new/item', seen).id, 'resource/new/item');
assert.equal(subjectToFile('doc/notes', seen).id, 'doc/notes');
assert.equal(subjectToFile(pull.id, seen), undefined);
assert.equal(subjectToFile('two words/subject', seen), undefined);
assert.equal(subjectToFile('justtext', seen), undefined);
await assert.rejects(readSidebarResources(async () => { throw new Error('Read not supported'); }), /Read not supported/);
assert.equal(seen.size, 3, 'a refused optional read never clears the existing browser');

assert.equal(subjectToFile('session/live', seen), undefined);
assert.equal(subjectToFile('pty/live', seen), undefined);

assert.equal(resourceRow(repo).title, 'acme/garden');

const exact = observation('resource/notes/target', 'custom.acme.note');
const specific = await readSidebarResources(async (filters, options) => {
  if (filters.subject_prefix) { assert.equal(options.limit, 1); assert.equal(filters.subject_prefix, exact.id); return { value: { items: [exact], page: { has_more: true } } }; }
  return { value: { items: [], page: { has_more: false } } };
}, exact.id);
assert.deepEqual(specific.rows, [exact]); assert.equal(specific.truncated, false);
const descendant = await readSidebarResources(async (filters) => ({ value: { items: filters.subject_prefix ? [observation(exact.id + '/child', 'custom.acme.note')] : [], page: { has_more: false } } }), exact.id);
assert.deepEqual(descendant.rows, [], 'a prefix descendant never masquerades as the requested subject');
