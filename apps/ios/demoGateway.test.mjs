import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { test } from 'node:test';
import WebSocket from 'ws';

test('demo gateway keeps an idle collections stream alive with protocol pings', { timeout: 35000 }, async t => {
  const gateway = spawn(process.execPath, [new URL('./demoGateway.mjs', import.meta.url).pathname, '0'], { stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(async () => {
    if (gateway.exitCode !== null || gateway.signalCode !== null) return;
    const stopped = once(gateway, 'exit');
    gateway.kill();
    await stopped;
  });
  let output = '';
  let port;
  for await (const chunk of gateway.stdout) {
    output += chunk;
    const listening = /demo gateway on :(\d+)/.exec(output);
    if (listening) { port = Number(listening[1]); break; }
  }
  assert.ok(port, output);
  const socket = new WebSocket(`ws://127.0.0.1:${port}/v1/client/collections`);
  t.after(() => socket.terminate());
  let pings = 0;
  socket.on('ping', () => { pings++; });
  await once(socket, 'open');
  // The app's receive deadline is 24 seconds. Keep the connection quiet beyond
  // that deadline: snapshots alone cannot prove the gateway's idle contract.
  await delay(26000);
  assert.ok(pings >= 3, `expected eight-second protocol pings, received ${pings}`);
  assert.equal(socket.readyState, WebSocket.OPEN);
});
