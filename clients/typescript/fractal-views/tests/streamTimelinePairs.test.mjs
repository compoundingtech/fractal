import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { conversationEntries } from '@fractal/views/conversationView';
import { applyConversation } from '@fractal/views/sessionView';

// Two stream items that arrived at the same graph index in the same millisecond, as Fractal
// serves them (crates/fractal-api-client-v0 pins this file to its conversation page). Today's
// views pair content with its header by message_id. A phone installed before that sorts the
// page and pairs each message with the content entry after it; it reads the page as if content
// named no message.
const fixture = JSON.parse(readFileSync(new URL('../../../../fixtures/clients/stream-timeline-pairs-v1.json', import.meta.url), 'utf8'));
const unnamed = items => items.map(item => item.type === 'content'
  ? { ...item, body: Object.fromEntries(Object.entries(item.body).filter(([key]) => key !== 'message_id')) }
  : item);

const mail = items => conversationEntries(applyConversation(undefined, { replace: true, items, hasMore: false }).entries, new Map())
  .filter(entry => entry.body.kind === 'mail')
  .map(entry => [entry.id, entry.body.subject, entry.body.text]);

test('each stream item keeps its own text, in any arrival order, on new and older phones', () => {
  const expected = [['message/00000000000000d1', 'first', 'first'], ['message/00000000000000d2', 'second', 'second']];
  for (const items of [fixture.items, unnamed(fixture.items)]) {
    assert.deepEqual(mail(items), expected);
    assert.deepEqual(mail([...items].reverse()), expected);
  }
});

test('the shown times are what keeps them apart on a phone that pairs by position', () => {
  // Served at one millisecond, both message entries would sort before both content entries.
  const tied = fixture.items.map(item => ({ ...item, timestamp: fixture.items[0].timestamp }));
  assert.notDeepEqual(mail(unnamed(tied)), mail(fixture.items));
  // Content that names its message pairs right even then; only the rows' order follows the IDs.
  const pairs = items => mail(items).sort((left, right) => left[0].localeCompare(right[0]));
  assert.deepEqual(pairs(tied), pairs(fixture.items));
});
