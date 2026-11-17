import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readMarkedUnread } from './unreadPaging.ts';

const pages = values => {
  const requested = [];
  return {
    requested,
    fetch: async cursor => {
      requested.push(cursor);
      assert.ok(requested.length <= values.length, 'fetched beyond cursor exhaustion');
      return values[requested.length - 1];
    },
  };
};

test('empty pages with continuation do not hide later marked-unread items', async () => {
  const row = { id: 'thread/reminder' };
  const source = pages([
    { items: [], cursor: 'opaque/first' },
    { items: [row], cursor: 'opaque/second' },
    { items: [], cursor: 'opaque/third' },
    { items: [] },
  ]);
  assert.deepEqual(await readMarkedUnread(source.fetch), { kind: 'items', items: [row] });
  assert.deepEqual(source.requested, [undefined, 'opaque/first', 'opaque/second', 'opaque/third']);
});

test('empty is shown only after an empty continuation page exhausts its cursor', async () => {
  const next = Promise.withResolvers();
  let requested = 0, settled = false;
  const result = readMarkedUnread(async cursor => {
    requested++;
    if (requested === 1) { assert.equal(cursor, undefined); return { items: [], cursor: 'opaque/continue' }; }
    assert.equal(cursor, 'opaque/continue');
    return next.promise;
  }).then(value => { settled = true; return value; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(requested, 2);
  assert.equal(settled, false, 'a cursor still exists, so empty is not an answer yet');
  next.resolve({ items: [] });
  assert.deepEqual(await result, { kind: 'empty' });
});

test('a repeated cursor, including a multi-page cycle, is a typed error', async () => {
  for (const cursors of [['a', 'a'], ['a', 'b', 'a']]) {
    const source = pages(cursors.map(cursor => ({ items: [], cursor })));
    assert.deepEqual(await readMarkedUnread(source.fetch), {
      kind: 'error', error: { kind: 'repeated-cursor', cursor: 'a' }, items: [],
    });
    assert.equal(source.requested.length, cursors.length);
  }
});

test('reaching the page bound returns a typed error rather than empty or a complete partial list', async () => {
  const row = { id: 'thread/partial' };
  const source = pages([{ items: [row], cursor: 'first' }, { items: [], cursor: 'second' }]);
  assert.deepEqual(await readMarkedUnread(source.fetch, 2), {
    kind: 'error', error: { kind: 'page-bound', maxPages: 2 }, items: [row],
  });
  assert.equal(source.requested.length, 2);
});

test('fetch failure midway retains partial rows and its cause, never an empty state', async () => {
  const cause = new Error('page unavailable');
  for (const items of [[], [{ id: 'thread/partial' }]]) {
    const requested = [];
    const result = await readMarkedUnread(async cursor => {
      requested.push(cursor);
      if (cursor === undefined) return { items, cursor: 'opaque/next' };
      throw cause;
    });
    assert.deepEqual(result, { kind: 'error', error: { kind: 'fetch-failed', cause }, items });
    assert.deepEqual(requested, [undefined, 'opaque/next']);
  }
});

test('invalid page bounds fail before fetching', async () => {
  for (const maxPages of [0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    const result = await readMarkedUnread(async () => assert.fail('invalid bound reached fetch'), maxPages);
    assert.equal(result.kind, 'error');
    assert.equal(result.error.kind, 'invalid-page-bound');
    assert.equal(Object.is(result.error.maxPages, maxPages), true);
  }
});

test('property: any empty-page prefix within the bound is followed to exhaustion', async () => {
  for (let length = 0; length < 20; length++) for (const finalItems of [[], [{ id: 'thread/found' }]]) {
    const source = pages([
      ...Array.from({ length }, (_, index) => ({ items: [], cursor: 'opaque/' + index })),
      { items: finalItems },
    ]);
    assert.deepEqual(await readMarkedUnread(source.fetch, length + 1),
      finalItems.length ? { kind: 'items', items: finalItems } : { kind: 'empty' });
    assert.equal(source.requested.length, length + 1);
  }
});
