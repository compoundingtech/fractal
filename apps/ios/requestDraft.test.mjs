import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clearRequestDraft, readRequestDraft, requestDismissalText, requestDraftKey, requestOptionNotes, writeRequestDraft, readPromptChoices, writePromptChoices } from './requestDraft.ts';
const item = { id: 'attention/example', revision: 'v1', request: { answers: [{ id: 'yes', label: 'Ship' }, { id: 'no', label: 'Hold' }] } };
test('request draft survives remount, retains every note exactly, and does not cross revisions', () => {
  const cache = new Map();
  const draft = { choice: { kind: 'option', id: 'yes' }, text: '', notes: { yes: '  selected note  ', no: 'other note' } };
  writeRequestDraft(cache, item, draft);
  assert.deepEqual(readRequestDraft(cache, item), draft);
  assert.deepEqual(requestOptionNotes(draft, 'yes'), [{ id: 'no', text: 'other note' }]);
  assert.deepEqual(requestOptionNotes(draft).map(note => note.id), ['yes', 'no']);
  assert.equal(requestDismissalText(cache, item), 'Ship:   selected note  \nHold: other note');
  assert.equal(readRequestDraft(cache, { ...item, revision: 'v2' }).choice, undefined);
});
test('confirmed receipt only clears the exact captured draft; refusal requires no draft mutation', () => {
  const cache = new Map();
  writeRequestDraft(cache, item, { text: 'first', notes: {} });
  const sent = cache.get(requestDraftKey(item));
  writeRequestDraft(cache, item, { text: 'later edit', notes: {} });
  clearRequestDraft(cache, item, sent);
  assert.equal(readRequestDraft(cache, item).text, 'later edit');
  clearRequestDraft(cache, item, cache.get(requestDraftKey(item)));
  assert.equal(cache.has(requestDraftKey(item)), false);
});
test('malformed saved drafts are ignored without evicting earlier unanswered drafts', () => {
  const cache = new Map([[requestDraftKey(item), '{"text":"x","notes":{"yes":1}}']]);
  assert.deepEqual(readRequestDraft(cache, item), { text: '', notes: {} });
  for (let i = 0; i < 30; i++) writeRequestDraft(cache, { id: `attention/${i}`, revision: 'v1' }, { text: `draft ${i}`, notes: {} });
  assert.equal(cache.size, 31);
  assert.equal(cache.has(requestDraftKey(item)), true);
});

test('harness multiselect drafts retain all selections across remount without accepting malformed indices', () => {
  const cache = new Map();
  const key = 'prompt:attention/example:call-v1';
  writePromptChoices(cache, key, [[0, 2], [1]]);
  assert.deepEqual(readPromptChoices(cache, key), [[0, 2], [1]]);
  assert.deepEqual(readPromptChoices(cache, 'prompt:attention/example:call-v2'), []);
  cache.set(key, '[[0,-1]]');
  assert.deepEqual(readPromptChoices(cache, key), []);
});
