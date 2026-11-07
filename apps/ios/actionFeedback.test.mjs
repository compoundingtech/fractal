import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { test } from 'node:test';
import { retryTransient, notApplied } from '../../clients/typescript/fractal-client/index.ts';

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
});

test('successful terminal create preserves the terminal navigation ID and clears prior error', async () => {
  const screens = [];
  const success = makeTerminal({ terminalCreate: async () => ({ value: { affected_ids: ['runtime/one', 'terminal/one'] } }), terminalScreen: async id => { screens.push(id); } });
  assert.equal(await success.create('shell'), 'terminal/one');
  assert.deepEqual(screens, ['terminal/one']);
  assert.equal(success.error(), '');
});

test('feed live and recovery callbacks never dismiss action feedback', () => {
  const connection = source.slice(source.indexOf('      onConnection:'), source.indexOf('      onWindowError:'));
  assert.doesNotMatch(connection, /setError\(/);
  assert.match(source, /await retryTransient\(8, action, retryable\); setError\(''\)/);
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
  const pairing = source.slice(source.indexOf('  async function completePairing('), source.indexOf('  const actions ='));
  const cleanup = pairing.indexOf('await push.forgetIssuer(profileKey)'), verify = pairing.indexOf('await verifyPairing('), commit = pairing.indexOf('await SecureStore.setItemAsync(profileKey');
  assert.ok(cleanup >= 0 && verify > cleanup && commit > verify);
  assert.doesNotMatch(pairing.slice(0, commit), /setCredential\(|deleteItemAsync\(profileKey/);
});
