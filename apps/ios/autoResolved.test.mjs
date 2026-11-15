import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { AUTO_RESOLVED_TITLE, OVERRIDE_TITLE, autoResolvedRows, closedOverride, overrideChoices, overrideCopy, overrideKey, overrideRefusal, overrideRequest, overrideRetryable } from './autoResolved.ts';

const now = Date.parse('2026-11-15T12:00:00Z');
const episode = 'a'.repeat(64);
const request = {
  version: 1, type: 'choice', question: 'Which region should the example deploy to?', why_person: 'Only you know the budget.',
  answers: [{ id: 'eu', label: 'Europe' }, { id: 'us', label: 'United States' }, { id: 'ap', label: 'Asia Pacific' }],
};
const ask = (extra = {}) => ({
  resolution_id: 'b'.repeat(64), title: 'Pick the example region', requester_id: 'agent/example/atlas',
  resolved_at: '2026-11-15T09:00:00Z', target_id: 'step-run/example/region', episode,
  default_answer: { id: 'eu', label: 'Europe' }, request,
  action_parameters: { 'work.override': { target_id: 'step-run/example/region', episode } },
  ...extra,
});
const without = (key) => { const value = ask(); delete value[key]; return value; };
const name = id => id === 'agent/example/atlas' ? 'Atlas' : id;

test('a row shows the title, the applied default, the asker and when it resolved', () => {
  const [row] = autoResolvedRows([ask()], name, now);
  assert.equal(row.key, 'b'.repeat(64));
  assert.equal(row.title, 'Pick the example region');
  assert.equal(row.asker, 'Atlas');
  assert.equal(row.context, 'Default: Europe · From: Atlas · Resolved 3h ago');
  assert.deepEqual(row.choices, [{ id: 'us', label: 'United States' }, { id: 'ap', label: 'Asia Pacific' }], 'every named answer but the default');
  assert.equal(AUTO_RESOLVED_TITLE, 'Auto-resolved');
  assert.equal(OVERRIDE_TITLE, 'Use my answer instead');
});

test('rows keep Fractal’s order and name an unknown asker by ID', () => {
  const rows = autoResolvedRows([ask(), ask({ resolution_id: 'c'.repeat(64), requester_id: 'agent/example/other' })], name, now);
  assert.deepEqual(rows.map(row => row.key), ['b'.repeat(64), 'c'.repeat(64)]);
  assert.equal(rows[1].asker, 'agent/example/other');
});

test('the override is hidden without action parameters, without a request or with no other named answer', () => {
  assert.deepEqual(overrideChoices(without('action_parameters')), []);
  assert.deepEqual(overrideChoices(ask({ action_parameters: {} })), []);
  assert.deepEqual(overrideChoices(without('request')), []);
  assert.deepEqual(overrideChoices(ask({ request: { ...request, answers: [{ id: 'eu', label: 'Europe' }] } })), []);
  const row = autoResolvedRows([without('action_parameters')], name, now)[0];
  assert.deepEqual(row.choices, [], 'the row still lists, without the override');
});

test('an ask that timed out without a default says so', () => {
  const [row] = autoResolvedRows([without('default_answer')], name, now);
  assert.equal(row.context, 'No default · From: Atlas · Resolved 3h ago');
});

test('the copy is verbatim', () => {
  assert.equal(overrideCopy('Atlas'), 'Your answer replaces the default. Work already done on the default is not undone; Atlas is told.');
});

test('work.override is fenced by the snapshot and keyed by episode and answer, the same on every try', () => {
  const fence = { snapshot_id: 'snapshot/example', subject_revisions: {} };
  const sent = overrideRequest(ask(), 'us', fence);
  assert.deepEqual(sent, {
    id: `action/person-override:${episode}:us`,
    idempotency_key: `person-override:${episode}:us`,
    fence: { snapshot_id: 'snapshot/example', subject_revisions: {} },
    parameters: { target_id: 'step-run/example/region', episode, answer: { id: 'us' } },
  });
  assert.equal(overrideKey(episode, 'us'), `person-override:${episode}:us`);
  const again = overrideRequest(ask(), 'us', { snapshot_id: 'snapshot/later', subject_revisions: {} });
  assert.equal(again.id, sent.id);
  assert.equal(again.idempotency_key, sent.idempotency_key);
  assert.throws(() => overrideRequest(without('action_parameters'), 'us', fence));
});

test('a recently closed item matching an overridable ask by episode gets the override', () => {
  const rows = autoResolvedRows([ask()], name, now);
  const closed = { id: 'attention/example/region', episode, closedElsewhere: true };
  assert.equal(closedOverride(closed, rows), rows[0]);
  assert.equal(closedOverride({ ...closed, closedElsewhere: false }, rows), undefined, 'only closed items');
  assert.equal(closedOverride({ ...closed, episode: 'd'.repeat(64) }, rows), undefined);
  assert.equal(closedOverride({ ...closed, episode: undefined }, rows), undefined);
  assert.equal(closedOverride(closed, autoResolvedRows([without('action_parameters')], name, now)), undefined, 'hidden once the window closed');
});

const refused = (code, details = {}) => new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code, message: code, retryable: false, details }, code === 'forbidden' ? 403 : 409);

test('a stale fence is not tried again and reads as the window closing; other refusals read plainly', () => {
  assert.equal(overrideRetryable(refused('stale-fence')), false);
  assert.equal(overrideRetryable(refused('rate-limited')), true);
  assert.equal(overrideRetryable(refused('internal', { applied: 'unknown' })), true, 'the identical action returns the first answer');
  assert.equal(overrideRetryable(refused('forbidden')), false);
  assert.match(overrideRefusal(refused('stale-fence')), /override window closed, or it was already answered/);
  assert.equal(overrideRefusal(refused('forbidden')), 'Only the person who was asked can replace its default.');
});

const source = path => ts.createSourceFile(path, readFileSync(new URL(path, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nodes = (root, predicate) => {
  const result = [];
  const visit = node => { if (predicate(node)) result.push(node); ts.forEachChild(node, visit); };
  visit(root);
  return result;
};
const calls = (root, expression) => nodes(root, node => ts.isCallExpression(node) && node.expression.getText() === expression);

test('the store reads auto-resolved asks without a timer and re-reads after an override', () => {
  const store = source('./store.tsx');
  const read = nodes(store, node => ts.isVariableDeclaration(node) && node.name.getText() === 'readAutoResolved')[0];
  assert.ok(read);
  assert.equal(calls(read, 'client.autoResolvedList').length, 1);
  assert.ok(nodes(read, ts.isIfStatement).some(node => node.expression.getText().includes("caps?.session_actor.startsWith('person/')")), 'only a person session reads its own asks');
  assert.equal(nodes(store, node => ts.isIdentifier(node) && node.text === 'setInterval').length, 0, 'no polling');
  const override = nodes(store, node => ts.isMethodDeclaration(node) && node.name.getText() === 'override')[0];
  assert.ok(override);
  assert.equal(calls(override, 'client.workOverride')[0].arguments[0].getText(), 'overrideRequest(ask, answerId, await fence())', 'snapshot fence only, no subject revisions');
  assert.deepEqual(calls(override, 'runAction')[0].arguments.slice(2).map(argument => argument.getText()), ['overrideRetryable', 'overrideRefusal'], 'refusals reach the existing action feedback');
  assert.equal(calls(override, 'readAutoResolved').length, 1);
});

test('Now lists the section below what waits and offers the override on matching closed rows', () => {
  const now = source('./screens/Now.tsx');
  assert.equal(nodes(now, node => ts.isJsxSelfClosingElement(node) && node.tagName.getText() === 'AutoResolvedSection').length, 1);
  assert.equal(calls(now, 'useFocusEffect').length, 1, 're-read when Now comes into view');
  assert.ok(calls(now, 'closedOverride').length >= 2, 'context menu and closed detail both offer it');
  const section = source('./screens/AutoResolved.tsx');
  assert.equal(calls(section, 'overrideCopy')[0].arguments[0].getText(), 'row.asker');
});
