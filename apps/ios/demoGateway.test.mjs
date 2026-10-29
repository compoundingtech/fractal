import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { test } from 'node:test';
import WebSocket from 'ws';
import { FractalClient } from '../../clients/typescript/fractal-client/index.ts';
import { CollectionFrame, Envelope, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import { ANCHOR_MS } from '../../clients/typescript/fractal-scenarios/src/index.ts';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';

const startGateway = async (t, { capture = false } = {}) => {
  const gateway = spawn(process.execPath, ['--experimental-transform-types', new URL('./demoGateway.mjs', import.meta.url).pathname, '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, EXPO_PUBLIC_FRACTAL_LANDING_CAPTURE: capture ? '1' : '0' },
  });
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

test('fleet-mid-refactor HTTP and collections decode for iOS Fleet, Now and conversation', { timeout: 15000 }, async t => {
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
  const sessions = await client.sessionsList();
  assert.equal(sessions.value.items.length, 4, 'Fleet receives the kit session inventory');
  assert.ok(sessions.value.items.some(item => item.kind === 'session' && item.id === builder.current_session_id && item.owner_id === builder.id));
  const devices = await client.devicesList();
  assert.equal(devices.value.items.length, 2, 'Fleet receives the kit device inventory');
  assert.ok(devices.value.items.every(item => item.kind === 'device' && item.person_id.startsWith('person/')));
  const launches = await client.launchesList();
  assert.equal(launches.value.items.length, 1, 'Fleet receives the kit launch inventory');
  const launch = launches.value.items[0];
  assert.equal(launch.kind, 'launch');
  const variants = await client.launchVariantsList(launch.id);
  assert.equal(variants.value.items.length, 2, 'Fleet receives both kit launch variants');
  assert.ok(variants.value.items.every(item => item.kind === 'launch-variant' && item.launch_id === launch.id));
  assert.deepEqual(variants.value.items.map(item => item.id).sort(), [...launch.variants].sort());
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
  assert.ok(thread.items.some(item => item.type === 'content'), 'real iOS Feed receives conversation content');
  assert.ok(decodedFrames.some(frame => frame.kind === 'conversation'));
  assert.deepEqual(decodeErrors, [], 'every received frame passes the strict client schema decoder');
  assert.deepEqual(issues, [], 'the iOS feed accepts the scenario without reconnects or refused windows');
});


test('capture mode freezes world timestamps and updates at the kit anchor', { timeout: 15000 }, async t => {
  const first = await startGateway(t, { capture: true });
  const read = async (base, route) => {
    const response = await fetch(`${base}/v1/client/${route}`, { headers: { authorization: 'Bearer demo-credential' } });
    assert.equal(response.status, 200);
    const wire = await response.json();
    decodeUnknownSync(Envelope, 'strict')(wire);
    return wire;
  };
  const initial = await read(first, 'agents');
  assert.equal(initial.snapshot.created_at, new Date(ANCHOR_MS).toISOString());
  await delay(100);
  const later = await read(first, 'agents');
  assert.deepEqual(later.value, initial.value);
  assert.deepEqual(later.snapshot, initial.snapshot);
  const second = await startGateway(t, { capture: true });
  const restarted = await read(second, 'agents');
  assert.deepEqual(restarted.value, initial.value);
  assert.deepEqual(restarted.snapshot, initial.snapshot);
  const session = initial.value.items.find(item => item.kind === 'agent' && item.current_session_id).current_session_id;
  const route = `sessions/${encodeURIComponent(session)}/timeline`;
  assert.deepEqual((await read(first, route)).value, (await read(second, route)).value);
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
