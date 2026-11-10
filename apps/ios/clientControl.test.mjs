import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientControlController, applyGrantedClientControl } from './clientControl.ts';
import { authorizeClientControl, revokeClientControl } from './clientControlWire.ts';

const command = (command_kind, args = {}) => ({ kind: 'device-command', id: 'device-command/fixture', command_kind, args, state: 'delivered' });
const scope = { verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'] };
const requests = [command('phone.inspect'), command('phone.navigate', { screen: 'NowRoot' }), command('phone.navigate', { screen: 'Attention', params: { id: 'attention/fixture' } })];
const refused = [command('phone.navigate', { screen: 'Conversation', params: { target: 'agent/fixture' } }), ...['phone.message.read', 'work.done', 'prompt.respond', 'answer', 'close', 'message.read'].map(verb => command(verb))];
const portsFor = effects => ({ foreground: () => true, visible: () => { effects.push('inspect'); return { screen: 'NowRoot' }; }, navigate: args => effects.push(args), answer: () => assert.fail('answer'), close: () => assert.fail('close'), read: () => assert.fail('read') });

test('local grant permits only pure inspect/NowRoot/Attention; no answer, close, read or result API', () => {
  const effects = [], ports = portsFor(effects), originalFetch = globalThis.fetch;
  globalThis.fetch = () => assert.fail('no answer/read/close/signed-result API call');
  try {
    for (const row of refused) applyGrantedClientControl(row, scope, ports);
    assert.deepEqual(effects, [], 'Conversation and every mutating verb are refused without results');
    for (const row of requests) applyGrantedClientControl(row, scope, ports);
    assert.deepEqual(effects, ['inspect', requests[1].args, requests[2].args]);
    effects.length = 0;
    applyGrantedClientControl(requests[2], { ...scope, screens: ['NowRoot'] }, ports);
    applyGrantedClientControl(requests[0], { ...scope, verbs: [] }, ports);
    applyGrantedClientControl(requests[2], scope, { ...ports, foreground: () => false });
    assert.deepEqual(effects, []);
  } finally { globalThis.fetch = originalFetch; }
});

for (const foreground of [true, false]) test(`absent authoritative source denies all commands (foreground=${foreground})`, async () => {
  const effects = [], controller = new ClientControlController({ ...portsFor(effects), foreground: () => foreground });
  for (const row of [...requests, ...refused]) await controller.handle(row);
  assert.deepEqual(effects, []);
  assert.equal(controller.getGrant(), undefined);
  const stop = controller.subscribe(() => assert.fail('no grant source'));
  await controller.revoke(); stop();
  assert.equal(await revokeClientControl(undefined), false);
  assert.equal(authorizeClientControl(requests[0], { grant: { runId: 'run/ui-only', revokeState: 'available' }, foreground: () => foreground }), false);
});

for (const behavior of ['grant start/end/expiry/revoke and device/session binding', 'wrong run and generated grant-scoped screen/verb refusals', 'canonical signed vectors and replay rejection', 'enrollment revoke acknowledgement', 'not-foreground signed result', 'committed visible state and local-policy wire-binding variants']) {
  test.todo(`#6274 codegen required: ${behavior}`);
}
