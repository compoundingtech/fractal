import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { ClientControlController } from './clientControl.ts';
const fixture = JSON.parse(readFileSync(new URL('../../fixtures/clients/client-control-v1.golden.json', import.meta.url)));
const binding = fixture.vectors[1].input;
const lifecycleFixture = JSON.parse(readFileSync(new URL('../../fixtures/clients/client-control-lifecycle-v1.json', import.meta.url)));
const lifecycle = (changes = {}) => ({ ...lifecycleFixture.resources[0], device: binding.device, session: binding.session, grant_id: binding.grant_id, run_id: binding.run_id, item_id: undefined, ...changes });
const grant = () => ({ grant_id: binding.grant_id, run_id: binding.run_id, device: binding.device, session: binding.session, actor_agent: 'agent/fixture', state: 'active', verbs: ['phone.inspect', 'phone.navigate'], screens: ['NowRoot', 'Attention'], max_commands: 50, expires_at: new Date(Date.now() + 60_000).toISOString() });
const command = () => ({ kind: 'device-command', id: binding.command_id, state: 'delivered', command_kind: 'phone.navigate', args: { screen: 'Attention', params: { id: binding.visible_state.opened_item.id } }, device_id: binding.device, session_actor: binding.session, issued_by: 'agent/fixture', grant_id: binding.grant_id, run_id: binding.run_id, expires_at: new Date(Date.now() + 30_000).toISOString(), control_grant: grant() });
const harness = (t, changes = {}, started = lifecycle()) => {
  const effects = [], results = [];
  const controller = new ClientControlController({ identity: { device: binding.device, session: binding.session }, foreground: () => true,
    visible: () => binding.visible_state, navigate: async args => { effects.push(args); }, result: async (value, current) => { assert.equal(current(), true); results.push(value); }, revoke: async () => { effects.push('revoke'); },
    answer: () => assert.fail('answer callback'), close: () => assert.fail('close callback'), read: () => assert.fail('read callback'), ...changes });
  t.after(controller.dispose);
  if (started) void controller.handle(started);
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
  const started = lifecycle({ item_id: binding.visible_state.opened_item.id });
  const { controller, effects, results } = harness(t, {}, started), originalFetch = globalThis.fetch;
  globalThis.fetch = () => assert.fail('no answer/read/close API');
  try {
    const row = command(); row.control_grant.item_id = row.args.params.id;
    await controller.handle(row); await controller.handle(row);
    const inspect = { ...command(), id: row.id + '/inspect', command_kind: 'phone.inspect', args: {} };
    inspect.control_grant.item_id = row.control_grant.item_id;
    await controller.handle(inspect);
    assert.deepEqual(effects, [row.args]); assert.equal(results.length, 2);
    assert.deepEqual(results[0].visible_state, binding.visible_state);
    assert.equal(controller.getGrant(), started, 'only lifecycle publishes the run indicator');
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
test('commands never infer an active lifecycle', async t => {
  const { controller, effects, results } = harness(t, {}, false);
  await controller.handle(command());
  assert.equal(controller.getGrant(), undefined); assert.deepEqual(effects, []); assert.deepEqual(results, []);
});

test('idle lifecycle enforces own identity, ordering, item binding and terminal finality', async t => {
  const { controller } = harness(t, {}, false);
  const started = lifecycle({ item_id: 'attention/bound', sequence: 2 });
  for (const change of [{ device: 'device/other' }, { session: 'session/other' }]) {
    await controller.handle({ ...started, ...change }); assert.equal(controller.getGrant(), undefined);
  }
  void controller.handle(started); assert.equal(controller.getGrant(), started);
  for (const change of [{ sequence: 1 }, { sequence: 2 }, { item_id: undefined, sequence: 3 }, { item_id: 'other', sequence: 3 }, { run_id: 'other', sequence: 3 }]) {
    await controller.handle({ ...started, ...change }); assert.equal(controller.getGrant(), started);
  }
  void controller.handle({ ...started, state: 'ended', reason: 'claim-ended', sequence: 3 });
  assert.equal(controller.getGrant(), undefined);
  await controller.handle({ ...started, sequence: 4 }); assert.equal(controller.getGrant(), undefined);
});

test('golden sequence zero and unsafe sequences fail closed', async t => {
  const invalid = lifecycleFixture.traces[0].steps.find(step => step.expected === 'refuse-invalid-frame').resource;
  assert.equal(invalid.sequence, 0);
  const golden = harness(t, { identity: { device: invalid.device, session: invalid.session } }, lifecycleFixture.resources[0]);
  void golden.controller.handle(invalid);
  assert.equal(golden.controller.getGrant(), undefined, 'shared invalid resource immediately clears active authority');
  for (const sequence of [invalid.sequence, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    const { controller, effects, results } = harness(t);
    void controller.handle(lifecycle({ sequence })); assert.equal(controller.getGrant(), undefined);
    await controller.handle(command()); assert.deepEqual(effects, []); assert.deepEqual(results, []);
  }
  const { controller } = harness(t);
  await controller.handle(lifecycle({ sequence: Number.MAX_SAFE_INTEGER }));
  assert.equal(controller.getGrant().sequence, Number.MAX_SAFE_INTEGER);
});

for (const [state, reason] of [['ended', 'run-ended'], ['ended', 'claim-ended'], ['revoked', 'revoked']]) test(`${reason} clears idle and in-flight authority synchronously`, async t => {
  const idle = harness(t);
  void idle.controller.handle(lifecycle({ state, reason, sequence: 2 }));
  assert.equal(idle.controller.getGrant(), undefined);
  const pending = Promise.withResolvers(), entered = Promise.withResolvers();
  const { controller, results } = harness(t, { navigate: async () => { entered.resolve(); await pending.promise; } });
  const applying = controller.handle(command()); await entered.promise;
  const queued = controller.handle({ ...command(), id: 'queued' });
  void controller.handle(lifecycle({ state, reason, sequence: 2 }));
  assert.equal(controller.getGrant(), undefined);
  pending.resolve(); await applying; await queued; assert.deepEqual(results, []);
});

test('lifecycle must match every command grant field, including optional item', async t => {
  for (const change of [{ grant_id: 'other' }, { run_id: 'other' }, { device: 'other' }, { session: 'other' }, { item_id: 'other' }]) {
    const { controller, effects, results } = harness(t, {}, lifecycle(change));
    await controller.handle(command()); assert.deepEqual(effects, []); assert.deepEqual(results, []);
  }
});

test('command budget is preserved across lifecycle updates and duplicate delivery', async t => {
  const { controller, effects, results } = harness(t);
  const row = command(); row.control_grant.max_commands = 1;
  await controller.handle(row); await controller.handle(row);
  await controller.handle(lifecycle({ sequence: 2 }));
  await controller.handle({ ...row, id: 'over-budget' });
  assert.equal(effects.length, 1); assert.equal(results.length, 1);
});
