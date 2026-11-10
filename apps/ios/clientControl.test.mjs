import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { ClientControlController } from './clientControl.ts';
const fixture = JSON.parse(readFileSync(new URL('../../fixtures/clients/client-control-v1.golden.json', import.meta.url)));
const binding = fixture.vectors[1].input;
const grant = () => ({ grant_id: binding.grant_id, run_id: binding.run_id, device: binding.device, session: binding.session, actor_agent: 'agent/fixture', state: 'active', verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'], max_commands: 50, expires_at: new Date(Date.now() + 60_000).toISOString() });
const command = () => ({ kind: 'device-command', id: binding.command_id, state: 'delivered', command_kind: 'phone.navigate', args: { screen: 'Attention', params: { id: binding.visible_state.opened_item.id } }, device_id: binding.device, session_actor: binding.session, issued_by: 'agent/fixture', grant_id: binding.grant_id, run_id: binding.run_id, expires_at: new Date(Date.now() + 30_000).toISOString(), control_grant: grant() });
const harness = (t, changes = {}) => {
  const effects = [], results = [];
  const controller = new ClientControlController({ identity: { device: binding.device, session: binding.session }, foreground: () => true,
    visible: () => binding.visible_state, navigate: async args => { effects.push(args); }, result: async (value, current) => { assert.equal(current(), true); results.push(value); }, revoke: async () => { effects.push('revoke'); },
    answer: () => assert.fail('answer callback'), close: () => assert.fail('close callback'), read: () => assert.fail('read callback'), ...changes });
  t.after(controller.dispose);
  return { controller, effects, results };
};

for (const [name, change] of [
  ['no grant', row => { delete row.control_grant; }], ['wrong run', row => { row.run_id += '/wrong'; }], ['wrong device', row => { row.device_id += '/wrong'; }], ['wrong session', row => { row.session_actor += '/wrong'; }],
  ['wrong issuer', row => { row.issued_by += '/wrong'; }], ['non-granted screen', row => { row.control_grant.screens = ['NowRoot']; }], ['non-granted verb', row => { row.control_grant.verbs = ['phone.inspect']; }],
  ['expired grant', row => { row.control_grant.expires_at = new Date(0).toISOString(); }], ['expired command', row => { row.expires_at = new Date(0).toISOString(); }], ['revoked grant', row => { row.control_grant.state = 'revoked'; }],
  ['Conversation', row => { row.args = { screen: 'Conversation', params: { target: 'agent/fixture' } }; row.control_grant.screens.push('Conversation'); }],
  ['wrong item_id', row => { row.control_grant.item_id = 'attention/other'; }],
  ...['phone.message.read', 'answer', 'close', 'work.done', 'message.read', 'prompt.respond'].map(verb => [verb, row => { row.command_kind = verb; row.control_grant.verbs.push(verb); }]),
]) test(`${name}: no navigation, action or signed result`, async t => {
  const { controller, effects, results } = harness(t), row = command(); change(row);
  await controller.handle(row); assert.deepEqual(effects, []); assert.deepEqual(results, []);
});

test('inspect and navigate use only pure ports and committed state; optional item_id and replay', async t => {
  const { controller, effects, results } = harness(t), originalFetch = globalThis.fetch;
  globalThis.fetch = () => assert.fail('no answer/read/close API');
  try {
    const row = command(); row.control_grant.item_id = row.args.params.id;
    await controller.handle(row); await controller.handle(row);
    const inspect = { ...command(), id: row.id + '/inspect', command_kind: 'phone.inspect', args: {} };
    await controller.handle(inspect);
    assert.deepEqual(effects, [row.args]); assert.equal(results.length, 2);
    assert.deepEqual(results[0].visible_state, binding.visible_state);
    assert.equal(controller.getGrant(), undefined, 'idle indicator requires generated lifecycle authority');
  } finally { globalThis.fetch = originalFetch; }
});
test('background command reports not-foreground without navigation', async t => {
  const { controller, effects, results } = harness(t, { foreground: () => false });
  await controller.handle(command()); assert.deepEqual(effects, []); assert.equal(results[0].outcome, 'not-foreground');
});
test('revoke ends local authority and invokes enrollment revoke; in-flight work cannot result', async t => {
  const pending = Promise.withResolvers(), entered = Promise.withResolvers();
  const { controller, effects, results } = harness(t, { navigate: async () => { entered.resolve(); await pending.promise; } });
  const applying = controller.handle(command()); await entered.promise;
  await controller.revoke(); pending.resolve(); await applying;
  await controller.handle({ ...command(), id: 'device-command/after-revoke' });
  assert.deepEqual(effects, ['revoke']); assert.deepEqual(results, []);
});
test('disconnect invalidates in-flight result', async t => {
  const pending = Promise.withResolvers(), entered = Promise.withResolvers();
  const { controller, results } = harness(t, { navigate: async () => { entered.resolve(); await pending.promise; } });
  const applying = controller.handle(command()); await entered.promise; controller.suspend(); pending.resolve(); await applying;
  assert.deepEqual(results, []);
});
test.todo('#6274: idle started/ended/revoked grant lifecycle resource is not in the generated collection union yet');
