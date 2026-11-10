import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';
import { ClientControlController } from './clientControl.ts';

const lifecycleFixture = JSON.parse(readFileSync(new URL('../../fixtures/clients/client-control-lifecycle-v1.json', import.meta.url)));
const settle = () => new Promise(resolve => setTimeout(resolve, 20));
for (const [testMode, granted] of [[undefined, true], [false, false], [false, true], [true, false], [true, true]]) {
  test(`commands subscription test mode=${testMode} capability=${granted}`, async () => {
    const { Feed } = await import('./feed.ts');
    const { client, sockets } = fakeClient();
    const foreground = new ForegroundGate('active');
    const feed = new Feed(client, { onWindow() {}, onConnection() {} }, foreground, () => 'action/test');
    const received = []; let resets = 0;
    const stop = feed.followCommands(testMode, granted, command => received.push(command), () => { resets++; });
    await settle();
    const subscribed = () => sockets.at(-1).sent.some(frame => frame.collection === 'commands');
    assert.equal(subscribed(), testMode === true && granted);
    const command = { kind: 'device-command', id: 'device-command/test', command_kind: 'phone.inspect', args: {}, state: 'delivered' };
    const lifecycle = lifecycleFixture.resources[0];
    const frame = { kind: 'snapshot', id: 'commands', collection: 'commands', items: [command, lifecycle] };
    sockets[0].frame(frame);
    assert.deepEqual(received, testMode === true && granted ? [lifecycle, command] : []);
    for (const resource of lifecycleFixture.resources.slice(1)) {
      sockets[0].frame({ ...frame, kind: 'changes', items: undefined, upserts: [resource], removes: [] });
    }
    assert.deepEqual(received, testMode === true && granted ? [lifecycle, command, ...lifecycleFixture.resources.slice(1)] : []);
    foreground.update('background');
    assert.equal(sockets[0].closed, true);
    assert.equal(resets > 0, testMode === true && granted, 'background invalidates grant authority');
    foreground.update('active');
    await settle();
    assert.equal(subscribed(), testMode === true && granted, 'same policy applies on reconnect');
    stop();
    const count = received.length;
    sockets.at(-1).frame(frame);
    assert.equal(received.length, count);
    feed.close();
  });
}

test('commands feed carries golden invalid sequence to the active controller', async t => {
  const { Feed } = await import('./feed.ts');
  const { client, sockets } = fakeClient();
  const started = lifecycleFixture.resources[0];
  const invalid = lifecycleFixture.traces[0].steps.find(step => step.expected === 'refuse-invalid-frame').resource;
  const controller = new ClientControlController({ identity: { device: started.device, session: started.session },
    foreground: () => true, visible: () => undefined,
    navigate: async () => assert.fail('no navigation'), result: async () => assert.fail('no result'), revoke: async () => {} });
  const foreground = new ForegroundGate('active');
  const feed = new Feed(client, { onWindow() {}, onConnection() {} }, foreground, () => 'action/test');
  t.after(() => { feed.close(); controller.dispose(); });
  feed.followCommands(true, true, controller.handle, controller.suspend);
  await settle();
  sockets[0].frame(lifecycleFixture.frames[0]);
  assert.deepEqual(controller.getGrant(), started);
  foreground.update('background'); assert.equal(controller.getGrant(), undefined);
  foreground.update('active'); await settle();
  sockets.at(-1).frame(lifecycleFixture.frames[0]);
  assert.deepEqual(controller.getGrant(), started, 'unchanged started snapshot restores the run indicator');
  sockets.at(-1).frame({ ...lifecycleFixture.frames[1], upserts: [invalid] });
  assert.equal(controller.getGrant(), undefined, 'sequence zero fails closed through the shipped feed path');
  for (const identity of ['device', 'session', 'absent']) await t.test(`replacement snapshot refuses ${identity} lifecycle`, () => {
    sockets.at(-1).frame(lifecycleFixture.frames[0]);
    assert.deepEqual(controller.getGrant(), started);
    const items = identity === 'absent' ? [] : [{ ...started, [identity]: 'foreign' }];
    sockets.at(-1).frame({ ...lifecycleFixture.frames[0], items });
    assert.equal(controller.getGrant(), undefined, 'foreign or absent lifecycle never restores authority');
  });
});
