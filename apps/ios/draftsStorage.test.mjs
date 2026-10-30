import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DRAFTS_KEY, LocalDrafts } from './draftsStorage.ts';

test('local draft content stays separate, restores only for the same actor and keeps newly typed text', async () => {
  const values = new Map(), storage = { async getItem(key) { return values.get(key) ?? null; }, async setItem(key, value) { values.set(key, value); } };
  const drafts = new LocalDrafts(storage);
  await drafts.save('person/ada', new Map([['agent/example', 'private draft words']]));
  const same = new Map([['agent/example', 'new typing']]); assert.equal(await drafts.restore('person/ada', same), 0); assert.equal(same.get('agent/example'), 'new typing');
  assert.equal(await drafts.restore('person/avery', new Map()), 0);
  const restored = new Map(); assert.equal(await drafts.restore('person/ada', restored), 1); assert.equal(restored.get('agent/example'), 'private draft words');
  assert.deepEqual([...values.keys()], [DRAFTS_KEY]);
  await drafts.save('person/ada', new Map()); assert.equal(await drafts.restore('person/ada', new Map()), 0);
});
test('draft storage is bounded, actor changes during an async read cancel recovery and failures are ignored', async () => {
  let raw = null;
  const storage = { async getItem() { return raw; }, async setItem(_key, value) { raw = value; } };
  const drafts = new LocalDrafts(storage);
  await drafts.save('person/ada', new Map(Array.from({ length: 40 }, (_, i) => [`agent/example-${i}`, `text-${i}`])));
  const restored = new Map(); assert.equal(await drafts.restore('person/ada', restored), 24);
  assert.equal(await drafts.restore('person/ada', new Map(), () => false), 0);
  const broken = new LocalDrafts({ getItem() { throw new Error('unavailable'); }, setItem() { throw new Error('unavailable'); } });
  assert.equal(await broken.restore('person/ada', new Map()), 0); await broken.save('person/ada', restored); await broken.clear();
});

test('edits while one storage write is pending maintain one latest view, not a snapshot backlog', async () => {
  const writes = [], releases = [];
  const storage = { async getItem() { return null; }, setItem(_key, raw) { writes.push(raw); return new Promise(resolve => releases.push(resolve)); } };
  const drafts = new LocalDrafts(storage), view = new Map([['agent/example', 'first']]);
  const writing = drafts.save('person/ada', view); await Promise.resolve();
  assert.equal(writes.length, 1);
  for (let i = 0; i < 100; i++) { view.set('agent/example', `edit-${i}`); drafts.save('person/ada', view); }
  assert.equal(writes.length, 1);
  releases.shift()(); await Promise.resolve(); await Promise.resolve();
  assert.equal(writes.length, 2); assert.equal(JSON.parse(writes[1]).entries[0][1], 'edit-99');
  releases.shift()(); await writing;
});
