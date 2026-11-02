import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { API_VERSION, FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { PhoneCommands, phoneCommandsGranted } from './phoneCommands.ts';
import { markMessageRead, readPhoneMessage } from './messageRead.ts';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';

const core = createRequire(new URL('./node_modules/@react-navigation/native/package.json', import.meta.url)).resolve('@react-navigation/core');
const { StackRouter } = await import(createRequire(core).resolve('@react-navigation/routers'));
const router = StackRouter({ initialRouteName: 'NowRoot' });
const config = { routeNames: ['NowRoot', 'Conversation', 'Documents'], routeParamList: {}, routeGetIdList: {} };
let state, ready = true;
const listeners = new Set();
globalThis.testPhoneNavigation = {
  isReady: () => ready, getCurrentRoute: () => state.routes[state.index],
  getRootState: () => ({ index: 0, routes: [{ name: 'Now' }] }),
  addListener: (_, listener) => { listeners.add(listener); return () => listeners.delete(listener); },
  dispatch: action => { assert.equal(action.payload.name, 'Now'); state = router.getStateForAction(state, { ...action, payload: { name: action.payload.params.screen, params: action.payload.params.params } }, config) ?? state; for (const listener of listeners) listener(); },
};
const { inspectPhone, navigatePhone } = await import('./phoneNavigation.ts');
const snapshot = { id: 'snapshot/test', host_id: 'host/test', store_index: 1, projection_version: 'client-projection.v0', created_at: '2020-01-01T00:00:00Z' };
const command = (id, kind = 'phone.inspect', args = {}) => ({ kind: 'device-command', id, revision: '1', command_kind: kind, args, state: 'delivered', expires_at: '2020-01-01T00:00:30Z' });
const frame = items => ({ kind: 'snapshot', id: 'commands', collection: 'commands', items, order: items.map(row => row.id), has_more: false, snapshot });
const flush = async () => { for (let n = 0; n < 10; n++) await Promise.resolve(); };
function setup() {
  state = router.getInitialState(config); ready = true;
  const gate = new ForegroundGate('active'), calls = [], acted = new Set();
  let online = true, deny = false, loseReport = false;
  const client = new FractalClient({ baseUrl: 'https://phone.test', fetchImpl: async (url, init) => {
    const body = init.body ? JSON.parse(init.body) : undefined; calls.push({ url, body });
    if (url.endsWith('/device-results') && loseReport) { loseReport = false; throw new TypeError('Network request failed'); }
    if (url.endsWith('/actions') && deny) return Response.json({ api_version: API_VERSION, error_version: 'st3.client.error.v0', code: 'scope-denied', message: 'Denied', retryable: false, details: {} }, { status: 403 });
    return Response.json({ api_version: API_VERSION, snapshot, value: url.endsWith('/capabilities') ? { capabilities: [], session_actor: 'client/phone' } : {} });
  } });
  const attention = [{ id: 'attention/message', revision: '7', source_id: 'message/source', actions: ['message.read'] }];
  const bindings = {
    foreground: () => gate.active, online: () => online,
    inspect: () => inspectPhone({ connection: { status: online ? 'online' : 'offline', route: 'HTTPS' }, foreground: gate.active, session_actor: 'client/phone', windows: { attention: 1, missions: 0, agents: 0 } }),
    navigate: navigatePhone,
    markRead: id => readPhoneMessage(id, attention, async item => { await markMessageRead(client, item, acted, () => 'action/read'); return true; }),
  };
  const executor = new PhoneCommands(client, () => bindings);
  return { gate, calls, acted, executor, deny: () => { deny = true; }, offline: () => { online = false; }, loseReport: () => { loseReport = true; }, results: () => calls.filter(call => call.url.endsWith('/device-results')).map(call => call.body) };
}

test('probe gates subscription, frames dispatch once, reconnect reprobes, background closes the socket', async () => {
  const h = setup(), fake = fakeClient(); let work;
  const feed = new Feed(fake.client, { onWindow() {}, onConnection() {}, onCommands: (rows, at, received) => { work = h.executor.receive(rows, at, received); } }, h.gate, () => 'action/test');
  try {
    await flush();
    assert.equal(phoneCommandsGranted({ capabilities: [] }), false);
    assert.equal(fake.sockets[0].sent.some(row => row.collection === 'commands'), false);
    feed.setCommandsGranted(phoneCommandsGranted({ capabilities: [{ id: 'phone-commands', version: 1, state: 'granted' }] }));
    assert.deepEqual(fake.sockets[0].sent.at(-1), { kind: 'subscribe', id: 'commands', collection: 'commands', limit: 8 });
    fake.sockets[0].frame(frame([command('inspect')])); await work;
    fake.sockets[0].frame(frame([command('inspect')])); await work;
    assert.equal(h.results().length, 1); assert.equal(h.results()[0].result.route, 'NowRoot');
    assert.equal(h.results()[0].result.tab, 'Now');
    h.gate.update('background'); assert.equal(fake.sockets[0].closed, true);
    h.gate.update('active'); await flush();
    assert.equal(fake.sockets[1].sent.some(row => row.collection === 'commands'), false);
    feed.setCommandsGranted(true); fake.sockets[1].frame(frame([command('inspect')])); await work;
    assert.equal(h.results().length, 1);
  } finally { feed.close(); h.executor.close(); }
});

test('typed navigation dispatches to the real stack router and reports the reached route', async () => {
  const h = setup();
  await h.executor.receive([command('nav', 'phone.navigate', { screen: 'Conversation', params: { target: 'agent/worker' } })], snapshot);
  assert.equal(state.routes.at(-1).params.target, 'agent/worker');
  assert.deepEqual(h.results()[0].result, { route: 'Conversation' }); assert.equal(listeners.size, 0);
  ready = false; await h.executor.receive([command('unready', 'phone.navigate', { screen: 'NowRoot' })], snapshot);
  assert.equal(h.results()[1].outcome, 'failed');
});

test('markRead uses real fresh fences, acted bookkeeping, generated actions and reports scope refusal', async () => {
  const h = setup();
  await h.executor.receive([command('read', 'phone.message.read', { attention_id: 'attention/message' })], snapshot);
  assert.equal(h.acted.has('attention/message'), true);
  assert.deepEqual(h.calls.find(call => call.url.endsWith('/actions')).body.fence, { snapshot_id: snapshot.id, subject_revisions: { 'attention/message': '7' } });
  assert.equal(h.calls.find(call => call.url.endsWith('/actions')).body.parameters.target_id, 'message/source');
  assert.equal(h.results()[0].outcome, 'completed');
  h.deny(); await h.executor.receive([command('denied', 'phone.message.read', { attention_id: 'attention/message' })], snapshot);
  assert.equal(h.results()[1].outcome, 'refused'); assert.equal(h.results()[1].error_code, 'scope-denied');
  await h.executor.receive([command('missing', 'phone.message.read', { attention_id: 'attention/missing' })], snapshot);
  assert.equal(h.results()[2].error_code, 'attention-unavailable');
});

test('daemon time drops expired and terminal rows despite phone clock skew; offline refuses without a handler', async () => {
  const h = setup();
  await h.executor.receive([{ ...command('expired'), expires_at: snapshot.created_at }, { ...command('terminal'), state: 'completed' }], snapshot);
  assert.equal(h.calls.length, 0);
  h.offline(); await h.executor.receive([command('offline', 'phone.message.read', { attention_id: 'attention/message' })], snapshot);
  assert.equal(h.results()[0].outcome, 'not-foreground'); assert.equal(h.acted.size, 0);
  h.gate.update('background'); await h.executor.receive([command('background')], snapshot); assert.equal(h.results().length, 1);
  h.executor.close(); await h.executor.receive([command('closed')], snapshot); assert.equal(h.results().length, 1);
});

test('a lost result retries the result POST, never the write', async () => {
  const h = setup(); h.loseReport();
  await h.executor.receive([command('read', 'phone.message.read', { attention_id: 'attention/message' })], snapshot);
  assert.equal(h.results().length, 2); assert.equal(h.calls.filter(call => call.url.endsWith('/actions')).length, 1);
  assert.deepEqual(h.results()[0], h.results()[1]);
});
