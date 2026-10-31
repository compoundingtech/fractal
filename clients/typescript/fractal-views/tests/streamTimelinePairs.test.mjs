import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { conversationEntries } from '@fractal/views/conversationView';
import { applyConversation } from '@fractal/views/sessionView';

// Two stream items that arrived at the same graph index in the same millisecond, as Fractal
// serves them (crates/fractal-api-client-v0 pins this file to its conversation page). A phone
// already installed sorts the page and pairs each message with the content entry after it.
const fixture = JSON.parse(readFileSync(new URL('../../../../fixtures/clients/stream-timeline-pairs-v1.json', import.meta.url), 'utf8'));

const mail = items => conversationEntries(applyConversation(undefined, { replace: true, items, hasMore: false }).entries, new Map())
  .filter(entry => entry.body.kind === 'mail')
  .map(entry => [entry.id, entry.body.subject, entry.body.text]);

test('each stream item keeps its own text, in any arrival order', () => {
  const expected = [['message/00000000000000d1', 'first', 'first'], ['message/00000000000000d2', 'second', 'second']];
  assert.deepEqual(mail(fixture.items), expected);
  assert.deepEqual(mail([...fixture.items].reverse()), expected);
});

test('the shown times are what keeps them apart', () => {
  // Served at one millisecond, both message entries would sort before both content entries.
  const tied = fixture.items.map(item => ({ ...item, timestamp: fixture.items[0].timestamp }));
  assert.notDeepEqual(mail(tied), mail(fixture.items));
});
