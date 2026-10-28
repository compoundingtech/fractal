// Demo-only gateway for Debug simulator screenshots; no login or real authority.
// Never point a Release build or a real device at this unauthenticated server.
// node --experimental-transform-types demoGateway.mjs [port]
import { createHash, generateKeyPairSync, randomBytes, sign as ed25519Sign } from 'node:crypto';
import http from 'node:http';
import { WebSocket, WebSocketServer } from 'ws';
import { ANCHOR_MS, loadWorld } from '../../clients/typescript/fractal-scenarios/src/index.ts';
import { createReplay, manualClock, realClock } from '../../clients/typescript/fractal-scenarios/src/replay/index.ts';

const port = Number(process.argv[2] ?? 8791);
const API_VERSION = 'st3.client.v0';
const capture = process.env.EXPO_PUBLIC_FRACTAL_LANDING_CAPTURE === '1';
const world = loadWorld('fleet-mid-refactor', { now: capture ? ANCHOR_MS : Date.now() });
const replay = createReplay(world, { clock: capture ? manualClock(world.now) : realClock({ start: world.now }) });
const person = world.cast.people[0].id;
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
// Only pairing and phone observations are demo adapters. All world reads, actions and
// collection/terminal traffic belong to the kit, including its time-based changes.
const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://demo');
    console.log(request.method, url.pathname);
    const send = (value) => {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify(value));
    };
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
server.on('upgrade', (request, socket, head) => collections.handleUpgrade(request, socket, head, ws => collections.emit('connection', ws, request)));
collections.on('connection', (ws, request) => {
  ws.pause();
  const protocols = request.headers['sec-websocket-protocol']?.split(',').map(value => value.trim()) ?? [];
  const upstream = replay.socket(new URL(request.url, 'ws://demo').href, protocols, {});
  upstream.onopen = () => ws.resume();
  upstream.onmessage = event => { if (ws.readyState === WebSocket.OPEN) ws.send(event.data); };
  upstream.onerror = () => ws.close(1011, 'Demo replay failed');
  // 1006 is a local abnormal-close indication and cannot be sent on the wire.
  upstream.onclose = event => ws.close(event.code === 1006 ? 1011 : event.code, event.reason);
  ws.on('message', data => upstream.send(String(data)));
  ws.on('error', () => upstream.close());
  // Preserve the real gateway's eight-second protocol heartbeat, even when idle.
  const heartbeat = setInterval(() => ws.ping(), 8000);
  heartbeat.unref();
  ws.on('close', () => { clearInterval(heartbeat); upstream.close(); });
});
server.on('close', () => replay.close());
server.listen(port, '0.0.0.0', () => console.log(`demo gateway on :${server.address().port}`));
