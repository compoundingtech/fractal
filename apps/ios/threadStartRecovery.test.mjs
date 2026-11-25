import test from 'node:test';
import assert from 'node:assert/strict';
import { importSavedThreadStarts, startRecoveryKey } from './threadStartRecovery.ts';
const start = { target: 'thread/original', title: 'Original title', origin: { agentId: 'agent/dotfiles/manager', sourceId: 'notice/original', episode: 'episode/original' } };
const key = 'thread-draft/start/' + start.target;
const draft = JSON.stringify({ text: 'original text', quote: { id: 'message/quote', text: 'kept quote' } });
const makeCache = () => new Map([[key, draft], [startRecoveryKey(start.target), JSON.stringify(start)]]);

test('saved Start migration commits packetless content before deleting source and is restart-idempotent', async () => {
  const cache = makeCache(), rows = new Map(), events = [];
  const commit = async entry => { assert.equal(cache.get(key), draft); events.push('commit'); if (!rows.has(entry.id)) rows.set(entry.id, structuredClone(entry)); return true; };
  await importSavedThreadStarts(cache, () => true, commit, async () => { events.push('save'); assert.equal(cache.has(key), false); });
  assert.deepEqual(events, ['commit', 'save']);
  const [row] = rows.values();
  assert.equal(row.id, 'legacy-start/' + start.target);
  assert.equal(row.serializedDraft, draft);
  assert.equal(row.text, 'original text');
  assert.deepEqual(row.message, { kind: 'start', target: start.target, title: start.title, origin: start.origin });
  assert.equal('packet' in row, false); assert.equal('expectedIdentities' in row, false);
  await importSavedThreadStarts(makeCache(), () => true, async entry => { if (!rows.has(entry.id)) rows.set(entry.id, entry); return true; }, async () => {});
  assert.equal(rows.size, 1);
});

test('commit failure, source edit and owner switch never delete or retarget the old saved Start', async () => {
  const cache = makeCache(), original = new Map(cache);
  await assert.rejects(importSavedThreadStarts(cache, () => true, async () => { throw new Error('disk full'); }, async () => assert.fail('no source deletion')), /disk full/);
  assert.deepEqual(cache, original);
  await importSavedThreadStarts(cache, () => true, async () => { cache.set(key, 'later edit'); return true; }, async () => assert.fail('changed draft must stay'));
  assert.equal(cache.get(key), 'later edit');
  let current = true;
  const switched = makeCache();
  await importSavedThreadStarts(switched, () => current, async () => { current = false; return true; }, async () => assert.fail('owner changed'));
  assert.deepEqual(switched, original);
});

test('failed removal save restores sources; malformed metadata/drafts remain unconverted', async () => {
  const cache = makeCache(), original = new Map(cache);
  await assert.rejects(importSavedThreadStarts(cache, () => true, async () => true, async () => { throw new Error('save failed'); }), /save failed/);
  assert.deepEqual(cache, original);
  for (const broken of [new Map([[key, draft], [startRecoveryKey(start.target), '{broken']]), new Map([[key, '{broken'], [startRecoveryKey(start.target), JSON.stringify(start)]])]) {
    const before = new Map(broken);
    await importSavedThreadStarts(broken, () => true, async () => assert.fail('cannot invent valid metadata'), async () => assert.fail('cannot delete malformed source'));
    assert.deepEqual(broken, before);
  }
});
