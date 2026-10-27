import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { test } from 'node:test';
import WebSocket from 'ws';
import { FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { CollectionFrame, Envelope, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';

const startGateway = async t => {
  const gateway = spawn(process.execPath, ['--experimental-transform-types', new URL('./demoGateway.mjs', import.meta.url).pathname, '0'], { stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(async () => {
    if (gateway.exitCode !== null || gateway.signalCode !== null) return;
    const stopped = once(gateway, 'exit');
    gateway.kill();
    await stopped;
  });
  let errors = '';
  gateway.stderr.on('data', chunk => { errors += chunk; });
  let output = '';
  t.after(() => {
    t.diagnostic(`gateway stdout:\n${output}`);
    if (errors.trim()) t.diagnostic(`gateway stderr:\n${errors}`);
  });
  const started = Promise.withResolvers();
  // Keep draining stdout after startup: returning from its async iterator destroys
  // the pipe, making the next HTTP request's console.log crash the child with EPIPE.
  gateway.stdout.on('data', chunk => {
    output += chunk;
    const listening = /demo gateway on :(\d+)/.exec(output);
    if (listening) started.resolve(`http://127.0.0.1:${listening[1]}`);
  });
  gateway.once('error', started.reject);
  gateway.once('exit', () => started.reject(new Error(`gateway did not start: ${output}\n${errors}`)));
  return started.promise;
};

test('fleet-mid-refactor HTTP and collections decode for iOS Now and conversation', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t);
  const decodedFrames = [];
  const decodeErrors = [];
  const client = new FractalClient({
    baseUrl,
    credential: async () => 'demo-credential-0123456789abcdef0123456789abcdef',
    fetchImpl: async (...args) => {
      const response = await fetch(...args);
      decodeUnknownSync(Envelope, 'strict')(await response.clone().json());
      return response;
    },
    socket: (url, protocols, headers) => {
      const socket = new WebSocket(url, protocols, { headers });
      socket.on('message', data => {
        try { decodedFrames.push(decodeUnknownSync(CollectionFrame, 'strict')(JSON.parse(String(data)))); }
        catch (error) { decodeErrors.push(error); }
      });
      return socket;
    },
  });
  const preflight = await (await fetch(`${baseUrl}/v1/client/capabilities`)).json();
  assert.equal(preflight.capabilities[0].id, 'device-key-proofs');
  const paired = await client.completePairing('demo', { code: 'demo', device_key: 'demo-test-key', device_name: 'Demo simulator' });
  assert.equal(paired.value.person_root_key_proof.subject, paired.value.person_id);
  const capabilities = await client.capabilities();
  assert.equal(capabilities.value.kind, 'capabilities');
  const agents = await client.agentsList();
  const builder = agents.value.items.find(item => item.id === 'agent/example/atlas/builder');
  assert.ok(builder, 'the capture agent is supplied by the kit world');
  assert.equal(builder.name, 'Atlas Builder');
  const timeline = await client.timelineList(builder.current_session_id);
  assert.ok(timeline.value.items.length > 0, 'HTTP conversation history decodes');

  const windows = {};
  const issues = [];
  const ready = Promise.withResolvers();
  const conversation = Promise.withResolvers();
  const feed = new Feed(client, {
    onWindow: (name, items) => {
      windows[name] = items;
      if (['attention', 'missions', 'agents'].every(name => windows[name])) ready.resolve();
    },
    onConnection: (_state, issue) => { if (issue) issues.push(issue); },
    onWindowError: (name, issue) => issues.push(`${name}: ${issue}`),
  }, new ForegroundGate('active'), () => 'action/demo-test');
  t.after(() => feed.close());
  feed.followConversation(builder.id, {
    onEntries: frame => conversation.resolve(frame),
    onIssue: issue => { if (issue) issues.push(issue); },
  });
  await ready.promise;
  const thread = await conversation.promise;
  assert.ok(windows.attention.some(item => item.title === 'Review the loadUser rename'));
  assert.ok(windows.missions.some(item => item.title === 'Rename fetchUser to loadUser'));
  assert.ok(windows.agents.some(item => item.id === builder.id));
  assert.equal(thread.sessionId, builder.current_session_id);
  assert.ok(thread.items.some(item => item.kind === 'content'), 'real iOS Feed receives conversation content');
  assert.ok(decodedFrames.some(frame => frame.kind === 'conversation'));
  assert.deepEqual(decodeErrors, [], 'every received frame passes the strict client schema decoder');
  assert.deepEqual(issues, [], 'the iOS feed accepts the scenario without reconnects or refused windows');
});

test('demo gateway keeps an idle collections stream alive with protocol pings', { timeout: 35000 }, async t => {
  const baseUrl = await startGateway(t);
  const socket = new WebSocket(`${baseUrl.replace('http:', 'ws:')}/v1/client/collections`);
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
