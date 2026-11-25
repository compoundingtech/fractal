import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { pairingCommitRouting, pairingTrustAllowed } from './pairingSecurity.ts';
import { credentialFor } from './fabricTarget.ts';
import { selectRoute } from './carrier.ts';

const memberA = { node: 'a'.repeat(64), service: 'client', name: 'A', device: true };
const memberC = { node: 'c'.repeat(64), service: 'client', name: 'C', device: true };
const independent = { node: 'd'.repeat(64), service: 'client', name: 'D' };
const previous = { carrier: 'fabric', members: [memberA, memberC, independent] };

test('scanned pairing refuses the unpinned override at the store policy boundary', () => {
  assert.equal(pairingTrustAllowed(true, true), false);
  assert.equal(pairingTrustAllowed(true, false), true);
  assert.equal(pairingTrustAllowed(false, true), true);
  assert.equal(pairingTrustAllowed(false, false), true);
});

test('QR commit selects the saved gateway and clears every global device binding', () => {
  const routing = pairingCommitRouting(previous, true);
  assert.equal(routing.carrier, 'tailscale');
  assert.deepEqual(routing.members, previous.members.map(({ device, ...member }) => member));
  assert.equal(previous.members[0].device, true, 'planning must not mutate the old profile before success');
  const committed = JSON.parse(JSON.stringify({ url: 'https://b.example/base', credential: 'B-bearer', routing }));
  const bridge = { phase: 'ready', bridgeUrl: 'http://127.0.0.1:1234', path: 'direct', reason: '', member: 'A', tried: 1 };
  const selected = selectRoute(committed.routing.carrier, true, committed.url, bridge, true);
  assert.equal(selected.baseUrl, committed.url);
  assert.notEqual(selected.route.kind, 'fabric');
  assert.equal(credentialFor(false, null, undefined, committed.credential), 'B-bearer');
  for (const member of committed.routing.members) {
    assert.notEqual(credentialFor(true, member, undefined, committed.credential), committed.credential, 'later selecting Fabric must not send B\'s bearer to an old member');
    assert.equal(credentialFor(true, member, 'member-own-bearer', committed.credential), 'member-own-bearer');
  }
});

test('QR commit also clears bindings on a saved-gateway route; manual pairing is unchanged', () => {
  assert.equal(pairingCommitRouting(previous, false), previous);
  const gatewayRoute = { ...previous, carrier: 'tailscale' };
  const committed = pairingCommitRouting(gatewayRoute, true);
  assert.equal(committed.carrier, 'tailscale');
  assert.equal(committed.members.some(member => member.device), false);
});

test('cancelling a re-pair draft retains the saved credential and signing profile', async () => {
  const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
  const a = source.indexOf('    async pairFromLink(');
  const b = source.indexOf('    /** Choose the carrier.', a);
  const readerStart = source.indexOf('async function readPhoneSigning(');
  const readerEnd = source.indexOf('\n/**', readerStart);
  const { outputText } = ts.transpileModule(`${source.slice(readerStart, readerEnd)}
exports.actions = { ${source.slice(a, b)} };
exports.readSigning = readPhoneSigning;`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } });
  const signing = { key: 'phone-public-key', person: 'person/avery', handle: 'retained-key-handle', chain: ['device-grant', 'root-grant'] };
  const original = JSON.stringify({ url: 'http://demo.local', credential: 'retained-bearer', signing });
  const profiles = new Map([['profile', original]]);
  let draft, issue;
  const ports = {
    PROFILE_KEY: 'profile', SIGNING_KEY: 'legacy-signing', credential: 'retained-bearer', REPAIR_WARNING: 'repair warning',
    normalizeGatewayUrl: value => value, setPairDraft: value => { draft = value; }, setPairingIssue: value => { issue = value; },
    SecureStore: {
      getItemAsync: async key => profiles.get(key),
      setItemAsync() { assert.fail('a draft must not overwrite the profile'); },
      deleteItemAsync() { assert.fail('a draft must not delete signing material'); },
    },
    removeDeviceKey() { assert.fail('cancelling a draft must not remove the existing key'); },
  };
  const exports = {};
  new Function('exports', ...Object.keys(ports), outputText)(exports, ...Object.values(ports));
  await exports.actions.pairFromLink('http://demo.local', 'pair/new', 'new-code');
  assert.ok(draft);
  exports.actions.cancelPairDraft();
  assert.equal(draft, null);
  assert.equal(issue, '');
  assert.equal(profiles.get('profile'), original);
  assert.deepEqual(await exports.readSigning('profile'), signing);
});
