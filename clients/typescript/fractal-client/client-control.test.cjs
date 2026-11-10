const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { webcrypto, createPublicKey, createHash, verify } = require('node:crypto');
const Module = require('node:module');
const filename = path.join(__dirname, 'ClientControl.generated.ts');
const source = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 }
}).outputText;
const loaded = new Module(filename, module);
loaded.filename = filename;
loaded.paths = module.paths;
loaded._compile(source, filename);
const { visibleStateCanonicalJson, visibleStateDigest, clientResultCanonicalBytes } = loaded.exports;
const fixture = JSON.parse(fs.readFileSync(path.join(__dirname, '../../../fixtures/clients/client-control-v1.golden.json'), 'utf8'));
if (!globalThis.crypto) globalThis.crypto = webcrypto;
const point = Buffer.from(fixture.public_key_sec1_base64url, 'base64url');
const key = createPublicKey({ key: { kty: 'EC', crv: 'P-256', x: point.subarray(1, 33).toString('base64url'), y: point.subarray(33).toString('base64url') }, format: 'jwk' });
function verifies(bytes, signature) {
  return verify('sha256', bytes, { key, dsaEncoding: 'ieee-p1363' }, signature);
}
test('all clients share canonical bytes, digest and purpose-specific fixed-key signatures', async () => {
  const message = Buffer.from(fixture.message_domain_bytes);
  const messageSignature = Buffer.from(fixture.message_domain_signature_p1363_base64url, 'base64url');
  assert.ok(verifies(message, messageSignature));
  for (const vector of fixture.vectors) {
    assert.equal(visibleStateCanonicalJson(vector.input.visible_state), vector.canonical_visible_state);
    assert.equal(await visibleStateDigest(vector.input.visible_state), vector.visible_state_sha256);
    const bytes = await clientResultCanonicalBytes(vector.input);
    assert.equal(Buffer.from(bytes).toString('utf8'), vector.canonical_bytes);
    const signature = Buffer.from(vector.signature_p1363_base64url, 'base64url');
    assert.ok(verifies(bytes, signature));
    for (const changed of vector.negative_bindings) assert.equal(verifies(await clientResultCanonicalBytes(changed), signature), false);
    assert.equal(verifies(message, signature), false);
    assert.equal(verifies(bytes, messageSignature), false);
  }
});
test('JCS refuses lone surrogate strings', () => {
  const state = { ...fixture.vectors[0].input.visible_state, route: '\ud800' };
  assert.throws(() => visibleStateCanonicalJson(state), /Invalid Unicode/);
});

test('injected SHA256 reproduces golden bytes without a WebCrypto dependency', async () => {
  let calls = 0;
  const sha256 = async bytes => { calls++; return new Uint8Array(createHash('sha256').update(bytes).digest()); };
  for (const vector of fixture.vectors) {
    assert.equal(await visibleStateDigest(vector.input.visible_state, sha256), vector.visible_state_sha256);
    assert.equal(Buffer.from(await clientResultCanonicalBytes(vector.input, sha256)).toString('utf8'), vector.canonical_bytes);
  }
  assert.equal(calls, fixture.vectors.length * 2);
});
