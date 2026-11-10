import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { createHash, generateKeyPairSync, sign, verify } from 'node:crypto';
import { API_VERSION, ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { claimPairingKey, pairingClaimRefusal, pairingClaimRequest, pairingPossessionProof, pairingCompletionBinding } from './pairingClaim.ts';
import { isTestPairingClient, pairingScanSession, scanOutcome } from './pairingScan.ts';
import { parsePairingQr } from './pairingQr.ts';

const candidate = { gateway: 'https://pair.example', id: 'pairing/0123456789abcdef01234567', code: 'ABCD2345' };
const qr = JSON.stringify({ _tag: 'fractal.pairing', version: 1, ...candidate, expires_at_unix_ms: 9_000_000_000_000 });
const key = { key: 'P-256 public key', storage: 'secure-enclave', handle: 'prepared-key' };
const setup = (testClient, claim = async () => {}) => {
  const calls = [];
  const session = pairingScanSession(testClient, {
    create: async () => { calls.push('create'); return key; },
    remove: async handle => { calls.push(['remove', handle]); },
    sign: async () => 'signature',
    claim: async request => { calls.push(['claim', request.key]); await claim(request); },
  });
  const camera = async () => { calls.push('camera'); return qr; };
  return { session, calls, camera };
};

test('test flag is strict and absent flags preserve normal pairing', () => {
  for (const extra of [undefined, {}, { testClientMode: false }, { testClientMode: 'true' }]) assert.equal(isTestPairingClient(extra), false);
  assert.equal(isTestPairingClient({ testClientMode: true }), true);
});

test('shipped scan coordinator creates before camera, claims, and transfers exactly the same key once', async () => {
  const { session, calls, camera } = setup(true);
  const raw = await session.scan(camera);
  const outcome = scanOutcome(parsePairingQr(raw), '');
  assert.equal(outcome.kind, 'prefill');
  assert.deepEqual(calls, ['create', 'camera']);
  assert.throws(() => session.takeCompletionKey(candidate), /scan and claim/);
  await session.claim(outcome.candidate);
  assert.throws(() => session.takeCompletionKey({ ...candidate, code: 'OTHER234' }), /scan and claim/);
  assert.equal(session.takeCompletionKey(candidate), key);
  assert.throws(() => session.takeCompletionKey(candidate), /scan and claim/);
  assert.deepEqual(calls, ['create', 'camera', ['claim', key]]);
});

test('normal shipped scan does not generate or claim a key', async () => {
  const { session, calls, camera } = setup(false);
  assert.equal(await session.scan(camera), qr);
  await session.claim(candidate);
  await session.discard();
  assert.deepEqual(calls, ['camera']);
});

test('typed binding refusal is surfaced, clears the candidate key, and never retries unbound', async () => {
  const refusal = new ClientError({ code: 'pairing-key-binding-required', message: 'binding required', retryable: false }, 403);
  const { session, calls, camera } = setup(true, async () => { throw refusal; });
  await session.scan(camera);
  await assert.rejects(session.claim(candidate), error => error === refusal);
  assert.match(pairingClaimRefusal(refusal), /Pairing refused \(pairing-key-binding-required\)/);
  assert.throws(() => session.takeCompletionKey(candidate), /scan and claim/);
  assert.deepEqual(calls, ['create', 'camera', ['claim', key], ['remove', key.handle]]);
});

test('cancelled camera discards its key without claiming', async () => {
  const { session, calls } = setup(true);
  assert.equal(await session.scan(async () => null), null);
  assert.deepEqual(calls, ['create', ['remove', key.handle]]);
});

const hash = async bytes => new Uint8Array(createHash('sha256').update(bytes).digest());
test('real claim adapter discovers v2 and fails closed until its generated SDK binding exists', async () => {
  for (const version of [1, 2]) {
    const requests = [];
    const { session, camera } = setup(true, claim => claimPairingKey(claim, {
      hash, fetch: async url => {
        requests.push(url);
        return new Response(JSON.stringify({ api_version: API_VERSION, capabilities: [{ id: 'pairing-key-binding', version, state: 'granted' }] }));
      },
    }));
    await session.scan(camera);
    await assert.rejects(session.claim(candidate), version === 1 ? /does not support scanning-key claims/ : /cannot submit a scanning-key claim/);
    assert.throws(() => session.takeCompletionKey(candidate), /scan and claim/);
    assert.deepEqual(requests, [`${candidate.gateway}/v1/client/capabilities?accepts=pairing-key-binding`]);
  }
});

test('Fleet and store use this coordinator, not an alternate test-only scan path', () => {
  const fleet = readFileSync(new URL('./screens/Fleet.tsx', import.meta.url), 'utf8');
  const store = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  assert.match(fleet, /actions\.scanPairingQr\(scanPairingQrNative\)/);
  assert.match(fleet, /await actions\.prefillScannedPairing\(gateway, id, code\)/);
  assert.match(store, /isTestPairingClient\(Constants\.expoConfig\?\.extra\)/);
  assert.match(store, /scanSession\.current!\.claim\(\{ gateway, id, code \}\)/);
  assert.match(store, /testPairingClient \? scanSession\.current!\.takeCompletionKey\(\{ gateway, id, code \}\) : await createDeviceKey\(\)/);
  assert.match(store, /device_public_key: made\.key/);
});

test('claim proof verifies as raw 64-byte P-256 over the exact PR 1b three-line message', async () => {
  const pair = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const jwk = pair.publicKey.export({ format: 'jwk' });
  const point = Buffer.concat([Buffer.from([4]), Buffer.from(jwk.x, 'base64url'), Buffer.from(jwk.y, 'base64url')]);
  const deviceKey = { ...key, key: `p256:${point.toString('base64url')}` };
  const request = await pairingClaimRequest({ ...candidate, key: deviceKey, sign: async (text, handle) => {
    assert.equal(handle, key.handle);
    return sign('sha256', Buffer.from(text, 'utf8'), { key: pair.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
  } }, hash);
  const expected = ['pairing.claim.v1', candidate.id, createHash('sha256').update(candidate.code, 'utf8').digest('hex')].join('\n');
  const signature = Buffer.from(request.proof, 'base64url');
  assert.equal(signature.length, 64);
  assert.equal(request.device_public_key, deviceKey.key);
  assert.equal(verify('sha256', Buffer.from(expected, 'utf8'), { key: pair.publicKey, dsaEncoding: 'ieee-p1363' }, signature), true);
  assert.equal(verify('sha256', Buffer.from(expected + '\n', 'utf8'), { key: pair.publicKey, dsaEncoding: 'ieee-p1363' }, signature), false);
  const completeProof = await pairingPossessionProof('pairing.complete.v1', { ...candidate, key: deviceKey,
    sign: async text => sign('sha256', Buffer.from(text), { key: pair.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url'),
  }, hash);
  assert.equal(verify('sha256', Buffer.from(expected), { key: pair.publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(completeProof, 'base64url')), false);
  assert.equal(verify('sha256', Buffer.from(expected.replace('pairing.claim.v1', 'pairing.complete.v1')), { key: pair.publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(completeProof, 'base64url')), true);
  await assert.rejects(pairingCompletionBinding({ ...candidate, key: deviceKey,
    sign: async text => sign('sha256', Buffer.from(text), { key: pair.privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url'),
  }, hash), /cannot submit a completion possession proof/);
});
test.todo('PR 1b SDK: generated claim request/response binding and submitted server refusals');
test.todo('PR 1b SDK: generated completion possession-proof field binding');
