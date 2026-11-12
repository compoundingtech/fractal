import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { approveRenewal, pendingRenewals, pendingRenewalText, renewalApprovalText, renewalOfferText, renewalResponseValue, watchRenewalForeground } from './deviceRenewal.ts';
import { ForegroundGate } from './foreground.ts';

test('pending and completed renewal envelopes accept both namespaces, not unrelated versions', () => {
  for (const value of [{ kind: 'device-renewals', renewals: [] }, { kind: 'paired-session', credential: 'replacement' }]) {
    for (const api_version of ['st3.client.v0', 'fractal.client.v0']) {
      assert.equal(renewalResponseValue({ api_version, value }), value);
    }
    for (const api_version of ['unrelated.client.v0', 'st3.client.v1', undefined]) {
      assert.throws(() => renewalResponseValue({ api_version, value }), /Invalid renewal response/);
    }
  }
  for (const envelope of [null, {}, { api_version: 'fractal.client.v0' }]) {
    assert.throws(() => renewalResponseValue(envelope), /Invalid renewal response/);
  }
});

test('the store polling subscription runs on active startup and each foreground, never background', () => {
  const gate = new ForegroundGate('active');
  let calls = 0;
  const stop = watchRenewalForeground(gate, async () => { calls++; });
  assert.equal(calls, 1);
  gate.update('background'); gate.update('inactive');
  assert.equal(calls, 1);
  gate.update('active'); gate.update('active');
  assert.equal(calls, 2);
  stop(); gate.update('background'); gate.update('active');
  assert.equal(calls, 2);
  const background = new ForegroundGate('background');
  const stopBackground = watchRenewalForeground(background, async () => { calls++; });
  assert.equal(calls, 2);
  background.update('active'); assert.equal(calls, 3);
  stopBackground();
});

const vector = JSON.parse(readFileSync(new URL('../../fixtures/clients/device-pairing-proofs-v1.json', import.meta.url)));
const now = 1_800_000_000_000;
const profile = () => ({ url: 'https://member.example', credential: 'previous-bearer-never-sent-for-renewal', personRootFingerprint: vector.fingerprint,
  signing: { key: 'p256:old-device-key', storage: 'secure-enclave', handle: 'old-handle', person: vector.person, chain: ['old-grant'] }, routing: { carrier: 'tailscale', members: [] } });
const offer = () => ({ renewal_id: 'renewal/example', migration_epoch: 'example-epoch', issuer_member: 'host/example', person_id: vector.person, requested_by: 'agent/example',
  scopes: ['control.messages'], old_public_key: 'p256:old-device-key', nonce: 'fresh-nonce', expires_at_unix_ms: now + 300_000, grant_expires_at_unix_ms: now + 29 * 24 * 60 * 60_000,
  person_root_key_proof: structuredClone(vector.proofs[1]), issuer_signature: 'offer-signature' });
const session = () => ({ kind: 'paired-session', device_id: 'device/0123456789abcdef01234567', person_id: vector.person,
  session_actor: `${vector.person}/session/example`, credential: 'new-test-bearer-at-least-thirty-two-characters', expires_at: new Date(now + 29 * 24 * 60 * 60_000).toISOString(),
  scopes: ['control.messages'], device_key_chain: [...vector.chain], device_key_proofs: structuredClone(vector.proofs) });

function setup() {
  const previous = profile(), challenge = offer(), events = [], requests = [];
  let saved = structuredClone(previous);
  const ports = {
    now: () => now,
    crypto: {
      async hash(bytes) { return new Uint8Array(createHash('sha256').update(bytes).digest()); },
      async verify(key, text, signature) {
        if (text.startsWith('fractal-device-renewal-offer-v1\n')) return signature === 'offer-signature' && text === renewalOfferText(challenge);
        const p256 = key.startsWith('p256:');
        const raw = Buffer.from(p256 ? key.slice(5) : key, 'base64url');
        const prefix = Buffer.from(p256 ? '3059301306072a8648ce3d020106082a8648ce3d030107034200' : '302a300506032b6570032100', 'hex');
        try { return verify(p256 ? 'sha256' : null, Buffer.from(text), { key: createPublicKey({ key: Buffer.concat([prefix, raw]), format: 'der', type: 'spki' }), dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url')); }
        catch { return false; }
      },
    },
    async post(url, path, body) { requests.push({ url, path, body }); return path.endsWith('/pending') ? { kind: 'device-renewals', renewals: [structuredClone(challenge)] } : session(); },
    async sign(text, handle) { events.push(['sign', text, handle]); return 'old-signature'; },
    async create() { events.push(['create']); return { key: vector.device_key, storage: 'secure-enclave', handle: 'candidate-handle' }; },
    async approve(text, handle) { events.push(['approve', text, handle]); return 'candidate-signature'; },
    async remove(handle) { events.push(['remove', handle]); },
    async read() { return saved; },
    async save(key, value) { events.push(['save', key]); saved = structuredClone(value); },
  };
  return { ports, previous, challenge, events, requests, saved: () => saved };
}

test('pending uses a domain-separated old-key proof, never the saved bearer, and verifies the pinned issuer', async () => {
  const t = setup();
  assert.deepEqual(await pendingRenewals(t.previous, t.previous.url, 'st3.client.v0', t.ports), [t.challenge]);
  assert.deepEqual(t.events[0], ['sign', pendingRenewalText(t.previous.signing.key, now), 'old-handle']);
  assert.equal(JSON.stringify(t.requests).includes(t.previous.credential), false);
  assert.equal(t.requests[0].body.timestamp_unix_ms, now);
  const unpinned = { ...t.previous, personRootFingerprint: undefined };
  assert.deepEqual(await pendingRenewals(unpinned, unpinned.url, 'st3.client.v0', t.ports), []);
});

test('pending refuses changed signed tuples, another old key, expired offers and wrong pins', async () => {
  for (const change of [value => { value.scopes.push('control.terminals'); }, value => { value.requested_by = 'agent/other'; }, value => { value.old_public_key = 'another-key'; }, value => { value.expires_at_unix_ms = now; }, value => { value.person_id = 'person/other'; }]) {
    const t = setup(); const altered = structuredClone(t.challenge); change(altered);
    t.ports.post = async () => ({ kind: 'device-renewals', renewals: [altered] });
    await assert.rejects(pendingRenewals(t.previous, t.previous.url, 'st3.client.v0', t.ports));
    assert.deepEqual(t.saved(), t.previous);
  }
  const t = setup(); t.previous.personRootFingerprint = vector.other_fingerprint;
  await assert.rejects(pendingRenewals(t.previous, t.previous.url, 'st3.client.v0', t.ports), /fingerprint mismatch/);
});

test('approval binds both signatures to all tuple fields and candidate then commits one verified profile', async () => {
  const t = setup();
  const next = await approveRenewal('profile', t.previous, t.previous.url, t.challenge, 'st3.client.v0', t.ports);
  const text = renewalApprovalText(t.challenge, vector.device_key);
  assert.deepEqual(t.events.slice(0, 3), [['create'], ['approve', text, 'candidate-handle'], ['sign', text, 'old-handle']]);
  assert.deepEqual(t.events.slice(-2), [['save', 'profile'], ['remove', 'old-handle']]);
  assert.equal(next.signing.key, vector.device_key); assert.equal(next.signing.handle, 'candidate-handle');
  assert.equal(next.personRootFingerprint, t.previous.personRootFingerprint); assert.deepEqual(next.routing, t.previous.routing);
  assert.deepEqual(t.saved(), next);
  assert.equal(t.requests[0].body.nonce, t.challenge.nonce);
  assert.equal(t.requests[0].body.expires_at_unix_ms, t.challenge.expires_at_unix_ms);
  assert.equal(t.requests[0].body.old_signature, 'old-signature'); assert.equal(t.requests[0].body.candidate_signature, 'candidate-signature');
  assert.equal(JSON.stringify(t.requests).includes(t.previous.credential), false);
});

test('completion resolves the current member route after native approval redials Fabric', async () => {
  const t = setup();
  let route = 'http://127.0.0.1:41001';
  t.ports.approve = async () => { route = 'http://127.0.0.1:41002'; return 'candidate-signature'; };
  await approveRenewal('profile', t.previous, async () => route, t.challenge, 'st3.client.v0', t.ports);
  assert.equal(t.requests[0].url, 'http://127.0.0.1:41002');
});

test('cancellation, unavailable enclave, protocol refusal, changed grants and secure-write failure retain the previous profile', async () => {
  const cases = [
    ports => { ports.approve = async () => { throw new Error('cancelled'); }; },
    ports => { ports.create = async () => ({ key: vector.device_key, storage: 'software', handle: 'candidate-handle' }); },
    ports => { ports.post = async () => { throw new Error('refused'); }; },
    ports => { ports.post = async () => ({ ...session(), person_id: 'person/other' }); },
    ports => { ports.post = async () => ({ ...session(), scopes: ['control.messages', 'control.terminals'] }); },
    ports => { ports.post = async () => { const value = session(); value.device_key_proofs[0].body.fields.key = 'p256:another'; return value; }; },
    ports => { ports.save = async () => { throw new Error('storage unavailable'); }; },
    ports => { ports.read = async () => ({ ...profile(), credential: 'concurrent-replacement' }); },
  ];
  for (const change of cases) {
    const t = setup(); change(t.ports);
    await assert.rejects(approveRenewal('profile', t.previous, t.previous.url, t.challenge, 'st3.client.v0', t.ports));
    assert.deepEqual(t.saved(), t.previous);
    assert.deepEqual(t.events.at(-1), ['remove', 'candidate-handle']);
    assert.equal(t.events.some(event => event[0] === 'remove' && event[1] === 'old-handle'), false);
  }
});

test('every approval tuple field and the candidate affect the signed bytes', () => {
  const value = offer(), text = renewalApprovalText(value, 'candidate');
  for (const key of ['renewal_id', 'migration_epoch', 'issuer_member', 'person_id', 'old_public_key', 'nonce']) assert.notEqual(renewalApprovalText({ ...value, [key]: `${value[key]}-changed` }, 'candidate'), text);
  assert.notEqual(renewalApprovalText({ ...value, scopes: ['other'] }, 'candidate'), text);
  assert.notEqual(renewalApprovalText({ ...value, expires_at_unix_ms: value.expires_at_unix_ms + 1 }, 'candidate'), text);
  assert.notEqual(renewalApprovalText({ ...value, grant_expires_at_unix_ms: value.grant_expires_at_unix_ms + 1 }, 'candidate'), text);
  assert.notEqual(renewalApprovalText(value, 'another-candidate'), text);
});
