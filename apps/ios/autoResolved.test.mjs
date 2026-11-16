import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { AUTO_RESOLVED_TITLE, OVERRIDE_TITLE, WORDS_REQUIRED, autoResolvedRows, autoResolvedVisible, closedOverride, needsWords, overrideAnswer, overrideChoices, overrideCopy, overrideKey, overrideRefusal, overrideRequest, overrideRetryable, singleOverride } from './autoResolved.ts';

const source = path => ts.createSourceFile(path, readFileSync(new URL(path, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const nodes = (root, predicate) => {
  const result = [];
  const visit = node => { if (predicate(node)) result.push(node); ts.forEachChild(node, visit); };
  visit(root);
  return result;
};
const calls = (root, expression) => nodes(root, node => ts.isCallExpression(node) && node.expression.getText() === expression);
const ancestor = (node, predicate) => {
  for (let parent = node.parent; parent; parent = parent.parent) if (predicate(parent)) return parent;
};

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
  const sent = overrideRequest(ask(), { id: 'us' }, fence);
  assert.deepEqual(sent, {
    id: `action/person-override:${episode}:us`,
    idempotency_key: `person-override:${episode}:us`,
    fence: { snapshot_id: 'snapshot/example', subject_revisions: {} },
    parameters: { target_id: 'step-run/example/region', episode, answer: { id: 'us' } },
  });
  assert.equal(overrideKey(episode, 'us'), `person-override:${episode}:us`);
  const again = overrideRequest(ask(), { id: 'us' }, { snapshot_id: 'snapshot/later', subject_revisions: {} });
  assert.equal(again.id, sent.id);
  assert.equal(again.idempotency_key, sent.idempotency_key);
  assert.throws(() => overrideRequest(without('action_parameters'), { id: 'us' }, fence));
});

const changes = { ...request, type: 'decision', answers: [{ id: 'approve', label: 'Approve', outcome: 'accept' }, { id: 'decline', label: 'Decline', outcome: 'decline' }, { id: 'revise', label: 'Request changes', outcome: 'request_changes' }] };

test('an answer requesting changes carries the person’s trimmed words; other answers send only their ID', () => {
  const decision = ask({ request: changes, default_answer: { id: 'approve', label: 'Approve' } });
  const [decline, revise] = overrideChoices(decision);
  assert.deepEqual([decline.id, revise.id], ['decline', 'revise']);
  assert.equal(needsWords(decline), false);
  assert.equal(needsWords(revise), true);
  assert.deepEqual(overrideAnswer(decline), { id: 'decline' });
  assert.deepEqual(overrideAnswer(decline, 'ignored words'), { id: 'decline' }, 'words go only with request_changes');
  assert.deepEqual(overrideAnswer(revise, '  Use the smaller instance.  '), { id: 'revise', text: 'Use the smaller instance.' });
  assert.equal(overrideAnswer(revise, '   '), WORDS_REQUIRED, 'empty words are refused, never sent');
  assert.equal(overrideAnswer(revise), WORDS_REQUIRED);
  const fence = { snapshot_id: 'snapshot/example', subject_revisions: {} };
  assert.deepEqual(overrideRequest(decision, overrideAnswer(revise, ' Use the smaller instance. '), fence).parameters,
    { target_id: 'step-run/example/region', episode, answer: { id: 'revise', text: 'Use the smaller instance.' } });
  assert.deepEqual(overrideRequest(decision, overrideAnswer(decline), fence).parameters.answer, { id: 'decline' });
  assert.equal(overrideRequest(decision, overrideAnswer(revise, 'x'), fence).idempotency_key, `person-override:${episode}:revise`);
});

test('the section shows for rows, a failed read, or an incomplete list even when it is empty', () => {
  const rows = autoResolvedRows([ask()], name, now);
  assert.equal(autoResolvedVisible([], { issue: '', complete: true }), false);
  assert.equal(autoResolvedVisible(rows, { issue: '', complete: true }), true);
  assert.equal(autoResolvedVisible([], { issue: 'Could not read auto-resolved asks: offline', complete: true }), true);
  assert.equal(autoResolvedVisible([], { issue: '', complete: false }), true, 'an empty capped read still warns that coverage is incomplete');
  const section = source('./screens/AutoResolved.tsx');
  const guard = nodes(section, ts.isIfStatement).find(node => node.expression.getText().includes('autoResolvedVisible'));
  assert.equal(guard?.expression.getText(), '!autoResolvedVisible(rows, autoResolved)');
  assert.ok(nodes(section, node => ts.isIdentifier(node) && node.text === 'INCOMPLETE_AUTO_RESOLVED').length >= 2, 'the warning is rendered');
});

test('one override per ask is in flight: a second is refused until the first settles, accepted or refused', async () => {
  const pending = new Set();
  const seen = [];
  let finish;
  const first = singleOverride(pending, episode, () => new Promise(resolve => { finish = resolve; }), () => seen.push(pending.has(episode)));
  assert.equal(pending.has(episode), true);
  let sentAgain = false;
  assert.equal(await singleOverride(pending, episode, async () => { sentAgain = true; return true; }), false);
  assert.equal(sentAgain, false, 'a double tap never sends a second override');
  assert.equal(await singleOverride(pending, 'e'.repeat(64), async () => true), true, 'another ask is not held up');
  finish(false);
  assert.equal(await first, false);
  assert.equal(pending.has(episode), false, 'a refusal clears it');
  assert.deepEqual(seen, [true, false], 'the rows redraw when it starts and when it settles');
  await assert.rejects(singleOverride(pending, episode, async () => { throw new Error('offline'); }));
  assert.equal(pending.has(episode), false, 'a thrown send clears it too');
  assert.equal(await singleOverride(pending, episode, async () => true), true);
});

test('the store and every entry point respect the pending override', () => {
  const store = source('./store.tsx');
  const override = nodes(store, node => ts.isMethodDeclaration(node) && node.name.getText() === 'override')[0];
  const single = calls(override, 'singleOverride')[0];
  assert.ok(single);
  assert.deepEqual(single.arguments.slice(0, 2).map(argument => argument.getText()), ['overriding.current', 'ask.episode']);
  assert.ok(ancestor(calls(override, 'client.workOverride')[0], node => node === single), 'the send runs inside the pending guard');
  assert.ok(ancestor(calls(override, 'readAutoResolved')[0], node => node === single), 'pending clears after the re-read, so a sent row never re-enables');
  const section = source('./screens/AutoResolved.tsx');
  assert.ok(nodes(section, ts.isIfStatement).some(node => node.expression.getText() === 'busy || overridePending.has(row.ask.episode)'));
  assert.ok(calls(section, 'Alert.prompt').length === 1 && calls(section, 'needsWords').length === 1, 'request_changes asks for words first');
  const detail = source('./screens/Now.tsx');
  assert.ok(detail.getText().includes("disabled={busy || status !== 'online' || overridePending.has(overridable.ask.episode)}"));
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

test('the store reads auto-resolved asks without a timer and re-reads after an override', () => {
  const store = source('./store.tsx');
  const read = nodes(store, node => ts.isVariableDeclaration(node) && node.name.getText() === 'readAutoResolved')[0];
  assert.ok(read);
  assert.equal(calls(read, 'client.autoResolvedList').length, 1);
  assert.ok(nodes(read, ts.isIfStatement).some(node => node.expression.getText().includes("caps?.session_actor.startsWith('person/')")), 'only a person session reads its own asks');
  assert.equal(nodes(store, node => ts.isIdentifier(node) && node.text === 'setInterval').length, 0, 'no polling');
  const override = nodes(store, node => ts.isMethodDeclaration(node) && node.name.getText() === 'override')[0];
  assert.ok(override);
  assert.equal(calls(override, 'client.workOverride')[0].arguments[0].getText(), 'overrideRequest(ask, answer, await fence())', 'snapshot fence only, no subject revisions');
  assert.deepEqual(calls(override, 'runAction')[0].arguments.slice(2).map(argument => argument.getText()), ['overrideRetryable', 'overrideRefusal'], 'refusals reach the existing action feedback');
  assert.equal(calls(override, 'readAutoResolved').length, 1);
});

test('Now lists the section below what waits and offers the override on matching closed rows', () => {
  const now = source('./screens/Now.tsx');
  assert.equal(nodes(now, node => ts.isJsxSelfClosingElement(node) && node.tagName.getText() === 'AutoResolvedSection').length, 1);
  const home = nodes(now, node => ts.isFunctionDeclaration(node) && node.name?.text === 'NowScreen')[0];
  assert.ok(home);
  assert.equal(calls(home, 'useFocusEffect').length, 1, 're-read when Now comes into view');
  assert.equal(nodes(home, node => ts.isJsxSelfClosingElement(node) && node.tagName.getText() === 'PendingInboxThreads').length, 1, 'thread recovery remains alongside auto-resolved asks');
  assert.ok(calls(now, 'closedOverride').length >= 2, 'context menu and closed detail both offer it');
  const section = source('./screens/AutoResolved.tsx');
  assert.equal(calls(section, 'overrideCopy')[0].arguments[0].getText(), 'row.asker');
});
