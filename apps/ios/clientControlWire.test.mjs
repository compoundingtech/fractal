import assert from 'node:assert/strict';
import { createHash, createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { clientResultCanonicalBytes, CLIENT_RESULT_SIGNED_FIELDS, visibleStateCanonicalJson, visibleStateDigest } from '../../clients/typescript/fractal-client/ClientControl.generated.ts';
import { revokeClientControl, signClientResult } from './clientControlWire.ts';

const golden = JSON.parse(readFileSync(new URL('../../fixtures/clients/client-control-v1.golden.json', import.meta.url)));
const sec1 = Buffer.from(golden.public_key_sec1_base64url, 'base64url');
const publicKey = createPublicKey({ key: Buffer.concat([
  Buffer.from('3059301306072a8648ce3d020106082a8648ce3d030107034200', 'hex'), sec1,
]), format: 'der', type: 'spki' });
const verifies = (bytes, signature) => verify('sha256', Buffer.from(bytes), { key: publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url'));
const privateKey = createPrivateKey({ key: { kty: 'EC', crv: 'P-256',
  x: sec1.subarray(1, 33).toString('base64url'), y: sec1.subarray(33).toString('base64url'),
  d: Buffer.from(golden.private_scalar_hex_for_tests, 'hex').toString('base64url'),
}, format: 'jwk' });

for (const vector of golden.vectors) test(`generated ${vector.name} JCS, digest, canonical bytes and P-256 binding`, async () => {
  assert.equal(visibleStateCanonicalJson(vector.input.visible_state), vector.canonical_visible_state);
  assert.equal(await visibleStateDigest(vector.input.visible_state), vector.visible_state_sha256);
  const bytes = await clientResultCanonicalBytes(vector.input);
  assert.deepEqual(Buffer.from(bytes), Buffer.from(vector.canonical_bytes));
  assert.equal(verifies(bytes, vector.signature_p1363_base64url), true);
  for (const binding of vector.negative_bindings) {
    assert.equal(verifies(await clientResultCanonicalBytes(binding), vector.signature_p1363_base64url), false);
  }
  assert.equal(verifies(bytes, golden.message_domain_signature_p1363_base64url), false);
  assert.equal(verifies(golden.message_domain_bytes, vector.signature_p1363_base64url), false);
});

test('message-domain fixture signature is valid only in its own domain', () => {
  assert.equal(verifies(golden.message_domain_bytes, golden.message_domain_signature_p1363_base64url), true);
});

for (const vector of golden.vectors) test(`adapter signs ${vector.name} golden bytes with the selected key and generated envelope`, async () => {
  const binding = vector.input;
  const signing = { key: `p256:${golden.public_key_sec1_base64url}`, person: 'person/test', chain: ['grant/test'], storage: 'secure-enclave', handle: 'selected-key' };
  const result = await signClientResult(binding, signing, async (text, handle) => {
    assert.equal(handle, signing.handle);
    assert.deepEqual(Buffer.from(text), Buffer.from(vector.canonical_bytes));
    return sign('sha256', Buffer.from(text), { key: privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
  }, 123, 'fresh-nonce');
  assert.deepEqual(result, { command_id: binding.command_id, outcome: binding.outcome, result: binding.visible_state, binding,
    signature: { format: 'fields-v1', signer: signing.person, key: signing.key, chain: signing.chain,
      signed_fields: [...CLIENT_RESULT_SIGNED_FIELDS], signed_at_unix_ms: 123, nonce: 'fresh-nonce', signature: result.signature.signature } });
  assert.equal(verifies(await clientResultCanonicalBytes(binding), result.signature.signature), true);
});

test('injected SHA-256 signs every golden vector without global WebCrypto', async () => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'crypto');
  Object.defineProperty(globalThis, 'crypto', { configurable: true, value: undefined });
  try {
    for (const vector of golden.vectors) {
      let hashes = 0;
      let signatures = 0;
      const result = await signClientResult(vector.input, { person: 'person/test', key: 'key/test', chain: [] }, async text => {
        signatures++;
        assert.equal(text, vector.canonical_bytes);
        return sign('sha256', Buffer.from(text), { key: privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64url');
      }, 123, 'nonce', async bytes => {
        hashes++;
        assert.equal(new TextDecoder().decode(bytes), vector.canonical_visible_state);
        return new Uint8Array(createHash('sha256').update(bytes).digest());
      });
      assert.equal(hashes, 1);
      assert.equal(signatures, 1);
      assert.equal(verifies(vector.canonical_bytes, result.signature.signature), true);
    }
  } finally {
    if (descriptor) Object.defineProperty(globalThis, 'crypto', descriptor);
    else delete globalThis.crypto;
  }
});

test('invalid Unicode in any generated string refuses the result without signing', async () => {
  const binding = golden.vectors.find(vector => vector.input.visible_state.opened_item !== null).input;
  const paths = [
    ...['purpose', 'command_kind', 'device', 'session', 'grant_id', 'run_id', 'command_id', 'outcome', 'error_code'].map(field => [field]),
    ...['route', 'screen'].map(field => ['visible_state', field]),
    ['visible_state', 'visible_item_ids', 0],
    ...['id', 'title', 'body'].map(field => ['visible_state', 'opened_item', field]),
  ];
  for (const path of paths) for (const invalid of ['\ud800', '\udc00']) {
    const input = structuredClone(binding);
    let target = input;
    for (const key of path.slice(0, -1)) target = target[key];
    target[path.at(-1)] = invalid;
    const result = await signClientResult(input, { person: 'person/test', key: 'key/test', chain: [] },
      async () => assert.fail(`signed invalid Unicode at ${path.join('.')}`), 123, 'nonce');
    assert.equal(result, undefined, path.join('.'));
  }
});

test('hashing and signing failures propagate rather than refusing a result', async () => {
  const binding = golden.vectors[0].input;
  const signing = { person: 'person/test', key: 'key/test', chain: [] };
  const failure = new Error('digest failed');
  await assert.rejects(signClientResult(binding, signing, async () => assert.fail('signed after digest failure'),
    123, 'nonce', async () => { throw failure; }), error => error === failure);
  const signingFailure = new Error('signing failed');
  await assert.rejects(signClientResult(binding, signing, async () => { throw signingFailure; },
    123, 'nonce'), error => error === signingFailure);
});

test('revoke awaits the SDK and propagates failure', async () => {
  let revoked;
  assert.equal(await revokeClientControl({ async clientTestRevoke(device) { revoked = device; } }, 'device/test'), undefined);
  assert.equal(revoked, 'device/test');
  const failure = new Error('revoke failed');
  await assert.rejects(revokeClientControl({ async clientTestRevoke() { throw failure; } }, 'device/test'), error => error === failure);
});
