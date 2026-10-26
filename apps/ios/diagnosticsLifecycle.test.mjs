import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { registerHooks } from 'node:module';
import { setImmediate } from 'node:timers/promises';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/Client.generated.ts';

// Substitute only the native/platform edges; execute the real lifecycle module,
// durable queue, redaction and ClientError classification.
const storage = new Map();
const listeners = new Set();
globalThis.testDiagnosticsPlatform = {
  storage: { getItem: async key => storage.get(key) ?? null, setItem: async (key, value) => { storage.set(key, value); } },
  randomUUID,
  AppState: { currentState: 'active', addEventListener: (_event, listener) => { listeners.add(listener); } },
};
globalThis.__DEV__ = false;
globalThis.HermesInternal = { hasPromise: () => true, enablePromiseRejectionTracker: () => {} };
registerHooks({
  resolve(specifier, context, nextResolve) {
    const source = {
      '@react-native-async-storage/async-storage': 'export default globalThis.testDiagnosticsPlatform.storage;',
      'expo-crypto': 'export const randomUUID = globalThis.testDiagnosticsPlatform.randomUUID;',
      expo: 'export const requireOptionalNativeModule = () => undefined;',
      'react-native': 'export const AppState = globalThis.testDiagnosticsPlatform.AppState; export const Linking = {}; export const Platform = { OS: "ios", Version: "27.0" };',
      './modules/st-diagnostics': 'export default undefined;',
      './app.json': 'export default { expo: { version: "1.0.0" } };',
    }[specifier];
    if (source) return { url: `data:text/javascript,${encodeURIComponent(source)}`, shortCircuit: true };
    return nextResolve(specifier, context);
  },
});

const foreground = state => { for (const listener of listeners) listener(state); };

test('discovery authentication refusals pause through time, foreground and capture until re-pairing', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 1_800_000_000_000 });
  const diagnostics = await import('./diagnostics.ts');
  const settle = async () => { await setImmediate(); };
  const runScheduled = async () => { await settle(); t.mock.timers.tick(0); await settle(); };
  for (const status of [401, 403]) {
    let discoveries = 0;
    let submissions = 0;
    const owner = {};
    const refused = {
      async discover() { discoveries++; throw new ClientError({ message: 'refused', code: 'unauthorized', retryable: false }, status); },
      async diagnosticsSubmit() { submissions++; assert.fail('discovery refusal must prevent submission'); },
    };
    diagnostics.connectDiagnostics(owner, refused, 'https://gateway.invalid', 'paired', true);
    await runScheduled();
    assert.equal(discoveries, 1);
    t.mock.timers.tick(120_000);
    await settle();
    foreground('background');
    foreground('active');
    diagnostics.captureJsError(new TypeError('captured while refused'));
    diagnostics.connectDiagnostics(owner, refused, 'https://gateway.invalid', 'paired', true);
    await runScheduled();
    t.mock.timers.tick(120_000);
    await settle();
    assert.equal(discoveries, 1, `${status} must stay paused across all lifecycle triggers`);
    assert.equal(submissions, 0);
    assert.ok(JSON.parse(storage.get('st3.client-diagnostics.v1')).events.length > 0);

    let replacementDiscoveries = 0;
    let replacementSubmissions = 0;
    const replacement = {
      async discover() { replacementDiscoveries++; return { value: { capabilities: [{ id: 'write.client-diagnostics', state: 'granted', version: 1 }] } }; },
      async diagnosticsSubmit(batch) {
        replacementSubmissions++;
        return { value: { acknowledged_event_ids: batch.events.map(event => event.event_id) } };
      },
    };
    diagnostics.connectDiagnostics(owner, replacement, 'https://gateway.invalid', 're-paired', true);
    await runScheduled();
    assert.equal(replacementDiscoveries, 1);
    assert.equal(replacementSubmissions, 1);
    assert.equal(JSON.parse(storage.get('st3.client-diagnostics.v1')).events.length, 0);
    diagnostics.disconnectDiagnostics(owner);
  }
});
