import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash, createPublicKey, generateKeyPairSync, sign, verify } from 'node:crypto';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { test } from 'node:test';
import WebSocket from 'ws';
import * as Schema from 'effect/Schema';
import { actionReceiptCheck, ClientError, FractalClient, notApplied } from '../../clients/typescript/fractal-client/index.ts';
import { CollectionFrame, Envelope, ErrorEnvelope, ThreadSummary, ThreadSummaryPage, ThreadTimelinePage, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import { ANCHOR_MS } from '../../clients/typescript/fractal-scenarios/src/index.ts';
import { Feed } from './feed.ts';
import { ForegroundGate } from './foreground.ts';
import { verifyPairing } from './pairingProof.ts';
import { signedBytes, signatureParameter, threadMessageSubject } from '../../clients/typescript/fractal-views/deviceSigning.ts';

import { ThreadEdits } from './threadEdits.ts';

const controlThreads = async (baseUrl, command) => {
  const response = await fetch(`${baseUrl}/__fixture/threads`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(command),
  });
  assert.equal(response.status, 200);
  return response.json();
};

const nextThreadFrame = (socket, accepts) => {
  const next = Promise.withResolvers();
  const received = data => {
    const frame = decodeUnknownSync(CollectionFrame, 'strict')(JSON.parse(String(data)));
    if (!accepts(frame)) return;
    socket.off('message', received);
    next.resolve(frame);
  };
  socket.on('message', received);
  return next.promise;
};
const startGateway = async (t, { capture = false, scenario } = {}) => {
  const gateway = spawn(process.execPath, ['--experimental-transform-types', new URL('./demoGateway.mjs', import.meta.url).pathname, '0', ...(scenario ? [scenario] : [])], {
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

test('clock-skew fixture supplies canonical native thread REST and live windows without changing authoritative Open', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'clock-skew' });
  const decodedFrames = [];
  const decodeErrors = [];
  const client = new FractalClient({
    baseUrl, credential: async () => 'demo-credential',
    fetchImpl: async (...args) => {
      const response = await fetch(...args);
      const path = new URL(args[0]).pathname;
      const valueSchema = path === '/v1/client/threads' ? ThreadSummaryPage
        : path.startsWith('/v1/client/threads/') ? path.endsWith('/timeline') ? ThreadTimelinePage : ThreadSummary : undefined;
      const envelopeSchema = valueSchema ? Schema.Struct({ ...Envelope.fields, value: valueSchema }) : Envelope;
      decodeUnknownSync(envelopeSchema, 'strict')(await response.clone().json());
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
  const capabilities = (await client.capabilities()).value.capabilities;
  assert.ok(capabilities.some(item => item.id === 'threads-v1' && item.version === 2 && item.state === 'granted'));
  assert.ok(capabilities.some(item => item.id === 'thread-summary-feed' && item.state === 'granted'));
  assert.ok(!capabilities.some(item => item.id.startsWith('thread.') && item.state === 'granted'), 'fixture grants no thread mutations');
  const context = { agent_id: 'agent/example/atlas/builder' };
  const summaries = (await client.threadList(context)).value.items;
  assert.deepEqual(summaries.map(item => item.id), ['thread/worker-tests', 'thread/rollout-notes', 'thread/cancellation']);
  const summary = summaries[0];
  assert.equal(summary.id, 'thread/worker-tests');
  assert.equal(summary.state, 'open');
  assert.equal(summary.revision, 'claim/skew-reopen');
  assert.equal(summary.title_revision, 'claim/title');
  assert.deepEqual((await client.threadGet(summary.id)).value, summary);
  assert.deepEqual((await client.threadList({ agent_id: 'agent/other' })).value.items, []);
  const closed = (await client.threadList({ recently_closed: true })).value.items;
  assert.equal(closed.length, 1);
  assert.equal(closed[0].id, 'thread/rollout-notes');
  assert.equal(closed[0].state, 'closed');
  assert.equal((await client.threadGet(closed[0].id)).value.revision, 'claim/rollout-closed');
  assert.deepEqual((await client.threadTimeline(closed[0].id)).value.items.map(item => item.kind), ['thread.closed', 'message']);
  assert.equal((await client.threadTimeline('thread/cancellation')).value.items[0].id, 'message/cancellation');
  const attention = (await client.attentionList()).value.items.find(item => item.title === 'Review the loadUser rename');
  assert.equal(attention.episode, 'claim/fixture-review-episode');
  assert.equal(attention.conversation_id, attention.requester_id);
  const origin = { origin_source: attention.source_id, origin_episode: attention.episode };
  const inbox = (await client.threadList(origin)).value.items;
  assert.equal(inbox.length, 1);
  assert.equal(inbox[0].id, 'thread/review-rename');
  assert.equal(inbox[0].agent_id, attention.requester_id, 'Inbox origin retains its real kit requester');
  assert.equal((await client.threadGet(inbox[0].id)).value.revision, 'claim/inbox-current');
  assert.equal((await client.threadTimeline(inbox[0].id)).value.items[0].id, 'message/inbox-review');
  assert.deepEqual((await client.threadList({ ...origin, origin_episode: 'claim/other-episode' })).value.items, []);
  const events = (await client.threadTimeline(summary.id)).value.items;
  assert.deepEqual(events.map(event => event.id), ['event/skew-close', 'event/skew-reopen', 'message/review-ready']);
  assert.equal(events[0].accepted_at_unix_ms, Date.parse('2026-11-09T12:59:00Z'));
  assert.equal(events[1].accepted_at_unix_ms, Date.parse('2026-11-09T12:58:00Z'));
  const windows = Array.from({ length: 5 }, () => Promise.withResolvers());
  const issues = [];
  const feed = new Feed(client, {
    onWindow: () => {}, onConnection: (_state, issue) => { if (issue) issues.push(issue); },
    onWindowError: (_name, issue) => issues.push(issue),
  }, new ForegroundGate('active'), () => 'action/demo-thread-test');
  t.after(() => feed.close());
  const follow = [
    feed.followThreadSummaries(context, { onWindow: window => windows[0].resolve(window), onIssue: issue => issues.push(issue) }),
    feed.followThreadSummary(summary.id, { onWindow: window => windows[1].resolve(window), onIssue: issue => issues.push(issue) }),
    feed.followThreadTimeline(summary.id, { onWindow: window => windows[2].resolve(window), onIssue: issue => issues.push(issue) }),
    feed.followThreadSummaries({ recently_closed: true }, { onWindow: window => windows[3].resolve(window), onIssue: issue => issues.push(issue) }),
    feed.followThreadSummaries(origin, { onWindow: window => windows[4].resolve(window), onIssue: issue => issues.push(issue) }),
  ];
  t.after(() => follow.forEach(held => held.close()));
  const [list, detail, timeline, closedLive, inboxLive] = await Promise.all(windows.map(window => window.promise));
  assert.deepEqual(list.items, summaries);
  assert.deepEqual(closedLive.items, closed);
  assert.deepEqual(inboxLive.items, inbox);
  assert.deepEqual(detail.items, [summary]);
  assert.deepEqual(timeline.items, events);
  assert.deepEqual(decodeErrors, []);
  assert.deepEqual(issues, []);
  assert.ok(decodedFrames.some(frame => frame.collection === 'thread-timeline'));
  assert.ok((await client.agentsList()).value.items.some(agent => agent.id === context.agent_id), 'base replay remains available');
  assert.deepEqual((await client.attentionList()).value.items.find(item => item.id === attention.id), attention, 'fixture reads never answer or mark the Inbox request');
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

test('mutable thread fixture acknowledges close, closed reply, reopen and rename, and definitively rejects one thread', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const caps = await client.capabilities();
  for (const action of ['thread.reply', 'thread.close', 'thread.reopen', 'thread.rename']) {
    assert.ok(caps.value.capabilities.some(capability => capability.id === action && capability.state === 'granted'));
  }
  const before = await client.threadGet('thread/action-preview');
  const closeRequest = {
    id: 'action/fixture-close', idempotency_key: 'fixture-thread-close-0001',
    fence: { snapshot_id: before.snapshot.id, subject_revisions: { [before.value.id]: before.value.revision } },
    parameters: { target_id: before.value.id },
  };
  const closed = await actionReceiptCheck({ kind: 'metadata', actionId: closeRequest.id, targetId: before.value.id })(() => client.threadClose(closeRequest));
  assert.equal(closed.value.status, 'completed');
  assert.equal(closed.value.snapshot_id, closed.snapshot.id);
  assert.equal(closed.snapshot.host_id, before.snapshot.host_id);
  assert.ok(closed.snapshot.store_index > before.snapshot.store_index);
  assert.equal((await client.threadGet(before.value.id)).value.state, 'closed');
  assert.deepEqual(await client.threadClose(closeRequest), closed, 'repeating the original request returns the same receipt');
  assert.equal((await client.threadTimeline(before.value.id)).value.items.filter(event => event.kind === 'thread.closed').length, 1);

  const replyRequest = {
    id: 'action/fixture-reply', idempotency_key: 'fixture-thread-reply-0001',
    fence: { snapshot_id: closed.snapshot.id, subject_revisions: {} },
    parameters: { target_id: before.value.id, content: 'A closed thread accepts this quoted reply.', thread_quote_id: 'message/action-preview' },
  };
  const replied = await actionReceiptCheck({ kind: 'message', actionId: replyRequest.id, targetId: before.value.id })(() => client.threadReply(replyRequest));
  const reply = (await client.threadTimeline(before.value.id)).value.items[0];
  assert.ok(replied.value.affected_ids.includes(reply.id));
  assert.equal(reply.thread_quote_id, 'message/action-preview');
  assert.equal((await client.threadGet(before.value.id)).value.state, 'closed', 'reply does not reopen');

  const afterReply = await client.threadGet(before.value.id);
  const reopenRequest = { ...closeRequest, id: 'action/fixture-reopen', idempotency_key: 'fixture-thread-reopen-0001', fence: { snapshot_id: replied.snapshot.id, subject_revisions: { [before.value.id]: afterReply.value.revision } } };
  const reopened = await actionReceiptCheck({ kind: 'metadata', actionId: reopenRequest.id, targetId: before.value.id })(() => client.threadReopen(reopenRequest));
  assert.equal((await client.threadGet(before.value.id)).value.state, 'open');
  const current = (await client.threadGet(before.value.id)).value;
  const renameRequest = {
    id: 'action/fixture-rename', idempotency_key: 'fixture-thread-rename-0001', fence: { snapshot_id: reopened.snapshot.id, subject_revisions: { [current.id]: current.revision } },
    parameters: { target_id: current.id, title: 'Renamed writable fixture', title_revision: current.title_revision },
  };
  const renamed = await actionReceiptCheck({ kind: 'metadata', actionId: renameRequest.id, targetId: current.id })(() => client.threadRename(renameRequest));
  assert.equal((await client.threadGet(current.id)).value.title, 'Renamed writable fixture');
  assert.ok(renamed.snapshot.store_index > reopened.snapshot.store_index);
  await assert.rejects(client.threadRename({
    ...renameRequest, id: 'action/fixture-stale-rename', idempotency_key: 'fixture-stale-rename-0001',
    parameters: { ...renameRequest.parameters, title: 'Stale rename must not apply' },
  }), error => {
    assert.ok(error instanceof ClientError);
    assert.equal(error.status, 409);
    assert.equal(error.response.code, 'stale-fence');
    assert.equal(error.response.retryable, false);
    assert.ok(notApplied(error), 'the SDK classifies a stale-fence code as a definitive refusal');
    assert.equal(error.response.applied, undefined);
    assert.deepEqual(error.response.details, {});
    decodeUnknownSync(ErrorEnvelope, 'strict')(error.response);
    return true;
  });
  assert.equal((await client.threadGet(current.id)).value.title, 'Renamed writable fixture');

  const rejected = (await client.threadGet('thread/rejected-preview')).value;
  const edits = new ThreadEdits(() => {}, error => error.message);
  const refusal = edits.run(rejected, { kind: 'close' }, () => client.threadClose({
    ...closeRequest, id: 'action/fixture-rejected', idempotency_key: 'fixture-thread-rejected-0001',
    fence: { snapshot_id: before.snapshot.id, subject_revisions: { [rejected.id]: rejected.revision } }, parameters: { target_id: rejected.id },
  }).then(() => ({ how: 'sent' })).catch(error => {
    assert.ok(error instanceof ClientError && error.status === 403);
    assert.equal(error.response.applied, undefined);
    assert.deepEqual(error.response.details, {});
    throw error;
  }));
  assert.equal(edits.project(rejected).state, 'closed', 'the fixture can exercise immediate optimistic presentation');
  assert.equal(edits.flash(rejected.id), undefined);
  const failed = await refusal;
  assert.equal(failed.how, 'failed');
  assert.match(failed.error, /intentionally rejects/);
  assert.equal(edits.project(rejected), rejected, 'definitive fixture refusal rolls back');
  assert.equal(edits.flash(rejected.id), undefined);
  assert.deepEqual((await client.threadGet(rejected.id)).value, rejected);
});

test('fixture state controls expose empty, retryable error, offline, revoked grant and quoted reply in order', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const context = { agent_id: 'agent/example/atlas/builder' };
  const controls = await (await fetch(`${baseUrl}/__fixture/threads`)).json();
  assert.deepEqual(controls.states.slice(0, 5), ['empty', 'error', 'offline', 'revoked', 'quoted']);
  const invalidControl = await fetch(`${baseUrl}/__fixture/threads`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ state: 'unknown' }),
  });
  assert.equal(invalidControl.status, 400);
  const invalidError = decodeUnknownSync(ErrorEnvelope, 'strict')(await invalidControl.json());
  assert.equal(invalidError.code, 'validation-failed');
  assert.equal(invalidError.retryable, false);
  assert.equal(invalidError.applied, undefined);
  assert.deepEqual(invalidError.details, {});
  assert.equal((await (await fetch(`${baseUrl}/__fixture/threads`)).json()).state, controls.state);
  let socket = new WebSocket(`${baseUrl.replace('http:', 'ws:')}/v1/client/collections`);
  t.after(() => socket.terminate());
  await once(socket, 'open');
  const snapshots = [];
  socket.on('message', data => { const frame = JSON.parse(String(data)); if (frame.kind === 'snapshot') snapshots.push(frame); });
  const initial = nextThreadFrame(socket, frame => frame.id === 'fixture-state');
  socket.send(JSON.stringify({ kind: 'subscribe', id: 'fixture-state', collection: 'threads', thread_context: context, limit: 25 }));
  await initial;

  const emptyLive = nextThreadFrame(socket, frame => frame.id === 'fixture-state');
  await controlThreads(baseUrl, { state: 'empty' });
  assert.deepEqual((await client.threadList(context)).value.items, []);
  assert.deepEqual((await emptyLive).items, []);
  const errorLive = nextThreadFrame(socket, frame => frame.id === 'fixture-state' && frame.kind === 'error');
  await controlThreads(baseUrl, { state: 'error' });
  const readError = await errorLive;
  assert.equal(readError.retryable, true);
  await assert.rejects(client.threadList(context), error => error instanceof ClientError && error.status === 503 && error.response.retryable);
  const beforeRecovery = snapshots.length;
  await controlThreads(baseUrl, { state: 'ready', broadcast: false });
  assert.ok((await client.threadList(context)).value.items.length, 'the same read succeeds when retried after recovery');
  await delay(10);
  assert.equal(snapshots.length, beforeRecovery, 'silent recovery leaves the inline error available for an explicit Retry');

  const disconnected = once(socket, 'close');
  await controlThreads(baseUrl, { state: 'offline' });
  await disconnected;
  await assert.rejects(client.threadList(context), 'offline fixture destroys product HTTP connections rather than returning a fake success');
  await controlThreads(baseUrl, { state: 'revoked' });
  await assert.rejects(client.threadList(context), error => error instanceof ClientError && error.status === 403 && /grant was revoked/.test(error.response.message));
  assert.ok((await client.capabilities()).value.capabilities.filter(capability => capability.id.startsWith('thread.')).every(capability => capability.state === 'ungranted'));
  socket = new WebSocket(`${baseUrl.replace('http:', 'ws:')}/v1/client/collections`);
  await once(socket, 'open');
  const refusedLive = nextThreadFrame(socket, frame => frame.id === 'fixture-state' && frame.kind === 'error');
  socket.send(JSON.stringify({ kind: 'subscribe', id: 'fixture-state', collection: 'threads', thread_context: context, limit: 25 }));
  assert.equal((await refusedLive).code, 'forbidden');

  const quotedLive = nextThreadFrame(socket, frame => frame.id === 'fixture-state' && frame.kind === 'snapshot');
  await controlThreads(baseUrl, { state: 'quoted' });
  assert.deepEqual((await quotedLive).items.map(item => item.id), ['thread/quoted-preview']);
  const quoted = (await client.threadTimeline('thread/quoted-preview')).value.items;
  assert.equal(quoted.length, 2);
  assert.equal(quoted[0].thread_quote_id, quoted[1].id);
  assert.match(quoted[1].content, /original quoted context/);
});

test('older-page fixture summaries update over REST and the real iOS exact-summary live feed without entering the head', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const frames = [], decodeErrors = [];
  const client = new FractalClient({
    baseUrl, credential: async () => 'demo-credential',
    socket: (url, protocols, headers) => {
      const socket = new WebSocket(url, protocols, { headers });
      socket.on('message', data => {
        try { frames.push(decodeUnknownSync(CollectionFrame, 'strict')(JSON.parse(String(data)))); }
        catch (error) { decodeErrors.push(error); }
      });
      return socket;
    },
  });
  const context = { agent_id: 'agent/example/atlas/builder', limit: 25 };
  const head = (await client.threadList(context)).value;
  assert.equal(head.items.length, 25);
  assert.ok(head.next_cursor);
  const older = (await client.threadList({ ...context, cursor: head.next_cursor })).value;
  assert.ok(older.items.some(item => item.id === 'thread/older-page'));
  assert.equal(older.next_cursor, null);
  const initialHead = Promise.withResolvers(), initialOlder = Promise.withResolvers(), updated = Promise.withResolvers();
  const windows = [], olderWindows = [], issues = [];
  const feed = new Feed(client, {
    onWindow() {}, onConnection: (_state, issue) => { if (issue) issues.push(issue); },
    onWindowError: (_name, issue) => issues.push(issue),
  }, new ForegroundGate('active'), () => 'action/fixture-pages');
  t.after(() => feed.close());
  feed.followThreadSummaries({ agent_id: context.agent_id }, {
    onWindow: window => { windows.push(window); initialHead.resolve(window); }, onIssue: issue => issues.push(issue),
  });
  feed.followThreadSummary('thread/older-page', {
    onWindow: window => {
      olderWindows.push(window);
      if (window.items[0]?.title === 'Older-page summary — live update') updated.resolve(window);
      else initialOlder.resolve(window);
    }, onIssue: issue => issues.push(issue),
  });
  const [headLive, olderLive] = await Promise.all([initialHead.promise, initialOlder.promise]);
  assert.equal(headLive.items.length, 25);
  assert.equal(headLive.hasMore, true);
  assert.equal(olderLive.items[0].title, 'Older-page summary — initial');
  await controlThreads(baseUrl, { update: 'older-rest' });
  assert.equal((await client.threadGet('thread/older-page')).value.title, 'Older-page summary — REST update');
  assert.equal((await client.threadList({ ...context, cursor: head.next_cursor })).value.items.at(-1).title, 'Older-page summary — REST update');
  assert.equal(olderWindows.length, 1, 'REST-only control deliberately leaves live state untouched until a live update');
  await controlThreads(baseUrl, { update: 'older-live' });
  const changed = await updated.promise;
  assert.ok(changed.snapshot.store_index > olderLive.snapshot.store_index);
  assert.equal(changed.items[0].title, 'Older-page summary — live update');
  assert.ok(windows.every(window => !window.items.some(item => item.id === 'thread/older-page')), 'older detail never leaks into the head membership');
  assert.ok(frames.some(frame => frame.kind === 'changes' && frame.upserts.some(item => item.id === 'thread/older-page')));
  assert.deepEqual(decodeErrors, []);
  assert.deepEqual(issues, []);
});

test('thread-actions retains clock-skew, closed and Inbox fixtures unchanged', { timeout: 15000 }, async t => {
  const originalUrl = await startGateway(t, { capture: true, scenario: 'clock-skew' });
  const mutableUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const original = new FractalClient({ baseUrl: originalUrl, credential: async () => 'demo-credential' });
  const mutable = new FractalClient({ baseUrl: mutableUrl, credential: async () => 'demo-credential' });
  for (const id of ['thread/worker-tests', 'thread/rollout-notes', 'thread/cancellation', 'thread/review-rename']) {
    assert.deepEqual((await mutable.threadGet(id)).value, (await original.threadGet(id)).value);
    assert.deepEqual((await mutable.threadTimeline(id)).value, (await original.threadTimeline(id)).value);
  }
  const context = { agent_id: 'agent/example/atlas/builder' };
  const oldRows = (await original.threadList(context)).value.items;
  const newRows = (await mutable.threadList(context)).value.items;
  assert.deepEqual(newRows.slice(0, oldRows.length), oldRows);
  assert.ok(newRows.some(item => item.id === 'thread/action-preview'));
  assert.deepEqual((await mutable.threadList({ recently_closed: true })).value, (await original.threadList({ recently_closed: true })).value);
  const attention = (await original.attentionList()).value.items.find(item => item.title === 'Review the loadUser rename');
  const origin = { origin_source: attention.source_id, origin_episode: attention.episode };
  assert.deepEqual((await mutable.threadList(origin)).value, (await original.threadList(origin)).value);
});

test('fixture reset restores mutations, receipts, scenarios and live windows with an advancing snapshot', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const initial = await client.threadGet('thread/action-preview');
  const initialTimeline = (await client.threadTimeline(initial.value.id)).value;
  const initialOlder = (await client.threadGet('thread/older-page')).value;
  const close = {
    id: 'action/reset-close', idempotency_key: 'fixture-reset-close-0001',
    fence: { snapshot_id: initial.snapshot.id, subject_revisions: { [initial.value.id]: initial.value.revision } }, parameters: { target_id: initial.value.id },
  };
  await client.threadClose(close);
  const afterClose = await client.threadGet(initial.value.id);
  await client.threadRename({
    id: 'action/reset-rename', idempotency_key: 'fixture-reset-rename-0001',
    fence: { snapshot_id: afterClose.snapshot.id, subject_revisions: { [initial.value.id]: afterClose.value.revision } },
    parameters: { target_id: initial.value.id, title_revision: initial.value.title_revision, title: 'Will be reset' },
  });
  await client.threadReply({
    id: 'action/reset-reply', idempotency_key: 'fixture-reset-reply-0001',
    fence: { snapshot_id: initial.snapshot.id, subject_revisions: {} },
    parameters: { target_id: initial.value.id, content: 'Will be removed by reset' },
  });
  await controlThreads(baseUrl, { update: 'older-live' });
  const socket = new WebSocket(`${baseUrl.replace('http:', 'ws:')}/v1/client/collections`);
  t.after(() => socket.terminate());
  await once(socket, 'open');
  const changedWindow = nextThreadFrame(socket, frame => frame.id === 'reset-live');
  socket.send(JSON.stringify({ kind: 'subscribe', id: 'reset-live', collection: 'threads', subject: initial.value.id }));
  const changed = await changedWindow;
  assert.equal(changed.items[0].title, 'Will be reset');
  assert.equal(changed.items[0].state, 'closed');
  const resetWindow = nextThreadFrame(socket, frame => frame.id === 'reset-live');
  const reset = await controlThreads(baseUrl, { reset: true });
  assert.equal(reset.state, 'ready');
  const restored = await resetWindow;
  assert.deepEqual(restored.items, [decodeUnknownSync(ThreadSummary)(initial.value)]);
  assert.ok(restored.snapshot.store_index > changed.snapshot.store_index);
  assert.deepEqual((await client.threadTimeline(initial.value.id)).value, initialTimeline);
  assert.deepEqual((await client.threadGet('thread/older-page')).value, initialOlder);
  await client.threadClose(close);
  assert.equal((await client.threadGet(initial.value.id)).value.state, 'closed', 'reset clears the receipt cache so reused fixture action IDs apply again');
  for (const state of ['empty', 'error', 'offline', 'revoked', 'quoted']) {
    await controlThreads(baseUrl, { state });
    const control = await controlThreads(baseUrl, { reset: true });
    assert.equal(control.state, 'ready');
    assert.deepEqual((await client.threadGet(initial.value.id)).value, initial.value);
    assert.ok((await client.threadList({ agent_id: initial.value.agent_id })).value.items.length > 1);
    assert.ok((await client.capabilities()).value.capabilities.some(item => item.id === 'thread.reply' && item.state === 'granted'));
  }
});

test('mutable demo pairing binds the phone signing key and sends a verifiably signed quoted reply', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const phone = generateKeyPairSync('ed25519');
  const key = phone.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64url');
  const session = (await client.completePairing('demo', { code: 'demo', device_public_key: key, key_storage: 'software' })).value;
  const crypto = {
    hash: async bytes => new Uint8Array(createHash('sha256').update(bytes).digest()),
    verify: async (publicKey, text, signature) => verify(null, Buffer.from(text), createPublicKey({
      key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(publicKey, 'base64url')]),
      format: 'der', type: 'spki',
    }), Buffer.from(signature, 'base64url')),
  };
  assert.ok(session.scopes.includes('control.messages'));
  await verifyPairing(session, key, undefined, crypto);
  await assert.rejects(verifyPairing(session, 'wrong-phone-key', undefined, crypto), /does not bind/);
  const corrupted = structuredClone(session);
  corrupted.device_key_proofs[0].signature.signature = 'invalid';
  await assert.rejects(verifyPairing(corrupted, key, undefined, crypto), /signature does not verify/);
  const thread = await client.threadGet('thread/quoted-preview');
  const id = 'action/signed-fixture-reply';
  const fields = {
    content: 'A signed fixture reply', from: session.person_id, to: thread.value.agent_id,
    thread_id: thread.value.id, thread_quote_id: 'message/quoted-original',
  };
  const message = {
    subject: await threadMessageSubject(fields, id, crypto.hash), signer: session.person_id, fields,
    key, chain: session.device_key_chain, nonce: Buffer.alloc(16, 1).toString('base64url'), signedAt: Date.now(),
  };
  const bytes = signedBytes(message);
  const signature = signatureParameter(message, sign(null, Buffer.from(bytes), phone.privateKey).toString('base64url'));
  assert.ok(await crypto.verify(key, bytes, signature.signature), 'the enrolled phone key verifies the canonical reply signature');
  const receipt = await client.threadReply({
    id, idempotency_key: 'fixture-signed-reply-0001', fence: { snapshot_id: thread.snapshot.id, subject_revisions: {} },
    parameters: { target_id: thread.value.id, content: fields.content, thread_quote_id: fields.thread_quote_id, signature },
  });
  assert.ok(receipt.value.affected_ids.includes(message.subject));
  const reply = (await client.threadTimeline(thread.value.id)).value.items[0];
  assert.equal(reply.id, message.subject);
  assert.equal(reply.thread_quote_id, fields.thread_quote_id);
});

test('mutable thread metadata enforces snapshot, subject and lifecycle fences', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const initial = await client.threadGet('thread/action-preview');
  let sequence = 0;
  const request = (fence, parameters = { target_id: initial.value.id }) => ({
    id: `action/fence-${++sequence}`, idempotency_key: `fixture-fence-${sequence}-0001`, fence, parameters,
  });
  const fence = { snapshot_id: initial.snapshot.id, subject_revisions: { [initial.value.id]: initial.value.revision } };
  const refuses = (status, code) => error => {
    assert.ok(error instanceof ClientError);
    assert.equal(error.status, status);
    assert.equal(error.response.code, code);
    assert.equal(error.response.applied, undefined);
    assert.deepEqual(error.response.details, {});
    return true;
  };
  for (const subject_revisions of [{}, { ...fence.subject_revisions, 'thread/other': 'claim/other' }]) {
    await assert.rejects(client.threadClose(request({ ...fence, subject_revisions })), refuses(422, 'validation-failed'));
  }
  for (const snapshot_id of ['snapshot/invalid', `snapshot/another-host/${initial.snapshot.store_index}/fixture`,
    `snapshot/${initial.snapshot.host_id}/${initial.snapshot.store_index + 999}/fixture`]) {
    await assert.rejects(client.threadClose(request({ ...fence, snapshot_id })), refuses(409, 'stale-fence'));
  }
  await assert.rejects(client.threadReopen(request(fence)), refuses(409, 'stale-fence'));
  await client.threadClose(request(fence));
  await assert.rejects(client.threadReopen(request(fence)), refuses(409, 'stale-fence'));
  const closed = await client.threadGet(initial.value.id);
  const currentFence = { ...fence, subject_revisions: { [initial.value.id]: closed.value.revision } };
  await assert.rejects(client.threadClose(request(currentFence)), refuses(409, 'stale-fence'));
  await client.threadReopen(request(currentFence));
  assert.equal((await client.threadGet(initial.value.id)).value.state, 'open', 'an older same-host snapshot remains valid with the current subject revision');
  await assert.rejects(client.threadReply(request(fence, { target_id: initial.value.id, content: 'Invalid subject fence' })), refuses(422, 'validation-failed'));
  for (const body of ['{', JSON.stringify({ type: 'thread.close' })]) {
    const result = await fetch(`${baseUrl}/v1/client/actions`, { method: 'POST', headers: { 'content-type': 'application/json' }, body });
    assert.equal(result.status, 422);
    assert.equal(decodeUnknownSync(ErrorEnvelope, 'strict')(await result.json()).code, 'validation-failed');
  }
});

test('mutable timeline pages are limited, scoped and pinned through their head snapshot', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const initial = await client.threadGet('thread/action-preview');
  const reply = index => client.threadReply({
    id: `action/page-${index}`, idempotency_key: `fixture-page-${index}-0001`,
    fence: { snapshot_id: initial.snapshot.id, subject_revisions: {} },
    parameters: { target_id: initial.value.id, content: `Timeline pagination message ${index}` },
  });
  for (let index = 0; index < 55; index++) await reply(index);
  const read = async (id, query = '') => {
    const result = await fetch(`${baseUrl}/v1/client/threads/${encodeURIComponent(id.slice(7))}/timeline${query}`);
    assert.equal(result.status, 200);
    const envelope = await result.json();
    decodeUnknownSync(Schema.Struct({ ...Envelope.fields, value: ThreadTimelinePage }), 'strict')(envelope);
    return envelope.value;
  };
  const first = await read(initial.value.id);
  assert.equal(first.items.length, 40);
  assert.ok(first.next_cursor);
  assert.equal((await read(initial.value.id, '?limit=200')).items.length, 50);
  assert.equal((await read(initial.value.id, '?limit=25')).items.length, 25);
  await reply(56);
  const older = await read(initial.value.id, `?cursor=${encodeURIComponent(first.next_cursor)}`);
  assert.equal(older.items.length, 16);
  assert.equal(older.next_cursor, null);
  const ids = [...first.items, ...older.items].map(item => item.id);
  assert.equal(new Set(ids).size, 56);
  assert.ok(!older.items.some(item => item.content === 'Timeline pagination message 56'));
  for (const [id, cursor] of [['thread/quoted-preview', first.next_cursor],
    [initial.value.id, first.next_cursor.replace(/:\d+:/, ':999999:')],
    [initial.value.id, first.next_cursor.replace(/:[^:]+$/, ':message/missing')]]) {
    const result = await fetch(`${baseUrl}/v1/client/threads/${encodeURIComponent(id.slice(7))}/timeline?cursor=${encodeURIComponent(cursor)}`);
    assert.equal(result.status, 422);
    assert.equal(decodeUnknownSync(ErrorEnvelope, 'strict')(await result.json()).code, 'validation-failed');
  }
});

test('recently closed subscriptions remove a reopened thread and admit its replacement', { timeout: 15000 }, async t => {
  const baseUrl = await startGateway(t, { capture: true, scenario: 'thread-actions' });
  const client = new FractalClient({ baseUrl, credential: async () => 'demo-credential' });
  const initial = await client.threadGet('thread/action-preview');
  await client.threadClose({
    id: 'action/window-close', idempotency_key: 'fixture-window-close-0001',
    fence: { snapshot_id: initial.snapshot.id, subject_revisions: { [initial.value.id]: initial.value.revision } },
    parameters: { target_id: initial.value.id },
  });
  const socket = new WebSocket(`${baseUrl.replace('http:', 'ws:')}/v1/client/collections`);
  t.after(() => socket.terminate());
  await once(socket, 'open');
  const firstFrame = nextThreadFrame(socket, frame => frame.id === 'closed-window');
  socket.send(JSON.stringify({ kind: 'subscribe', id: 'closed-window', collection: 'threads', thread_context: { recently_closed: true }, limit: 1 }));
  const first = await firstFrame;
  assert.equal(first.items.length, 1);
  const selected = await client.threadGet(first.items[0].id);
  const changedFrame = nextThreadFrame(socket, frame => frame.id === 'closed-window');
  await client.threadReopen({
    id: 'action/window-reopen', idempotency_key: 'fixture-window-reopen-0001',
    fence: { snapshot_id: selected.snapshot.id, subject_revisions: { [selected.value.id]: selected.value.revision } },
    parameters: { target_id: selected.value.id },
  });
  const changed = await changedFrame;
  assert.equal(changed.kind, 'changes');
  assert.deepEqual(changed.removes, [selected.value.id]);
  assert.equal(changed.upserts.length, 1);
  assert.notEqual(changed.upserts[0].id, selected.value.id);
  assert.deepEqual(changed.order, changed.upserts.map(item => item.id));
  assert.equal(changed.has_more, false);
});
