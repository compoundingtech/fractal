// Demo-only gateway for Debug simulator screenshots; no login or real authority.
// Never point a Release build or a real device at this unauthenticated server.
// node --experimental-transform-types demoGateway.mjs [port] [clock-skew|thread-actions]
// clock-skew stays read-only. thread-actions is mutable, disposable fixture authority only.
// In thread-actions, GET /__fixture/threads lists controls. POST {"state":"empty"} then
// error, offline, revoked, quoted (or ready); {"update":"older-rest"|"older-live"} changes the older page.
import { createHash, generateKeyPairSync, randomBytes, sign as ed25519Sign } from 'node:crypto';
import http from 'node:http';
import { WebSocket, WebSocketServer } from 'ws';
import * as Schema from 'effect/Schema';
import { ANCHOR_MS, loadWorld, threadClockSkewEvents } from '../../clients/typescript/fractal-scenarios/src/index.ts';
import { ActionRequest, ActionResult, Attention, CollectionCommand, CollectionFrame, Envelope, ErrorEnvelope, ThreadSummary, ThreadSummaryPage, ThreadTimelineEvent, ThreadTimelinePage, decodeUnknownSync } from '../../clients/typescript/fractal-client/Schema.generated.ts';
import { createReplay, manualClock, realClock } from '../../clients/typescript/fractal-scenarios/src/replay/index.ts';
import { threadMessageSubject } from '../../clients/typescript/fractal-views/deviceSigning.ts';

const port = Number(process.argv[2] ?? 8791);
const scenario = process.argv[3];
if (scenario !== undefined && !['clock-skew', 'thread-actions'].includes(scenario)) throw new Error(`Unknown demo scenario: ${scenario}`);
const mutableThreads = scenario === 'thread-actions';
const API_VERSION = 'st3.client.v0';
const capture = process.env.EXPO_PUBLIC_FRACTAL_LANDING_CAPTURE === '1';
// Validate with the shared decoder, but serialize the wire DTO, not decoded Options.
const checked = (schema, value) => { decodeUnknownSync(schema, 'strict')(value); return value; };
const baseWorld = loadWorld('fleet-mid-refactor', { now: capture ? ANCHOR_MS : Date.now() });
const person = baseWorld.cast.people[0].id;
const inboxAttention = baseWorld.slices.attention.state.attention.find(item =>
  item.person_id === person && item.title === 'Review the loadUser rename');
const inboxEpisode = 'claim/fixture-review-episode';
if (scenario && !inboxAttention?.requester_id) throw new Error('Fixture Inbox attention requires its original requester');
// Only this selected fixture gets an explicit origin episode. The ordinary kit
// world remains unchanged, including all non-thread landing capture routes.
const world = scenario ? { ...baseWorld, slices: { ...baseWorld.slices, attention: {
  ...baseWorld.slices.attention, state: { ...baseWorld.slices.attention.state,
    attention: baseWorld.slices.attention.state.attention.map(item => item === inboxAttention
      ? checked(Attention, { ...item, episode: inboxEpisode, conversation_id: item.requester_id, thread_unread: true }) : item),
  },
} } } : baseWorld;
const replay = createReplay(world, { clock: capture ? manualClock(world.now) : realClock({ start: world.now }) });
// Use the real kit agent so the unchanged native conversation exposes AgentThreads.
const threadAgent = 'agent/example/atlas/builder';
const fixtureNow = Date.parse('2026-11-09T13:00:00Z');
const thread = scenario ? checked(ThreadSummary, {
  id: 'thread/worker-tests', agent_id: threadAgent, title: 'Worker tests — clock-skew fixture',
  state: 'open', revision: 'claim/skew-reopen', title_revision: 'claim/title', message_count: 1, unread: true,
}) : undefined;
const threadEvents = thread ? [
  ...threadClockSkewEvents(fixtureNow, person),
  { kind: 'message', id: 'message/review-ready', actor_id: threadAgent, accepted_at_unix_ms: fixtureNow - 3 * 60_000,
    content: 'Fixture/preliminary: Close at 12:59, then Reopen at 12:58. Authoritative state remains Open despite clock skew.', thread_quote_id: null },
].map(event => checked(ThreadTimelineEvent, event))
  .sort((left, right) => right.accepted_at_unix_ms - left.accepted_at_unix_ms) : [];
const summaries = thread ? [
  thread,
  checked(ThreadSummary, { id: 'thread/rollout-notes', agent_id: threadAgent, title: 'Rollout notes',
    state: 'closed', revision: 'claim/rollout-closed', title_revision: 'claim/rollout-title', message_count: 1, unread: false }),
  checked(ThreadSummary, { id: 'thread/cancellation', agent_id: threadAgent, title: 'Cancellation behavior',
    state: 'open', revision: 'claim/cancellation-current', title_revision: 'claim/cancellation-title', message_count: 1, unread: false }),
  checked(ThreadSummary, { id: 'thread/review-rename', agent_id: inboxAttention.requester_id, title: 'Review rename — Inbox thread fixture',
    state: 'open', revision: 'claim/inbox-current', title_revision: 'claim/inbox-title', message_count: 1, unread: true }),
] : [];
const fixtureMessage = (id, actor, minutes, content) => checked(ThreadTimelineEvent, {
  kind: 'message', id, actor_id: actor, accepted_at_unix_ms: fixtureNow + minutes * 60_000, content, thread_quote_id: null,
});
const timelines = new Map(thread ? [
  [thread.id, threadEvents],
  ['thread/rollout-notes', [
    checked(ThreadTimelineEvent, { kind: 'thread.closed', id: 'event/rollout-closed', actor_id: person, accepted_at_unix_ms: fixtureNow - 35 * 60_000 }),
    fixtureMessage('message/rollout-notes', person, -40, 'Keep the original rollout plan in the release notes.'),
  ]],
  ['thread/cancellation', [
    fixtureMessage('message/cancellation', threadAgent, -5, 'Cancellation keeps the original attempt fenced. Should we cover the retry case too?'),
  ]],
  ['thread/review-rename', [
    fixtureMessage('message/inbox-review', inboxAttention.requester_id, -10, 'Fixture/preliminary: discuss this exact Inbox review origin without marking the request read or answering it.'),
  ]],
] : []);
// The existing clock-skew scenario is unchanged. Only this opt-in fixture grants mutations.
const fixtureStates = ['empty', 'error', 'offline', 'revoked', 'quoted', 'ready'];
let fixtureState = 'ready', fixtureRevision = 1;
const fixtureConnections = new Map();
const fixtureReceipts = new Map();
if (mutableThreads) {
  const added = [
    ['action-preview', 'Writable thread — fixture'],
    ['rejected-preview', 'Rejected changes — fixture'],
    ['quoted-preview', 'Quoted reply — fixture'],
    ...Array.from({ length: 24 }, (_, index) => [`page-row-${index + 1}`, `Pagination row ${index + 1}`]),
    ['older-page', 'Older-page summary — initial'],
  ].map(([id, title]) => checked(ThreadSummary, {
    id: `thread/${id}`, agent_id: threadAgent, title, state: 'open',
    revision: `claim/${id}-initial`, title_revision: `claim/${id}-title`,
    message_count: id === 'quoted-preview' ? 2 : 1, unread: false,
  }));
  summaries.push(...added);
  for (const summary of added) timelines.set(summary.id, [
    fixtureMessage(`message/${summary.id.slice(7)}`, threadAgent, -5, 'Disposable fixture thread; no real agent or authority is affected.'),
  ]);
  timelines.set('thread/quoted-preview', [
    checked(ThreadTimelineEvent, { ...fixtureMessage('message/quoted-response', person, -1, 'Yes — keep this reply attached to the original message.'), thread_quote_id: 'message/quoted-original' }),
    fixtureMessage('message/quoted-original', threadAgent, -2, 'Should this reply retain the original quoted context?'),
  ]);
}
const initialSummaries = [...summaries];
const initialTimelines = new Map(timelines);
const contextThreads = context => {
  if (mutableThreads && fixtureState === 'empty') return [];
  if (mutableThreads && fixtureState === 'quoted') return summaries.filter(item => item.id === 'thread/quoted-preview' && item.agent_id === context?.agent_id);
  if (context?.agent_id) return summaries.filter(item => item.agent_id === context.agent_id);
  if (context?.recently_closed === true) return summaries.filter(item => item.state === 'closed');
  if (thread && context?.origin_source === inboxAttention.source_id && context?.origin_episode === inboxEpisode) {
    return summaries.filter(item => item.id === 'thread/review-rename');
  }
  return [];
};
const threadEnvelope = async (value, valueSchema = ThreadSummary) => {
  const revision = fixtureRevision;
  const base = await (await replay.fetch('http://demo/v1/client/capabilities')).json();
  // EnvelopeOf<T> uses the generated metadata with this endpoint's typed value,
  // not the legacy resource Envelope.value union (which excludes thread pages).
  const snapshot = mutableThreads ? { ...base.snapshot, id: `snapshot/threads-fixture-${revision}`, store_index: base.snapshot.store_index + revision } : base.snapshot;
  return checked(Schema.Struct({ ...Envelope.fields, value: valueSchema }), { ...base, snapshot, value });
};
const fixtureWindow = command => {
  const all = command.collection === 'threads'
    ? command.subject ? summaries.filter(item => item.id === command.subject) : contextThreads(command.thread_context)
    : timelines.get(command.subject) ?? [];
  const limit = command.limit ?? 200;
  return { items: all.slice(0, limit), hasMore: mutableThreads && all.length > limit };
};
const sendFixtureWindow = async (ws, command, changedId) => {
  if (ws.readyState !== WebSocket.OPEN) return;
  if (mutableThreads && ['error', 'revoked'].includes(fixtureState)) {
    ws.send(JSON.stringify(checked(CollectionFrame, {
      kind: 'error', id: command.id, collection: command.collection,
      code: fixtureState === 'revoked' ? 'forbidden' : 'internal',
      message: fixtureState === 'revoked' ? 'Fixture thread grant was revoked.' : 'Fixture thread read failed. Refresh threads to retry.',
      retryable: fixtureState === 'error',
    })));
    return;
  }
  const { items, hasMore } = fixtureWindow(command);
  if (changedId && command.collection === 'threads' && !items.some(item => item.id === changedId)) return;
  if (changedId && command.collection === 'thread-timeline' && command.subject !== changedId) return;
  const { snapshot } = await threadEnvelope(thread);
  if (ws.readyState !== WebSocket.OPEN || !fixtureConnections.get(ws)?.has(command.id)) return;
  const frame = changedId && command.collection === 'threads' ? {
    kind: 'changes', upserts: items.filter(item => item.id === changedId), removes: [],
  } : { kind: 'snapshot', items };
  ws.send(JSON.stringify(checked(CollectionFrame, {
    ...frame, id: command.id, collection: command.collection, snapshot,
    order: items.map(item => item.id), has_more: hasMore,
  })));
};
const broadcastFixture = async changedId => {
  await Promise.all([...fixtureConnections].flatMap(([ws, subscriptions]) =>
    [...subscriptions.values()].map(command => sendFixtureWindow(ws, command, changedId))));
};
const b64url = bytes => Buffer.from(bytes).toString('base64url');
const rawPublic = pair => b64url(pair.publicKey.export({ type: 'spki', format: 'der' }).subarray(-32));
const hostPair = generateKeyPairSync('ed25519');
const personRootPair = generateKeyPairSync('ed25519');
const hostKey = rawPublic(hostPair), personRootKey = rawPublic(personRootPair);
const sha256hex = bytes => createHash('sha256').update(bytes).digest('hex');
// Canonical CBOR maps in Rust's lexical key order with shortest integer lengths, matching the
// app's pairingProof.ts encoder, so the phone's content hashes match these grant ids.
const cbor = value => {
  const out = [];
  const head = (major, count) => {
    if (count < 24) out.push(major * 32 + count);
    else {
      const size = count <= 0xff ? 1 : count <= 0xffff ? 2 : count <= 0xffffffff ? 4 : 8;
      out.push(major * 32 + ({ 1: 24, 2: 25, 4: 26, 8: 27 })[size]);
      let n = BigInt(count);
      const bytes = Array(size);
      for (let at = size - 1; at >= 0; at--) { bytes[at] = Number(n & 255n); n >>= 8n; }
      out.push(...bytes);
    }
  };
  const write = item => {
    if (item === null) out.push(0xf6);
    else if (typeof item === 'boolean') out.push(item ? 0xf5 : 0xf4);
    else if (typeof item === 'number') head(item < 0 ? 1 : 0, item < 0 ? -1 - item : item);
    else if (typeof item === 'string') { const bytes = Buffer.from(item, 'utf8'); head(3, bytes.length); out.push(...bytes); }
    else if (Array.isArray(item)) { head(4, item.length); for (const entry of item) write(entry); }
    else {
      const fields = Object.entries(item).sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0);
      head(5, fields.length);
      for (const [key, entry] of fields) { write(key); write(entry); }
    }
  };
  write(value);
  return Buffer.from(out);
};
const grantId = grant => sha256hex(cbor([grant.batch_id, grant.subject, grant.kind, grant.origin, grant.actor ?? null, grant.body, grant.predecessors]));
const signGrant = (grant, privateKey) => {
  const content = sha256hex(cbor(['smallclaims.claim-content.v1', grant.subject, grant.kind, grant.actor ?? null, grant.body]));
  const text = `smallclaims-claim-v1\n${content}\n${grant.signature.signer}\n\n${grant.signature.key}\n${grant.signature.chain.join(',')}\n${grant.signature.nonce}\n${grant.signature.signed_at_unix_ms}`;
  grant.signature.signature = b64url(ed25519Sign(null, Buffer.from(text, 'utf8'), privateKey));
};
// The read-only demo person root, granted by the invented host authority and signed with its key.
const rootProof = () => {
  const grant = {
    id: '', batch_id: `batch/demo/${randomBytes(16).toString('hex')}`, subject: person, kind: 'principal.key-granted', origin: 'demo-gateway', actor: 'host/demo',
    body: { evidence: [], fields: { issuer: 'host/demo', issuer_key: hostKey, key: personRootKey, role: 'root' } },
    predecessors: [],
    signature: { signer: 'host/demo', key: hostKey, chain: [], nonce: randomBytes(16).toString('hex'), signed_at_unix_ms: Date.now(), signature: '' },
  };
  grant.id = grantId(grant);
  signGrant(grant, hostPair.privateKey);
  return grant;
};
// Pairing and phone observations are demo adapters. Opt-in thread fixtures are
// disposable; clock-skew stays read-only and all non-thread traffic stays kit replay.
const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://demo');
    console.log(request.method, url.pathname);
    const send = (value) => {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify(value));
    };
    const fail = (status, code, message, requestId = 'request/thread-fixture') => {
      response.writeHead(status, { 'content-type': 'application/json' });
      response.end(JSON.stringify(checked(ErrorEnvelope, {
        api_version: API_VERSION, error_version: 'st3.client.error.v0', request_id: requestId,
        code, message, retryable: status >= 500, applied: 'none', details: { applied: 'none' },
      })));
    };
    if (mutableThreads && url.pathname === '/__fixture/threads') {
      if (request.method === 'POST') {
        const chunks = [];
        for await (const chunk of request) chunks.push(chunk);
        const command = JSON.parse(Buffer.concat(chunks).toString('utf8'));
        if (command.reset === true && command.state === undefined && command.update === undefined) {
          summaries.splice(0, summaries.length, ...initialSummaries);
          timelines.clear();
          for (const [id, events] of initialTimelines) timelines.set(id, events);
          fixtureReceipts.clear();
          fixtureState = 'ready';
          fixtureRevision++;
          await broadcastFixture();
        } else if (fixtureStates.includes(command.state) && command.update === undefined && command.reset === undefined) {
          fixtureState = command.state;
          fixtureRevision++;
          if (fixtureState === 'offline') for (const ws of fixtureConnections.keys()) ws.terminate();
          else await broadcastFixture();
        } else if (['older-rest', 'older-live'].includes(command.update) && command.state === undefined && command.reset === undefined) {
          fixtureRevision++;
          const index = summaries.findIndex(item => item.id === 'thread/older-page');
          summaries[index] = checked(ThreadSummary, { ...summaries[index],
            title: command.update === 'older-rest' ? 'Older-page summary — REST update' : 'Older-page summary — live update',
            revision: `claim/older-page-${fixtureRevision}`, title_revision: `claim/older-page-${fixtureRevision}`,
          });
          if (command.update === 'older-live') await broadcastFixture('thread/older-page');
        } else { fail(400, 'validation-failed', 'Unknown thread fixture control.'); return; }
      } else if (request.method !== 'GET') { fail(405, 'validation-failed', 'Use GET or POST for fixture controls.'); return; }
      send({ scenario, state: fixtureState, revision: fixtureRevision, states: fixtureStates,
        reset: { reset: true }, updates: ['older-rest', 'older-live'], writable: 'thread/action-preview',
        rejected: 'thread/rejected-preview', quoted: 'thread/quoted-preview', older: 'thread/older-page' });
      return;
    }
    if (mutableThreads && fixtureState === 'offline') { request.socket.destroy(); return; }
    const threadRoute = url.pathname.startsWith('/v1/client/threads');
    if (mutableThreads && threadRoute && ['error', 'revoked'].includes(fixtureState)) {
      fail(fixtureState === 'revoked' ? 403 : 503, fixtureState === 'revoked' ? 'forbidden' : 'internal',
        fixtureState === 'revoked' ? 'Fixture thread grant was revoked.' : 'Fixture thread read failed. Refresh threads to retry.');
      return;
    }
    if (mutableThreads && request.method === 'POST' && url.pathname === '/v1/client/actions') {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const body = Buffer.concat(chunks);
      const raw = JSON.parse(body.toString('utf8'));
      if (['thread.close', 'thread.reopen', 'thread.rename', 'thread.reply'].includes(raw.type)) {
        const action = checked(ActionRequest, raw);
        if (fixtureState === 'revoked' || action.parameters.target_id === 'thread/rejected-preview') {
          fail(403, 'forbidden', fixtureState === 'revoked' ? 'Fixture thread grant was revoked.' : 'This fixture intentionally rejects thread changes.', action.id);
          return;
        }
        if (fixtureReceipts.has(action.id)) { send(fixtureReceipts.get(action.id)); return; }
        const index = summaries.findIndex(item => item.id === action.parameters.target_id);
        if (index < 0) { fail(404, 'not-found', 'No such fixture thread.', action.id); return; }
        const selected = summaries[index];
        if (action.type === 'thread.rename' && action.parameters.title_revision !== selected.title_revision) {
          fail(409, 'stale-fence', 'The fixture title changed. Refresh and rename again.', action.id);
          return;
        }
        fixtureRevision++;
        const claim = `claim/thread-fixture-${fixtureRevision}`;
        let event;
        if (action.type === 'thread.reply') {
          const fields = {
            content: action.parameters.content, from: action.parameters.signature?.signer ?? person,
            to: selected.agent_id, thread_id: selected.id,
            ...(action.parameters.thread_quote_id ? { thread_quote_id: action.parameters.thread_quote_id } : {}),
          };
          const id = await threadMessageSubject(fields, action.id, async bytes => new Uint8Array(createHash('sha256').update(bytes).digest()));
          event = checked(ThreadTimelineEvent, { kind: 'message', id, actor_id: fields.from,
            accepted_at_unix_ms: fixtureNow + fixtureRevision * 1000, content: fields.content, thread_quote_id: fields.thread_quote_id ?? null });
          summaries[index] = checked(ThreadSummary, { ...selected, message_count: (selected.message_count ?? 0) + 1 });
        } else if (action.type === 'thread.rename') {
          summaries[index] = checked(ThreadSummary, { ...selected, title: action.parameters.title, revision: claim, title_revision: claim });
          event = checked(ThreadTimelineEvent, { kind: 'thread.renamed', id: claim, actor_id: person,
            accepted_at_unix_ms: fixtureNow + fixtureRevision * 1000, previous_title: selected.title, title: action.parameters.title });
        } else {
          summaries[index] = checked(ThreadSummary, { ...selected, state: action.type === 'thread.close' ? 'closed' : 'open', revision: claim });
          event = checked(ThreadTimelineEvent, { kind: action.type === 'thread.close' ? 'thread.closed' : 'thread.reopened',
            id: claim, actor_id: person, accepted_at_unix_ms: fixtureNow + fixtureRevision * 1000 });
        }
        timelines.set(selected.id, [event, ...timelines.get(selected.id)]);
        const receipt = await threadEnvelope({
          kind: 'action-result', action_id: action.id, operation_id: `operation/thread-fixture-${fixtureRevision}`,
          status: 'completed', affected_ids: action.type === 'thread.reply' ? [selected.id, event.id] : [selected.id],
          ...(action.type === 'thread.reply' ? {} : { metadata_claim_id: claim }),
          snapshot_id: `snapshot/threads-fixture-${fixtureRevision}`,
        }, ActionResult);
        fixtureReceipts.set(action.id, receipt);
        send(receipt);
        await broadcastFixture(selected.id);
        return;
      }
      // Non-thread actions retain the unchanged replay adapter after consuming the request body.
      const result = await replay.fetch(url, {
        method: request.method,
        headers: new Headers(Object.entries(request.headers).flatMap(([name, value]) => value === undefined ? [] : [[name, Array.isArray(value) ? value.join(', ') : value]])),
        body,
      });
      response.writeHead(result.status, Object.fromEntries(result.headers));
      response.end(Buffer.from(await result.arrayBuffer()));
      return;
    }
    if (url.pathname === '/v1/client/capabilities' && !request.headers.authorization) {
      send({ api_version: API_VERSION, capabilities: [{ id: 'device-key-proofs', state: 'granted', version: 1 }] });
      return;
    }
    if (/^\/v1\/client\/pairings\/[^/]+\/complete$/.test(url.pathname)) {
      // Real invented-key proofs: the phone still performs its own read-only verification.
      const envelope = await (await replay.fetch('http://demo/v1/client/capabilities')).json();
      send({ ...envelope, value: { kind: 'paired-session', credential: 'demo-credential-0123456789abcdef0123456789abcdef', device_id: 'device/' + '0'.repeat(23) + '1', expires_at: '2099-12-31T00:00:00Z', person_id: person, scopes: ['read.projections', 'attention', 'message.send', 'terminal.input'], session_actor: person, device_key_chain: [], device_key_proofs: [], person_root_key_proof: rootProof() } });
      return;
    }
    if (url.pathname === '/v1/client/observations') {
      // Phone measurements use the flat observation envelope, not a world projection.
      send({ api_version: API_VERSION, request_id: 'request/demo-observations', value: { accepted: true } });
      return;
    }
    if (thread && request.method === 'GET') {
      if (url.pathname === '/v1/client/capabilities') {
        const base = await (await replay.fetch(url)).json();
        send(checked(Envelope, { ...base, value: { ...base.value, capabilities: [
          ...base.value.capabilities,
          { id: 'threads-v1', state: 'granted', version: 2 },
          { id: 'thread-summary-feed', state: 'granted', version: 1 },
          ...(mutableThreads ? ['thread.reply', 'thread.close', 'thread.reopen', 'thread.rename'].map(id => ({
            id, state: fixtureState === 'revoked' ? 'ungranted' : 'granted', version: 1,
          })) : []),
        ] } }));
        return;
      }
      const route = url.pathname.split('/').slice(3).map(decodeURIComponent);
      if (route[0] === 'threads' && route.length === 1) {
        const all = contextThreads({
          agent_id: url.searchParams.get('agent_id'), recently_closed: url.searchParams.get('recently_closed') === 'true',
          origin_source: url.searchParams.get('origin_source'), origin_episode: url.searchParams.get('origin_episode'),
        });
        const offset = mutableThreads ? Number(url.searchParams.get('cursor') ?? 0) : url.searchParams.has('cursor') ? all.length : 0;
        const limit = Math.max(1, Math.min(200, Number(url.searchParams.get('limit') ?? 200)));
        const items = all.slice(offset, offset + limit);
        send(await threadEnvelope({ items, next_cursor: mutableThreads && offset + limit < all.length ? String(offset + limit) : null }, ThreadSummaryPage));
        return;
      }
      const selected = summaries.find(item => item.id === `thread/${route[1]}`);
      if (route[0] === 'threads' && selected) {
        if (route.length === 2) { send(await threadEnvelope(selected)); return; }
        if (route.length === 3 && route[2] === 'timeline') {
          const items = url.searchParams.has('cursor') ? [] : timelines.get(selected.id);
          send(await threadEnvelope({ items: items.slice(0, Number(url.searchParams.get('limit') ?? 200)), next_cursor: null }, ThreadTimelinePage));
          return;
        }
      }
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const result = await replay.fetch(url, {
      method: request.method,
      headers: new Headers(Object.entries(request.headers).flatMap(([name, value]) => value === undefined ? [] : [[name, Array.isArray(value) ? value.join(', ') : value]])),
      ...(chunks.length === 0 ? {} : { body: Buffer.concat(chunks) }),
    });
    response.writeHead(result.status, Object.fromEntries(result.headers));
    response.end(Buffer.from(await result.arrayBuffer()));
  } catch (error) {
    console.error(error);
    response.writeHead(500, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ message: 'Demo replay failed' }));
  }
});

const collections = new WebSocketServer({ noServer: true, handleProtocols: protocols => [...protocols][0] });
server.on('upgrade', (request, socket, head) => {
  if (mutableThreads && fixtureState === 'offline') { socket.destroy(); return; }
  collections.handleUpgrade(request, socket, head, ws => collections.emit('connection', ws, request));
});
collections.on('connection', (ws, request) => {
  ws.pause();
  const protocols = request.headers['sec-websocket-protocol']?.split(',').map(value => value.trim()) ?? [];
  const upstream = replay.socket(new URL(request.url, 'ws://demo').href, protocols, {});
  upstream.onopen = () => ws.resume();
  upstream.onmessage = event => { if (ws.readyState === WebSocket.OPEN) ws.send(event.data); };
  upstream.onerror = () => ws.close(1011, 'Demo replay failed');
  // 1006 is a local abnormal-close indication and cannot be sent on the wire.
  upstream.onclose = event => ws.close(event.code === 1006 ? 1011 : event.code, event.reason);
  const threadSubscriptions = new Map();
  fixtureConnections.set(ws, threadSubscriptions);
  ws.on('message', async data => {
    try {
      if (thread) {
        const raw = JSON.parse(String(data));
        if (raw.kind === 'unsubscribe' && threadSubscriptions.delete(raw.id)) return;
        if (raw.kind === 'subscribe' && ['threads', 'thread-timeline'].includes(raw.collection)) {
          const command = checked(CollectionCommand, raw);
          threadSubscriptions.set(command.id, command);
          await sendFixtureWindow(ws, command);
          return;
        }
      }
      upstream.send(String(data));
    } catch (error) {
      console.error(error);
      ws.close(1011, 'Demo fixture failed');
    }
  });
  ws.on('error', () => upstream.close());
  // Preserve the real gateway's eight-second protocol heartbeat, even when idle.
  const heartbeat = setInterval(() => ws.ping(), 8000);
  heartbeat.unref();
  ws.on('close', () => { fixtureConnections.delete(ws); clearInterval(heartbeat); upstream.close(); });
});
server.on('close', () => replay.close());
server.listen(port, '0.0.0.0', () => console.log(`demo gateway on :${server.address().port}`));
