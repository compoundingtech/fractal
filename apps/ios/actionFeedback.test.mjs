import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';
import { API_VERSION, retryTransient, notApplied } from '../../clients/typescript/fractal-client/index.ts';
import { requirePairingAdvertisement } from './pairingProof.ts';

const source = readFileSync(new URL('./store.tsx', import.meta.url), 'utf8');
const slice = (start, end, method = false) => {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from);
  const code = source.slice(from, to);
  return stripTypeScriptTypes(method ? `({${code}})` : code);
};
// Exercise the actual store callbacks without loading React Native, as in conversationResend.test.mjs.
const runSource = slice('  async function runAction(', '\n  /**');
const createSource = slice('    async createTerminal(', '    async createLaunch(', true);
const repairSource = slice('    repairDevice() {', '    /** A Debug pairing link', true);
const makeTerminal = (client, status = 'online') => {
  let error = 'previous action failed';
  const setError = value => { error = typeof value === 'function' ? value(error) : value; };
  const runAction = new Function('status', 'proof', 'setError', 'setBusy', 'retryTransient', 'notApplied', 'loadLists', 'errorText', `${runSource}; return runAction;`)(status, undefined, setError, () => {}, retryTransient, notApplied, async () => {}, error => error.message);
  const create = new Function('client', 'setError', 'runAction', 'actionId', 'fence', `return ${createSource}.createTerminal;`)(client, setError, runAction, () => 'action/test', async () => ({}));
  return { create, error: () => error };
};

test('terminal create names connection failure and missing terminal ID in the banner', async () => {
  const disconnected = makeTerminal(null);
  assert.equal(await disconnected.create('shell'), null);
  assert.match(disconnected.error(), /Could not create terminal: still connecting/);
  const missing = makeTerminal({ terminalCreate: async () => ({ value: { affected_ids: ['runtime/only'] } }) });
  assert.equal(await missing.create('shell'), null);
  assert.equal(missing.error(), 'Could not create terminal: Fractal returned no terminal ID.');
  const refused = makeTerminal({ terminalCreate: async () => { throw new Error('not allowed'); } });
  assert.equal(await refused.create('shell'), null);
  assert.equal(refused.error(), 'Could not create terminal: not allowed');
  const connecting = makeTerminal({}, 'connecting');
  assert.equal(await connecting.create('shell'), null);
  assert.match(connecting.error(), /Could not create terminal: Still connecting/);
  console.log('PASS terminal create names connection failure and missing terminal ID in the banner');
});

test('successful terminal create preserves the terminal navigation ID and clears prior error', async () => {
  const screens = [];
  const success = makeTerminal({ terminalCreate: async () => ({ value: { affected_ids: ['runtime/one', 'terminal/one'] } }), terminalScreen: async id => { screens.push(id); } });
  assert.equal(await success.create('shell'), 'terminal/one');
  assert.deepEqual(screens, ['terminal/one']);
  assert.equal(success.error(), '');
  console.log('PASS successful terminal create preserves the terminal navigation ID and clears prior error');
});

test('action error survives actual feed reconnecting and live transitions', () => {
  const callback = slice('      onConnection:', '      onWindowError:', true);
  let error = 'Could not create terminal: not allowed';
  const statuses = [];
  const dependencies = {
    current: () => true, proofRef: { current: { record: () => {} } },
    setStatus: value => statuses.push(value), setConnectionIssue: () => {},
    setError: value => { error = value; }, capsClient: { current: null }, setCaps: () => {},
    routeRef: { current: { kind: 'tailscale' } }, loadCapabilities: async () => {},
    carrier: { feedLive: () => {} }, feedMisses: { current: 0 },
  };
  const transition = new Function(...Object.keys(dependencies), `let connection = 0; return ${callback}.onConnection;`)(...Object.values(dependencies));
  transition('reconnecting');
  transition('live');
  assert.deepEqual(statuses, ['offline', 'online']);
  assert.equal(error, 'Could not create terminal: not allowed');
  console.log('PASS action error survives actual feed reconnecting and live transitions');
});

test('Release re-pair opens a blank form on the saved gateway without discarding credentials', () => {
  let draft, issue;
  const repair = new Function('credential', 'url', 'proof', 'setPairDraft', 'setPairingIssue', `return ${repairSource}.repairDevice;`)('old bearer', 'https://gateway.example', undefined, value => { draft = value; }, value => { issue = value; });
  repair();
  assert.deepEqual(draft, { gateway: 'https://gateway.example', id: '', code: '' });
  assert.match(issue, /current pairing stays in use/);
  const fleet = readFileSync(new URL('./screens/Fleet.tsx', import.meta.url), 'utf8');
  assert.match(fleet, /label="re-pair this device" onPress=\{actions.repairDevice\}/);
  assert.match(fleet, /const submit = .*actions.pair\(id, code, fingerprint, unpinned\)/);
  console.log('PASS Release re-pair opens a blank form on the saved gateway without discarding credentials');
});

const pairingHarness = (gateway = 'https://gateway.example', fabric = {}) => {
  const pairingSource = slice('  async function completePairing(', '  /** Forget this phone');
  const oldProfile = JSON.stringify({ url: gateway, credential: 'old bearer', signing: { handle: 'old-key' } });
  const profiles = new Map([['profile', oldProfile]]);
  const events = [];
  const requests = [];
  const dependencies = {
    proof: undefined, validatePairingTrust: () => 'trusted fingerprint', canVerifyPairing: () => true, requirePairingAdvertisement,
    PROFILE_KEY: 'profile', pairDraft: null,
    SecureStore: {
      getItemAsync: async key => profiles.get(key),
      setItemAsync: async (key, value) => { events.push('commit'); profiles.set(key, value); },
    },
    gatewayFetch: () => async url => { requests.push(url); return { ok: true, text: async () => JSON.stringify({ api_version: API_VERSION, capabilities: [{ id: 'device-key-proofs', version: 1, state: 'granted' }] }) }; },
    fabricBuilt: false, carrierChoice: 'tailscale', fabricSnapshot: { phase: 'off' }, fabricMember: null,
    isFabricGatewayId: value => value.startsWith('fabric://'), fabricGatewayId: target => `fabric://${target.node}/${encodeURIComponent(target.service)}`, API_VERSION,
    pairingFabricNode: () => ({}), createDeviceKey: async () => ({ key: 'new-public-key', handle: 'new-key', storage: 'secure' }),
    push: { forgetIssuer: async () => { events.push('cleanup'); return true; }, releaseIssuer: () => events.push('release') },
    FractalClient: class {
      constructor({ baseUrl }) { requests.push(baseUrl); }
      async completePairing() { events.push('complete'); return { value: { kind: 'paired-session', device_id: 'device/0123456789abcdef01234567' } }; }
    },
    clientName: () => 'test', app: { expo: { version: 'test' } },
    verifyPairing: async () => { events.push('verify'); throw new Error('proof verification failed'); },
    verifyGrantSignature: () => {}, Crypto: {},
    removeDeviceKey: async handle => events.push(`remove:${handle}`),
    errorText: error => error.message,
    ...fabric,
  };
  const complete = new Function(...Object.keys(dependencies), `${pairingSource}; return completePairing;`)(...Object.values(dependencies));
  return { complete: () => complete('pair/1', 'code', 'fingerprint', false, gateway), profiles, oldProfile, events, requests };
};

test('failed replacement verification preserves the old secure profile and key', async () => {
  const { complete, profiles, oldProfile, events } = pairingHarness();
  await assert.rejects(complete(), /proof verification failed.*previous profile was retained/i);
  assert.equal(profiles.get('profile'), oldProfile);
  assert.deepEqual(events, ['cleanup', 'complete', 'verify', 'remove:new-key', 'release']);
  console.log('PASS failed replacement verification preserves the old secure profile and key');
});

test('re-pairing own member A refuses connected member B before submitting a code', async () => {
  const memberA = { node: 'a'.repeat(64), service: 'paired', name: 'A', device: true };
  const memberB = { node: 'b'.repeat(64), service: 'paired', name: 'B' };
  const ready = member => ({ fabricBuilt: true, fabricNode: 'phone-node', carrierChoice: 'fabric', fabricSnapshot: { phase: 'ready', member: member.name, bridgeUrl: 'http://127.0.0.1:1234' }, fabricMember: member });
  for (const gateway of ['https://gateway.example', `fabric://${memberA.node}/paired`]) {
    for (const selected of [memberB, { ...memberA, service: 'other', device: undefined }]) {
      const attempt = pairingHarness(gateway, ready(selected));
      await assert.rejects(attempt.complete(), /code was not submitted; the previous profile was retained/i);
      assert.deepEqual(attempt.requests, [], 'a wrong member or service receives neither discovery nor the pairing code');
      assert.deepEqual(attempt.events, [], 'refusal happens before notification cleanup or key creation');
      assert.equal(attempt.profiles.get('profile'), attempt.oldProfile);
    }
    const matching = pairingHarness(gateway, ready(memberA));
    await assert.rejects(matching.complete(), /proof verification failed/i);
    const expected = gateway.startsWith('fabric://') ? 'http://127.0.0.1:1234' : gateway;
    assert.deepEqual(matching.requests, [`${expected}/v1/client/capabilities?accepts=fabric-gateway-grant`, expected]);
    assert.equal(matching.profiles.get('profile'), matching.oldProfile);
  }
  console.log('PASS re-pairing own member A refuses connected member B before submitting a code');
});

test('scanned pairing stays pinned to its gateway despite a different ready Fabric member', async () => {
  const gateway = 'https://scanned.example/base';
  const attempt = pairingHarness(gateway, {
    pairDraft: { gateway, id: 'pair/1', code: 'code', scanned: true },
    fabricBuilt: true, fabricNode: 'phone-node', carrierChoice: 'fabric',
    fabricSnapshot: { phase: 'ready', member: 'B', bridgeUrl: 'http://127.0.0.1:1234' },
    fabricMember: { node: 'b'.repeat(64), service: 'paired', name: 'B' },
  });
  await assert.rejects(attempt.complete(), /proof verification failed/i);
  assert.deepEqual(attempt.requests, [`${gateway}/v1/client/capabilities?accepts=fabric-gateway-grant`, gateway]);
  assert.equal(attempt.profiles.get('profile'), attempt.oldProfile);
  console.log('PASS scanned pairing stays pinned to its gateway despite a different ready Fabric member');
});
