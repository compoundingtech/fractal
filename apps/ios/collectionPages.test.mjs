import assert from 'node:assert/strict';
import { listCollectionPages, mergedRows } from './collectionPages.ts';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { test } from 'node:test';
import ts from 'typescript';
import { API_VERSION, FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { OLDER_PAGE, readOlder } from '@fractal/views/sessionView';

const snapshot = { id: 'snapshot/1', host_id: 'host/a', store_index: 1, projection_version: 'client-projection.v0' };
const result = (items, more, next) => ({ snapshot, value: { items, page: { has_more: more, next_cursor: next } } });
const calls = [];
const complete = await listCollectionPages(async ({ cursor }) => {
  calls.push(cursor);
  return cursor ? result([{ id: 'attention/2' }], false, null) : result([{ id: 'attention/1' }], true, 'next');
}, 30);
assert.deepEqual(calls, [undefined, 'next']);
assert.equal(complete.truncated, false);
assert.equal(complete.pages.flatMap(page => page.value.items).length, 2);

let boundedCalls = 0;
const bounded = await listCollectionPages(async () => result([], true, `next-${++boundedCalls}`), 30, 3);
assert.equal(boundedCalls, 3);
assert.equal(bounded.truncated, true);

let firstPages = 0, secondPages = 0;
const retried = await listCollectionPages(async ({ cursor }) => {
  if (!cursor) { firstPages++; return result([], true, 'next'); }
  secondPages++;
  if (secondPages === 1) throw { response: { code: 'page-cursor-expired' } };
  return result([], false, null);
}, 30);
assert.equal(retried.truncated, false);
assert.equal(firstPages, 2);
assert.equal(secondPages, 2);
await assert.rejects(listCollectionPages(async () => result([], true, 'same'), 30), /did not advance/);

// A cursor gap restarts the listing like an expired cursor, never as a bare error (410, 2026-10-06).
let gapped = 0;
const afterGap = await listCollectionPages(async ({ cursor }) => {
  if (!cursor) return result([{ id: 'agent/a' }], true, 'next');
  if (++gapped === 1) throw { response: { code: 'cursor-gap' } };
  return result([{ id: 'agent/b' }], false, null);
}, 30);
assert.equal(gapped, 2);
assert.deepEqual(afterGap.pages.flatMap(page => page.value.items).map(item => item.id), ['agent/a', 'agent/b']);

// A row that moved while the pages were read is on two of them: it shows once, newest copy, first place.
assert.deepEqual(
  mergedRows([{ id: 'a', n: 1 }, { id: 'b', n: 1 }, { id: 'a', n: 2 }, { id: 'c', n: 1 }]),
  [{ id: 'a', n: 2 }, { id: 'b', n: 1 }, { id: 'c', n: 1 }],
);
assert.deepEqual(mergedRows([{ n: 1 }, { n: 2 }]), [{ n: 1 }, { n: 2 }], 'rows without an id are kept');

// Run the production store method without loading React Native into Node, as in
// observationReporting.test.mjs. Keep the real history reader and HTTP request builder.
const source = ts.createSourceFile('store.tsx', readFileSync(new URL('./store.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const methods = [];
const visit = node => {
  if (ts.isMethodDeclaration(node) && node.name.getText(source) === 'olderTimeline') methods.push(node);
  ts.forEachChild(node, visit);
};
visit(source);
assert.equal(methods.length, 1);
const bindOlderTimeline = new Function('client', 'status', 'readOlder', 'OLDER_PAGE', 'errorText',
  ts.transpile(`return ({ ${methods[0].getText(source)} }).olderTimeline;`, { target: ts.ScriptTarget.ES2022 }));

test('iOS timeline continuations send only the opaque cursor; initial reads retain their limit', async t => {
  const requests = [];
  const server = createServer((request, response) => {
    const url = new URL(request.url, 'http://localhost');
    const value = url.pathname === '/v1/client/capabilities'
      ? { limits: { max_page_items: OLDER_PAGE } }
      : { items: [], page: { limit: 5, has_more: false, next_cursor: null } };
    if (url.pathname !== '/v1/client/capabilities') requests.push(url);
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ api_version: API_VERSION, snapshot, value }));
  });
  t.after(() => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())));
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const client = new FractalClient({ baseUrl: `http://127.0.0.1:${server.address().port}` });
  const olderTimeline = bindOlderTimeline(client, 'online', readOlder, OLDER_PAGE, error => String(error));
  const cursor = 'opaque/served-limit-5+token=';
  const continued = await olderTimeline('session/ios-history', { cursor: { value: cursor, at: Date.now() } }, undefined);
  assert.deepEqual(continued, { items: [], hasMore: false, cursor: undefined });
  assert.equal(requests.length, 1);
  assert.equal(requests[0].pathname, '/v1/client/sessions/ios-history/timeline');
  assert.deepEqual([...requests[0].searchParams], [['cursor', cursor]]);

  await olderTimeline('session/ios-history', {}, undefined);
  assert.equal(requests.length, 2);
  assert.deepEqual([...requests[1].searchParams], [['limit', String(OLDER_PAGE)]]);

  await olderTimeline('session/ios-history', { cursor: { value: 'expired', at: 0 } }, undefined);
  assert.equal(requests.length, 3);
  assert.deepEqual([...requests[2].searchParams], [['limit', String(OLDER_PAGE)]]);
});
