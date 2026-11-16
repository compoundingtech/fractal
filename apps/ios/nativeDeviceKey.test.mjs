import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { test } from 'node:test';

// Expo rejects excess native arguments. Model that bridge rule without loading React Native.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === 'expo') return {
      url: 'data:text/javascript,export function requireOptionalNativeModule() { return globalThis.testDeviceKeyNative; }',
      shortCircuit: true,
    };
    return nextResolve(specifier, context);
  },
});
let imports = 0;
async function adapter(native) {
  globalThis.testDeviceKeyNative = native;
  return import(`./modules/fractal-device-key/index.ts?native=${++imports}`);
}

test('existing pairings retain the old native argument counts', async () => {
  const key = { key: 'p256:legacy', storage: 'software' };
  const calls = [];
  const module = await adapter({
    async current(...args) { assert.equal(args.length, 0); calls.push('current'); return key; },
    async sign(...args) { assert.deepEqual(args, ['message']); calls.push('sign'); return 'signature'; },
    async remove(...args) { assert.equal(args.length, 0); calls.push('remove'); },
  });
  assert.equal(module.canVerifyPairing(), false);
  assert.deepEqual(await module.currentDeviceKey(), key);
  assert.equal(await module.signWithDeviceKey('message'), 'signature');
  await module.removeDeviceKey();
  assert.deepEqual(calls, ['current', 'sign', 'remove']);
});

test('new pairings select their committed key handle for native operations', async () => {
  const key = { key: 'p256:new', storage: 'secure-enclave', handle: 'candidate-key' };
  const calls = [];
  const module = await adapter({
    async create() { return key; },
    async current(...args) { calls.push(['current', ...args]); return key; },
    async sign(...args) { calls.push(['sign', ...args]); return 'signature'; },
    async remove(...args) { calls.push(['remove', ...args]); },
    async verify(...args) { calls.push(['verify', ...args]); return true; },
  });
  assert.equal(module.canVerifyPairing(), true);
  assert.deepEqual(await module.createDeviceKey(), key);
  await module.currentDeviceKey(key.handle);
  await module.signWithDeviceKey('message', key.handle);
  await module.removeDeviceKey(key.handle);
  assert.equal(await module.verifyGrantSignature('root', 'receipt', 'signature'), true);
  assert.deepEqual(calls, [
    ['current', key.handle], ['sign', 'message', key.handle], ['remove', key.handle],
    ['verify', 'root', 'receipt', 'signature'],
  ]);
});

test('renewal uses dedicated enclave creation and native approval without changing ordinary signing', async () => {
  const key = { key: 'p256:renewed', storage: 'secure-enclave', handle: 'renewal-key' };
  const calls = [];
  const module = await adapter({
    async createRenewal() { calls.push('createRenewal'); return key; },
    async approveRenewal(...args) { calls.push(['approveRenewal', ...args]); return 'approval-signature'; },
    async sign(...args) { calls.push(['sign', ...args]); return 'message-signature'; },
  });
  assert.deepEqual(await module.createRenewalKey(), key);
  assert.equal(await module.approveWithDeviceKey('bound-tuple', key.handle), 'approval-signature');
  assert.equal(await module.signWithDeviceKey('ordinary-message', key.handle), 'message-signature');
  assert.deepEqual(calls, ['createRenewal', ['approveRenewal', 'bound-tuple', key.handle], ['sign', 'ordinary-message', key.handle]]);
});

test('older native builds refuse renewal rather than falling back to ordinary key creation or signing', async () => {
  const module = await adapter({
    async create() { assert.fail('ordinary creation must not approve renewal'); },
    async sign() { assert.fail('ordinary signing must not approve renewal'); },
  });
  await assert.rejects(module.createRenewalKey(), /cannot approve renewal/);
  await assert.rejects(module.approveWithDeviceKey('tuple', 'handle'), /cannot approve renewal/);
});
