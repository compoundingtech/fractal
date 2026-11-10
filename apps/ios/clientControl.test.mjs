import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientControlController } from './clientControl.ts';
import { authorizeClientControl, revokeClientControl } from './clientControlWire.ts';

const command = (command_kind, args) => ({
  kind: 'device-command',
  id: `device-command/${command_kind}`,
  revision: 'revision/one',
  updated_at: '2026-11-10T12:00:00Z',
  device_id: 'device/phone',
  command_kind,
  args,
  state: 'delivered',
  issued_at: '2026-11-10T12:00:00Z',
  expires_at: '2026-11-10T12:00:30Z',
  issued_by: 'person/operator',
  session_actor: 'agent/operator',
});
const commands = [
  command('phone.inspect', {}),
  command('phone.navigate', { screen: 'NowRoot' }),
  command('phone.navigate', { screen: 'Conversation', params: { target: 'agent/one', sessionId: 'session/one' } }),
  command('phone.message.read', { message_id: 'message/one' }),
];

for (const foreground of [true, false]) {
  test(`absent grant source denies every existing command (foreground=${foreground}) without effects or results`, async () => {
    const effects = [];
    const controller = new ClientControlController({
      navigate: args => effects.push(['navigate', args]),
      visible: () => { effects.push(['visible']); return { screen: 'NowRoot' }; },
      foreground: () => { effects.push(['foreground']); return foreground; },
    });
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (...args) => {
      effects.push(['result/network', args]);
      assert.fail('closed control must not post wire results');
    };
    try {
      for (const row of commands) assert.equal(await controller.handle(row), undefined);
      assert.equal(controller.getGrant(), undefined);
      assert.deepEqual(effects, [], 'denial must not navigate, inspect, read foreground, or send results');
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
}

test('wire adapter never treats a UI projection as grant authority', async () => {
  const context = {
    grant: { runId: 'run/untrusted-ui-projection', revokeState: 'available' },
    foreground: () => assert.fail('closed adapter must not evaluate app ports'),
  };
  for (const row of commands) assert.equal(authorizeClientControl(row, context), false);
  assert.equal(await revokeClientControl(context.grant), false);
  assert.equal(await revokeClientControl(undefined), false);
});

test('grant store remains absent through revoke and subscription disposal', async () => {
  const controller = new ClientControlController({
    navigate: () => assert.fail('unexpected navigation'),
    visible: () => assert.fail('unexpected inspection'),
    foreground: () => assert.fail('unexpected foreground read'),
  });
  let updates = 0;
  const unsubscribe = controller.subscribe(() => updates++);
  assert.equal(controller.getGrant(), undefined);
  await controller.revoke();
  assert.equal(controller.getGrant(), undefined, 'failed wire revoke must not manufacture a grant or enrollment transition');
  assert.equal(updates, 0);
  unsubscribe();
  await controller.revoke();
  assert.equal(updates, 0);
});

for (const behavior of [
  'authoritative grant lifecycle: start, expiry, removal, reconnect and device binding',
  'wrong-run, non-granted screen/verb, stale, replayed and revoked command rejection',
  'canonical signed command/result vectors and signature verification',
  'revoke acknowledgement and failure without claiming enrollment ended',
  'not-foreground signed wire results without navigation',
  'inspect/navigate signed wire results from the committed visible route',
]) {
  test.todo(`#6274 codegen required: ${behavior}`);
}
