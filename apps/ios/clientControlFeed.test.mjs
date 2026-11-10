import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ForegroundGate } from './foreground.ts';
import { fakeClient } from './fakeFeed.mjs';

const settle = () => new Promise(resolve => setTimeout(resolve, 20));
for (const [testMode, granted] of [[undefined, true], [false, false], [false, true], [true, false], [true, true]]) {
  test(`commands subscription test mode=${testMode} capability=${granted}`, { todo: '#6274 codegen: base client index exports absent ClientControl.generated.ts' }, async () => {
    const { Feed } = await import('./feed.ts');
    const { client, sockets } = fakeClient();
    const foreground = new ForegroundGate('active');
    const feed = new Feed(client, { onWindow() {}, onConnection() {} }, foreground, () => 'action/test');
    const received = [];
    const stop = feed.followCommands(testMode, granted, command => received.push(command));
    await settle();
    const subscribed = () => sockets.at(-1).sent.some(frame => frame.collection === 'commands');
    assert.equal(subscribed(), testMode === true && granted);
    const command = { kind: 'device-command', id: 'device-command/test', command_kind: 'phone.inspect', args: {}, state: 'delivered' };
    const frame = { kind: 'snapshot', id: 'commands', collection: 'commands', items: [command] };
    sockets[0].frame(frame);
    assert.equal(received.length, testMode === true && granted ? 1 : 0);
    foreground.update('background');
    assert.equal(sockets[0].closed, true);
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
