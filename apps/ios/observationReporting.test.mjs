import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
import { MINUTE_MS, Observations, toObservationReport } from './observations.ts';

// Execute the production callback without importing React Native's StoreProvider into Node.
const source = ts.createSourceFile('store.tsx', readFileSync(new URL('./store.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const callbacks = [];
const visit = node => {
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && node.left.getText(source) === 'state.send' && ts.isArrowFunction(node.right)) callbacks.push(node.right);
  ts.forEachChild(node, visit);
};
visit(source);
assert.equal(callbacks.length, 1);
const callback = ts.transpile(`return (${callbacks[0].getText(source)});`, { target: ts.ScriptTarget.ES2022 });
class ClientError extends Error { constructor(status) { super(`status ${status}`); this.status = status; } }
const bindSend = new Function('state', 'observations', 'toObservationReport', 'client', 'ClientError', 'setError', 'errorText', callback);
const setup = (storage = { read: async () => null, write: async () => {} }) => {
  const clock = { at: Date.UTC(2026, 9, 14, 12, 0, 5), now() { return this.at; }, id: () => '11111111-1111-4111-8111-111111111111' };
  const observations = new Observations(storage, clock, () => ({ carrier: 'tailscale' }));
  const state = { busy: false, last: 0, send: undefined };
  const sent = [], errors = [];
  const pending = Promise.withResolvers(), started = Promise.withResolvers();
  const client = { observationsReport: report => { sent.push(report); started.resolve(); return pending.promise; } };
  state.send = bindSend(state, observations, toObservationReport, client, ClientError, error => errors.push(error), error => error.message);
  observations.ready = state.send;
  return { clock, observations, state, sent, pending, started, errors };
};

test('report preparation reserves the send slot before a minute sweep reenters ready', async () => {
  const { clock, observations, state, sent, pending, started } = setup();
  observations.observe('ios-connect', 100);
  clock.at += MINUTE_MS;
  const sending = state.send();
  assert.equal(state.busy, true);
  await started.promise;
  assert.equal(sent.length, 1);
  assert.equal(sent[0].samples.length, 1);
  await state.send();
  assert.equal(sent.length, 1, 'one report is in flight');
  pending.resolve({});
  await sending;
  assert.equal(state.busy, false);
  assert.equal(observations.waiting, 0);
});

test('a null report releases the preparation slot without throttling the next measurement', async () => {
  const { state, sent } = setup();
  await state.send();
  assert.equal(state.busy, false);
  assert.equal(state.last, 0);
  assert.equal(sent.length, 0);
});

test('a preparation failure releases busy, surfaces the cause, and does not send', async () => {
  const { observations, state, sent, errors } = setup();
  observations.nextReport = async () => { throw new Error('summary storage unavailable'); };
  await state.send();
  assert.equal(state.busy, false);
  assert.equal(state.last, 0);
  assert.equal(sent.length, 0);
  assert.deepEqual(errors, ['summary storage unavailable']);
});

test('transport rejection releases busy and preserves the identical durable batch', async () => {
  const { clock, observations, state, sent, pending, started } = setup();
  observations.observe('ios-connect', 100);
  clock.at += MINUTE_MS;
  const sending = state.send();
  await started.promise;
  pending.reject(new Error('transport unavailable'));
  await sending;
  assert.equal(state.busy, false);
  assert.equal(observations.waiting, 1);
  state.last = 0;
  await state.send();
  assert.equal(sent.length, 2);
  assert.deepEqual(sent[1], sent[0]);
});

test('a content refusal is durably removed, other transport failures keep the batch', async () => {
  for (const [status, kept] of [[400, 0], [413, 0], [422, 0], [404, 1], [429, 1], [503, 1]]) {
    const { clock, observations, state, pending, started } = setup();
    observations.observe('ios-connect', 100);
    clock.at += MINUTE_MS;
    const sending = state.send();
    await started.promise;
    pending.reject(new ClientError(status));
    await sending;
    assert.equal(state.busy, false, `${status}`);
    assert.equal(observations.waiting, kept, `${status}: a ${kept ? 'retryable' : 'refused'} report`);
  }
});

test('upload waits for durable storage and failed acknowledgement storage retains the batch', async () => {
  const saved = Promise.withResolvers();
  let hold = true;
  const { clock, observations, state, sent, pending, started, errors } = setup({
    read: async () => null,
    write: async () => { if (hold) await saved.promise; else throw new Error('ack storage unavailable'); },
  });
  observations.observe('ios-connect', 100);
  clock.at += MINUTE_MS;
  const sending = state.send();
  assert.equal(sent.length, 0, 'no upload before storage completes');
  saved.resolve();
  await started.promise;
  hold = false;
  pending.resolve({});
  await sending;
  assert.equal(state.busy, false);
  assert.equal(observations.waiting, 1, 'accepted server report remains replayable until durable removal');
  assert.deepEqual(errors, ['ack storage unavailable']);
});
