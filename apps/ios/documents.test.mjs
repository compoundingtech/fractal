import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { ClientError } from '../../clients/typescript/st3-client/index.ts';
import { emptyDocumentList, documentListPlaceholder } from '@smalltalk/st3-views';
import { changedAt, documentsServed, followDocument, nextDocumentPage, openDocument, referencedDocuments, versionLine, whyUnreadable } from './documents.ts';
import { inline, markdown } from './markdown.ts';

// The same shared fixture stui and the shared views test: a long document with a table, a code
// block and nested lists, and a two-document link cycle.
const fixture = JSON.parse(readFileSync(new URL('../../fixtures/clients/documents-v1.json', import.meta.url), 'utf8'));
const long = fixture.links[0].text;
const bytes = text => [...new TextEncoder().encode(text)];
const hash = character => character.repeat(64);
const envelope = value => ({ value });
const failure = (status, code) => new ClientError({ api_version: 'st3.client.v0', code, message: code, details: {}, request_id: 'request/example' }, status);

// A reader that answers from a map of documents, the way document.get does: a bare name is its latest version.
function reader(documents, calls = []) {
  return {
    async documentGet(name, options) {
      calls.push(['documentGet', name, options]);
      const found = documents[name];
      if (found instanceof Error) throw found;
      if (!found) throw failure(404, 'not-found');
      return envelope({ reference: `${found.name}@${found.version}`, bytes: bytes(found.text), name: found.name, version: found.version, size: found.text.length, changed_at_unix_ms: found.changed, latest: true });
    },
    async documentsList(options, requestOptions) {
      calls.push(['documentsList', options, requestOptions]);
      const items = Object.values(documents).filter(item => !(item instanceof Error) && item.name.startsWith(options.prefix ?? 'doc/'))
        .sort((a, b) => a.name.localeCompare(b.name))
        .map(item => ({ reference: `${item.name}@${item.version}`, name: item.name, version: item.version, size: item.text.length, changed_at_unix_ms: item.changed, latest: true }));
      const start = options.cursor ? Number(options.cursor) : 0;
      const page = items.slice(start, start + 2);
      const more = start + 2 < items.length;
      return envelope({ items: page, has_more: more, next_cursor: more ? String(start + 2) : null, history: false, limit: options.limit });
    },
    async documentReferencesGet(subject, options) {
      calls.push(['documentReferencesGet', subject, options]);
      if (subject === 'mission-run/broken') throw new TypeError('network down');
      return envelope({ subject, complete: true, references: [
        { reference: 'doc/garden/plan', name: 'doc/garden/plan', version: null, source: 'step', step: 'step-run/garden/plant' },
        { reference: `doc/garden/seeds@${hash('a')}`, name: 'doc/garden/seeds', version: hash('a'), source: 'step', step: null },
      ] });
    },
  };
}

const garden = {
  'doc/garden/plan': { name: 'doc/garden/plan', version: hash('c'), text: long, changed: 1_700_000_000_000 },
  'doc/cycle/a': { name: 'doc/cycle/a', version: hash('1'), text: fixture.links[2].text, changed: 1_700_000_100_000 },
  'doc/cycle/b': { name: 'doc/cycle/b', version: hash('2'), text: fixture.links[3].text, changed: 1_700_000_200_000 },
};

test('opens the long document by name, unchanged, with its version, changed time and links', async () => {
  const calls = [];
  const signal = new AbortController().signal;
  const view = await openDocument(reader(garden, calls), 'doc/garden/plan', signal);
  assert.equal(view.state, 'ready');
  assert.equal(view.text, long, 'the text is shown as the author wrote it');
  assert.ok(view.text.includes('| North | Ada | doc/garden/beds/north |'));
  assert.ok(view.text.includes('```kdl\nstep "plant" {'));
  assert.ok(view.text.includes('     1. Water the beds'));
  assert.equal(view.text.split('\n').length > 400, true);
  assert.equal(view.shortVersion, 'cccccccccccc');
  assert.equal(view.changedAtUnixMs, 1_700_000_000_000);
  assert.deepEqual(view.links, fixture.links[0].links);
  assert.deepEqual(calls, [['documentGet', 'doc/garden/plan', { signal }]], 'one read, through document.get, cancellable');
  assert.match(versionLine(view), /^version cccccccccccc · latest · changed \d{4}-\d\d-\d\d \d\d:\d\d$/);
});

test('renders the long document: tables and code stay as written, nested lists keep their indent, links open', () => {
  const blocks = markdown(long);
  assert.ok(blocks.some(block => block.kind === 'table' && block.text === '| North | Ada | doc/garden/beds/north |'));
  assert.ok(blocks.some(block => block.kind === 'fence' && block.text === '```kdl'));
  assert.ok(blocks.some(block => block.kind === 'code' && block.text === '  document "doc/garden/seeds@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"'));
  const nested = blocks.find(block => block.kind === 'item' && block.runs.some(run => run.text.includes('Water the beds')));
  assert.equal(nested.indent, 5);
  assert.equal(blocks.filter(block => block.kind === 'text' && block.runs[0]?.text.startsWith('Line ')).length, 400);
  const opened = blocks.flatMap(block => block.runs ?? []).filter(run => run.document).map(run => run.document);
  assert.deepEqual(opened, ['doc/garden/checklist', `doc/garden/notes@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`, 'doc/garden/checklist', 'doc/garden/quote', 'doc/garden/last', 'doc/garden', 'doc/garden/short']);
  // Links inside a code span, a table or a code block stay as written there and are followed from
  // the document's links list, which holds every link once.
  const listed = fixture.links[0].links.map(link => link.reference);
  for (const reference of opened) assert.ok(listed.includes(reference), reference);
  for (const reference of ['doc/garden/soil.md', 'doc/garden/beds/north', `doc/garden/seeds@${hash('a')}`]) assert.ok(listed.includes(reference) && !opened.includes(reference), reference);
});

test('a doc/ reference in a message is a link that opens the document; look-alikes are not', () => {
  assert.deepEqual(inline('See doc/garden/plan.'), [{ text: 'See ', style: 'plain' }, { text: 'doc/garden/plan', style: 'link', document: 'doc/garden/plan' }, { text: '.', style: 'plain' }]);
  assert.deepEqual(inline('[the plan](doc/garden/plan)'), [{ text: 'the plan', style: 'link', url: undefined, document: 'doc/garden/plan' }]);
  assert.deepEqual(inline('mydoc/x and doc/ and doc/garden/short@abc'), [{ text: 'mydoc/x and doc/ and ', style: 'plain' }, { text: 'doc/garden/short', style: 'link', document: 'doc/garden/short' }, { text: '@abc', style: 'plain' }]);
  assert.equal(inline('https://example.com/doc/x').filter(run => run.document).length, 0);
});

test('following the link cycle returns along the trail instead of growing it', async () => {
  let trail = [fixture.trail.open[0]];
  const steps = [];
  for (const reference of fixture.trail.open.slice(1)) {
    const view = await openDocument(reader(garden), trail.at(-1));
    assert.ok(view.links.some(link => link.reference === reference), `${trail.at(-1)} links to ${reference}`);
    const step = followDocument(trail, reference);
    steps.push(step);
    if (step.kind === 'push') trail = step.trail;
    if (step.kind === 'back') trail = trail.slice(0, trail.length - step.count);
  }
  assert.deepEqual(trail, fixture.trail.expected);
  assert.deepEqual(steps.map(step => step.kind), ['push', 'back', 'push']);
  assert.deepEqual(followDocument(['doc/cycle/a'], 'doc/cycle/a'), { kind: 'stay' });
});

test('a document that cannot be shown says why, never a blank', async () => {
  for (const { status, code, state } of fixture.errors) {
    const error = status === null ? new TypeError('fetch failed') : failure(status, code);
    assert.equal(whyUnreadable(error), state, code);
    const view = await openDocument(reader({ 'doc/x': error }), 'doc/x');
    assert.equal(view.state, 'unavailable');
    assert.equal(view.why, state);
    assert.ok(view.explanation.includes('doc/x'));
  }
  const offline = await openDocument(null, 'doc/garden/plan');
  assert.equal(offline.why, 'remote-unavailable');
  const notText = await openDocument({ documentGet: async () => envelope({ reference: `doc/blob@${hash('d')}`, bytes: [0xff, 0xfe] }) }, 'doc/blob');
  assert.equal(notText.why, 'not-text');
  assert.equal((await openDocument(reader(garden), 'doc/garden/missing')).why, 'not-found');
});

test('the list pages under a prefix and keeps loading, empty and failed apart', async () => {
  const calls = [];
  const many = { ...garden, 'doc/garden/a': { name: 'doc/garden/a', version: hash('e'), text: 'a', changed: 1 }, 'doc/garden/b': { name: 'doc/garden/b', version: hash('f'), text: 'b', changed: 2 } };
  let list = emptyDocumentList('doc/garden/');
  assert.equal(documentListPlaceholder(list), 'Loading…');
  list = await nextDocumentPage(reader(many, calls), list);
  assert.deepEqual(list.rows.map(row => row.name), ['doc/garden/a', 'doc/garden/b']);
  assert.equal(list.nextCursor, '2');
  list = await nextDocumentPage(reader(many, calls), list);
  assert.deepEqual(list.rows.map(row => row.name), ['doc/garden/a', 'doc/garden/b', 'doc/garden/plan']);
  assert.equal(list.nextCursor, null);
  assert.equal(list.rows[2].shortVersion, 'cccccccccccc');
  assert.deepEqual(calls.map(call => call[1]), [{ prefix: 'doc/garden/', limit: 50 }, { prefix: 'doc/garden/', cursor: '2', limit: 50 }]);
  const empty = await nextDocumentPage(reader(many), emptyDocumentList('doc/orchard/'));
  assert.equal(documentListPlaceholder(empty), 'No documents under doc/orchard/.');
  const failed = await nextDocumentPage(null, emptyDocumentList());
  assert.equal(documentListPlaceholder(failed), 'The host did not answer, so the documents cannot be listed now.');
});

test('a mission names its documents once, with the step that pinned them; a failed read is reported', async () => {
  const found = await referencedDocuments(reader(garden), ['mission/garden', 'mission-run/garden/1']);
  assert.deepEqual(found.references.map(reference => [reference.reference, reference.step]), [['doc/garden/plan', 'step-run/garden/plant'], [`doc/garden/seeds@${hash('a')}`, null]]);
  assert.equal(found.failed, null);
  const partial = await referencedDocuments(reader(garden), ['mission/garden', 'mission-run/broken']);
  assert.equal(partial.references.length, 2);
  assert.equal(partial.failed, 'remote-unavailable');
});

test('an older st that does not serve documents is told apart from one that has not said yet', () => {
  assert.equal(documentsServed(null), null);
  assert.equal(documentsServed({ capabilities: [] }), false);
  assert.equal(documentsServed({ capabilities: [{ id: 'documents', version: 1, state: 'ungranted' }] }), false);
  assert.equal(documentsServed({ capabilities: [{ id: 'documents', version: 1, state: 'granted' }] }), true);
  assert.equal(changedAt(null), null);
});
