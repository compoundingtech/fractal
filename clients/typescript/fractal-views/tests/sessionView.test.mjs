import assert from 'node:assert/strict';
import { applyConversation, applyOlderPage, isSnapshotChurn, isUnmanaged, isUnresolved, listSessionPages, olderNote, readOlder, recentTimeline, sessionLabel, timelineText } from '@fractal/views/sessionView';

const managed = { id: 'session/managed', kind: 'session', owner_id: 'agent/one', state: 'running' };
const exact = { id: 'session/exact', kind: 'session', owner_id: 'external-session/codex/one', state: 'running', managed: false, driver: 'codex', native_session_id: 'one' };
const unresolved = { id: 'session/process', kind: 'session', owner_id: 'external-process/claude/42', state: 'running', managed: false, driver: 'claude', native_session_id: null, process: { pid: 42, exact_session: false } };
assert.equal(isUnmanaged(managed), false);
assert.equal(isUnmanaged(exact), true);
assert.equal(isUnresolved(exact), false);
assert.equal(isUnresolved(unresolved), true);
assert.match(sessionLabel(exact, 'gateway-host'), /Undeclared.*codex session.*gateway-host/);
assert.match(sessionLabel(unresolved, 'gateway-host'), /Undeclared.*unresolved claude process.*gateway-host/);

const calls = [];
const sessions = await listSessionPages(async options => {
  calls.push(options);
  return { value: { items: options.cursor ? [unresolved] : [managed, exact], page: { has_more: !options.cursor, next_cursor: options.cursor ? null : 'next' } } };
}, 2);
assert.deepEqual(sessions.map(session => session.id), ['session/managed', 'session/exact', 'session/process']);
assert.deepEqual(calls, [{ limit: 2, cursor: undefined, history: false }, { limit: 2, cursor: 'next', history: false }]);
await assert.rejects(listSessionPages(async () => ({ value: { items: [], page: { has_more: true, next_cursor: 'same' } } }), 2), /did not advance/);
let boundedCalls = 0;
await listSessionPages(async () => {
  boundedCalls++;
  return { value: { items: [exact], page: { has_more: true, next_cursor: `page-${boundedCalls}` } } };
}, 2);
assert.equal(boundedCalls, 5);
const expired = { response: { code: 'page-cursor-expired' } };
assert.equal(isSnapshotChurn(expired), true);
assert.equal(isSnapshotChurn(new Error('offline')), false);
let firstPages = 0, secondPages = 0;
const restarted = await listSessionPages(async options => {
  if (!options.cursor) {
    firstPages++;
    return { value: { items: [managed], page: { has_more: true, next_cursor: 'next' } } };
  }
  secondPages++;
  if (secondPages === 1) throw expired;
  return { value: { items: [exact], page: { has_more: false, next_cursor: null } } };
}, 2);
assert.equal(firstPages, 2);
assert.equal(secondPages, 2);
assert.deepEqual(restarted.map(session => session.id), ['session/managed', 'session/exact']);
const mixedTimeline = [
  { id: 'old-status', sequence: 177576, timestamp: '2026-10-04T17:15:16Z' },
  { id: 'native-answer', sequence: 1682, timestamp: '2026-10-04T21:40:10Z' },
  { id: 'usage', sequence: 194964, timestamp: '2026-10-04T21:40:11.000Z' },
  { id: 'tool-call', sequence: 1684, timestamp: '2026-10-04T21:40:19.000Z' },
];
assert.deepEqual(recentTimeline(mixedTimeline, 3).map(entry => entry.id), ['native-answer', 'usage', 'tool-call']);
assert.deepEqual(recentTimeline([
  { id: 'call', sequence: 2, timestamp: '2026-10-04T21:40:19Z' },
  { id: 'prose', sequence: 1, timestamp: '2026-10-04T23:40:19+02:00' },
  { id: 'later', sequence: 0, timestamp: '2026-10-04T21:40:19.100Z' },
]).map(entry => entry.id), ['prose', 'call', 'later']);

// Equivalent RFC3339 spellings tie by source sequence, then ID, never namespace or
// input order. Distinct nanoseconds must not collapse to Date.parse's millisecond.
const chronology = [
  { id: 'native/z', sequence: 7, timestamp: '2026-10-04T21:40:19.000000200Z' },
  { id: 'graph/a', sequence: 7, timestamp: '2026-10-04T16:40:19.0000002-05:00' },
  { id: 'native/early', sequence: 999, timestamp: '2026-10-04T23:40:19.000000100+02:00' },
  { id: 'graph/zero', sequence: 5000, timestamp: '2026-10-04T21:40:19Z' },
  { id: 'native/tie', sequence: 6, timestamp: '2026-10-04T21:40:19.000000200Z' },
];
const chronologicalIds = ['graph/zero', 'native/early', 'native/tie', 'graph/a', 'native/z'];
const originalChronology = structuredClone(chronology);
assert.deepEqual(recentTimeline(chronology).map(entry => entry.id), chronologicalIds);
assert.deepEqual(recentTimeline([...chronology].reverse()).map(entry => entry.id), chronologicalIds);
assert.deepEqual(chronology, originalChronology, 'ordering never rewrites source counters or timestamps');

const conversational = chronology.map(entry => ({ ...entry, type: 'content', body: { text: entry.id } }));
let chronologicalConversation = applyConversation(undefined, {
  replace: true, items: conversational.slice(0, 2), hasMore: true,
  sessionId: 'session/chronology', olderCursor: 'frozen-first',
}, 2, 10);
const chronologicalRequest = { sessionId: chronologicalConversation.sessionId, historyVersion: chronologicalConversation.historyVersion };
chronologicalConversation = applyOlderPage(chronologicalConversation, chronologicalRequest, {
  items: conversational.slice(2, 4), hasMore: true, cursor: 'frozen-next',
}, 20);
const frozenOlder = chronologicalConversation.older;
const revisedNative = { ...conversational[0], body: { text: 'revised in place' } };
chronologicalConversation = applyConversation(chronologicalConversation, {
  replace: false, items: [revisedNative, conversational[4]], hasMore: false,
}, 2);
assert.deepEqual(chronologicalConversation.entries.map(entry => entry.id), chronologicalIds);
assert.equal(chronologicalConversation.entries.at(-1), revisedNative);
assert.equal(chronologicalConversation.newestSequence, 7, 'older history does not advance the live frame high-water');
assert.equal(chronologicalConversation.entries.find(entry => entry.id === 'graph/zero').sequence, 5000, 'presentation does not renumber independent counters');
assert.equal(chronologicalConversation.older, frozenOlder, 'deltas preserve the frozen older-page cursor');
assert.equal(chronologicalConversation.historyVersion, chronologicalRequest.historyVersion);
const refreshedChronology = applyConversation(chronologicalConversation, {
  replace: true, items: [revisedNative], hasMore: true, olderCursor: 'fresh-first',
}, 2, 30);
assert.deepEqual(refreshedChronology.older.cursor, { value: 'fresh-first', at: 30 });
assert.equal(applyOlderPage(refreshedChronology, chronologicalRequest, {
  items: conversational, hasMore: false,
}), refreshedChronology, 'a frozen page from before replacement cannot reappear');

const numericPageCalls = [];
const numericOlder = await readOlder(async cursor => {
  numericPageCalls.push(cursor);
  return cursor
    ? { items: [conversational[2]], hasMore: false }
    : { items: [conversational[0]], hasMore: true, cursor: 'numeric-older' };
}, {}, conversational[1]);
assert.deepEqual(numericPageCalls, [undefined, 'numeric-older']);
assert.equal(numericOlder.items[0], conversational[2]);
assert.equal(timelineText({ media_type: 'text/plain', text: 'Hello' }), 'Hello');
assert.equal(timelineText({ message_id: 'metadata-only' }), null);
assert.equal(olderNote({ older: { loading: false }, hasOlder: false, entries: [{ sequence: 1 }] }), 'Start of this session · earlier ones: fractal conversations sessions');
await assert.rejects(
  readOlder(async () => ({ items: [{ sequence: 2 }], hasMore: true, cursor: 'c' }), {}, { sequence: 1 }),
  (error) => error instanceof Error && error.message === 'this session is too long to read further back here; fractal conversations timeline reads it all',
);
