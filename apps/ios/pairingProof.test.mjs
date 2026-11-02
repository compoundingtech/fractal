import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fabricGrantProblem, pairingFabricNode, validatePairingTrust, verifyPairing } from './pairingProof.ts';

// Public receipts exported from a real, isolated daemon predating fingerprint support.
const vector = JSON.parse(readFileSync(new URL('../../fixtures/clients/device-pairing-proofs-v1.json', import.meta.url)));
const crypto = {
  async hash(bytes) { return new Uint8Array(createHash('sha256').update(bytes).digest()); },
  async verify(key, text, signature) {
    const p256 = key.startsWith('p256:');
    const raw = Buffer.from(p256 ? key.slice(5) : key, 'base64url');
    const prefix = Buffer.from(p256 ? '3059301306072a8648ce3d020106082a8648ce3d030107034200' : '302a300506032b6570032100', 'hex');
    try {
      return verify(p256 ? 'sha256' : null, Buffer.from(text), { key: createPublicKey({ key: Buffer.concat([prefix, raw]), format: 'der', type: 'spki' }), dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url'));
    } catch { return false; }
  },
};
function session() {
  return { kind: 'paired-session', device_id: 'device/0123456789abcdef01234567', person_id: vector.person,
    session_actor: 'person/avery/session/example', credential: 'test-only-bearer-not-a-real-credential-0000',
    expires_at: '2026-11-01T00:00:00Z', scopes: ['control.messages'], device_key_chain: [...vector.chain], device_key_proofs: structuredClone(vector.proofs) };
}

test('missing, truncated, noncanonical and conflicting fingerprint input refuses before completion', () => {
  assert.throws(() => validatePairingTrust('', false), /fingerprint/);
  assert.throws(() => validatePairingTrust('sha256:short', false), /fingerprint|encoding/);
  assert.throws(() => validatePairingTrust(vector.fingerprint + '=', false), /encoding/);
  assert.throws(() => validatePairingTrust(vector.fingerprint, true), /never both/);
  assert.equal(validatePairingTrust('', true), undefined);
  assert.equal(validatePairingTrust(` ${vector.fingerprint} `, false), vector.fingerprint);
});

test('phone verifies real Rust CBOR hashes, Ed25519 signatures and the separately supplied pin', async () => {
  await verifyPairing(session(), vector.device_key, vector.fingerprint, crypto);
});

test('a self-consistent chain accepted without a pin fails against independent trust', async () => {
  await verifyPairing(session(), vector.device_key, undefined, crypto);
  await assert.rejects(verifyPairing(session(), vector.device_key, vector.other_fingerprint, crypto), /fingerprint mismatch/);
});

test('swapped device keys, proof content, signatures and issuer links are rejected', async () => {
  await assert.rejects(verifyPairing(session(), 'p256:wrong', vector.fingerprint, crypto), /does not bind/);
  const content = session(); content.device_key_proofs[0].body.fields.label = 'changed';
  await assert.rejects(verifyPairing(content, vector.device_key, vector.fingerprint, crypto), /claim ID/);
  const signature = session(); signature.device_key_proofs[0].signature.signature = 'invalid';
  await assert.rejects(verifyPairing(signature, vector.device_key, vector.fingerprint, crypto), /signature does not verify/);
  const linked = session(); linked.device_key_proofs.reverse();
  await assert.rejects(verifyPairing(linked, vector.device_key, vector.fingerprint, crypto), /returned chain/);
});

test('read-only pin verification retains no signing grant and detects a missing older-member root proof', async () => {
  const display = { ...session(), scopes: ['read.projections'], device_key_chain: [], device_key_proofs: [], person_root_key_proof: structuredClone(vector.proofs[1]) };
  await verifyPairing(display, vector.device_key, vector.fingerprint, crypto);
  await assert.rejects(verifyPairing(display, vector.device_key, vector.other_fingerprint, crypto), /fingerprint mismatch/);
  delete display.person_root_key_proof;
  await assert.rejects(verifyPairing(display, vector.device_key, vector.fingerprint, crypto), /upgrade/);
  await verifyPairing(display, vector.device_key, undefined, crypto);
});

test('malformed public proofs and sessions do not reflect injected secret text', async () => {
  const malformed = session();
  malformed.device_key_proofs[0].signature.signed_at_unix_ms = 'secret-sentinel-never-in-errors';
  await assert.rejects(verifyPairing(malformed, vector.device_key, vector.fingerprint, crypto), error => !error.message.includes('secret-sentinel') && /Unsupported/.test(error.message));
  const bearer = session(); bearer.credential = 'short';
  await assert.rejects(verifyPairing(bearer, vector.device_key, vector.fingerprint, crypto), /Invalid paired session/);
});

test('the phone sends its Fabric NodeID only to a member that grants paired phones over Fabric', () => {
  const node = '0123456789abcdef'.repeat(4);
  const proofs = { id: 'device-key-proofs', version: 1, state: 'granted' };
  const grants = { id: 'fabric-gateway-grant', version: 1, state: 'granted' };
  assert.deepEqual(pairingFabricNode([proofs, grants], node), { fabric_node_id: node });
  // An older member refuses the unknown field, so it never sees it.
  assert.deepEqual(pairingFabricNode([proofs], node), {});
  assert.deepEqual(pairingFabricNode([proofs, { ...grants, version: 2 }], node), {});
  // No fabric in this build, or no identity yet.
  assert.deepEqual(pairingFabricNode([proofs, grants], ''), {});
  assert.deepEqual(pairingFabricNode([proofs, grants], node.toUpperCase()), {});
});

test('only a failed Fabric grant is worth telling the person after pairing', () => {
  assert.equal(fabricGrantProblem(undefined), '');
  assert.equal(fabricGrantProblem({ state: 'granted', service: 'demo-client/0', peer: 'st-device-0123456789ab' }), '');
  assert.equal(fabricGrantProblem({ state: 'not-enabled' }), '');
  assert.equal(fabricGrantProblem({ state: 'not-requested' }), '');
  assert.equal(fabricGrantProblem({ state: 'already-trusted', peer: 'demo-phone' }), '');
  assert.match(fabricGrantProblem({ state: 'failed', detail: 'fabric add failed' }), /^Paired, but .*: fabric add failed\. Tailscale or LAN still reaches it\.$/);
});
