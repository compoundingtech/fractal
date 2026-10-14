import assert from 'node:assert/strict';
import { test } from 'node:test';
import { PushRegistrationLifecycle, pushDecision, pushRequest } from './pushRegistration.ts';

const token = 'ab'.repeat(32);
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
function harness({ enabled = false, environment = 'development', permission = true } = {}) {
  const calls = [];
  const state = { saved: enabled, permission, token, changed: undefined, stops: 0 };
  const ports = {
    bundleId: 'com.example.smalltalk', version: '1.0',
    readEnabled: async () => state.saved,
    writeEnabled: async value => { calls.push(['setting', value]); state.saved = value; },
    environment: async () => environment,
    permission: async request => { calls.push(['permission', request]); return state.permission; },
    token: async () => { calls.push(['token']); return state.token; },
    listen: changed => { state.changed = changed; return () => { state.stops++; state.changed = undefined; }; },
  };
  const remote = {
    register: async request => { calls.push(['register', request]); return { registered: true, environment: request.environment }; },
    unregister: async () => { calls.push(['unregister']); return { registered: false }; },
  };
  const lifecycle = new PushRegistrationLifecycle(ports);
  return { calls, state, ports, remote, lifecycle, connect: () => lifecycle.configure({ paired: true, granted: true, remote }) };
}

test('decision matrix gates all native acquisition and paired writes', () => {
  for (const paired of [false, true]) for (const granted of [false, true]) for (const enabled of [false, true]) for (const available of [false, true]) for (const permission of [false, true]) {
    const expected = !paired || !granted ? 'none' : !enabled || !permission ? 'unregister' : available ? 'register' : 'none';
    assert.equal(pushDecision({ paired, granted, enabled, available, permission }), expected);
  }
});

test('request uses only bounded native metadata and never reflects malformed tokens', () => {
  assert.deepEqual(pushRequest(token, 'development', 'com.example.smalltalk', '1.0'), { token, environment: 'development', bundle_id: 'com.example.smalltalk', app_version: '1.0' });
  for (const value of ['', 'a', 'abc', 'zz', 'ab'.repeat(257), { token }, 'secret-sentinel']) assert.equal(pushRequest(value, 'production', 'com.example.smalltalk', '1.0'), undefined);
  assert.ok(pushRequest('AB', 'production', 'a', 'v'));
  assert.ok(pushRequest('ab'.repeat(256), 'production', 'a'.repeat(255), 'v'.repeat(64)));
  for (const bundle of [null, '', 'a'.repeat(256), 'com.example/invalid', 'non-ascii-ä']) assert.equal(pushRequest(token, 'development', bundle, '1.0'), undefined);
  for (const version of [null, '', 'v'.repeat(65), 'newline\n', 'non-ascii-ä']) assert.equal(pushRequest(token, 'development', 'com.example.smalltalk', version), undefined);
});

test('default-off launch unregisters but never requests permission, acquires tokens or listens', async () => {
  const h = harness(); h.connect(); await h.lifecycle.start();
  assert.equal(h.lifecycle.snapshot().enabled, false);
  assert.deepEqual(h.calls, [['unregister']]);
  assert.equal(h.state.changed, undefined);
  await h.lifecycle.setEnabled(true);
  assert.equal(h.state.saved, true);
  assert.equal(h.lifecycle.snapshot().status, 'registered');
  assert.deepEqual(h.calls.slice(1).map(call => call[0]), ['setting', 'permission', 'token', 'register']);
  assert.deepEqual(h.calls.find(call => call[0] === 'permission'), ['permission', true]);
});

test('launch and token rotation register directly with APNs without a permission prompt', async () => {
  const h = harness({ enabled: true, environment: 'production' }); h.connect(); await h.lifecycle.start();
  assert.equal(h.lifecycle.snapshot().environment, 'production');
  assert.equal(h.lifecycle.snapshot().status, 'registered');
  h.calls.length = 0;
  h.state.changed('cd'.repeat(32));
  await h.lifecycle.refresh();
  assert.equal(h.calls.find(call => call[0] === 'register')[1].token, 'cd'.repeat(32));
  assert.ok(h.calls.filter(call => call[0] === 'permission').every(call => call[1] === false));
  assert.equal(JSON.stringify(h.lifecycle.snapshot()).includes(token), false);
  assert.equal(JSON.stringify(h.lifecycle.snapshot()).includes('cd'.repeat(32)), false);
});

test('unpaired or unsupported sessions never read permission, acquire tokens or mutate', async () => {
  for (const context of [{ paired: false, granted: true }, { paired: true, granted: false }]) {
    const h = harness({ enabled: true });
    h.lifecycle.configure({ ...context, remote: h.remote });
    await h.lifecycle.start(); await h.lifecycle.setEnabled(true);
    assert.deepEqual(h.calls, []); assert.equal(h.state.changed, undefined);
  }
});

test('a missing entitlement or unknown environment never requests permission or acquires a token', async () => {
  const h = harness({ environment: null }); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
  await h.lifecycle.setEnabled(true);
  assert.equal(h.lifecycle.snapshot().enabled, false); assert.deepEqual(h.calls, []);
});

test('denied permission removes prior registration; launch still does not prompt', async () => {
  const h = harness({ enabled: true, permission: false }); h.connect(); await h.lifecycle.start();
  assert.equal(h.lifecycle.snapshot().status, 'permission-denied');
  assert.deepEqual(h.calls, [['permission', false], ['unregister']]);
  await h.lifecycle.setEnabled(false);
  assert.equal(h.state.saved, false); assert.equal(h.state.changed, undefined);
});

test('an opt-out while native acquisition is pending fences the token before any register', async () => {
  const h = harness(); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
  const acquired = deferred(), reading = deferred();
  h.ports.token = () => { reading.resolve(); return acquired.promise; };
  const enabling = h.lifecycle.setEnabled(true); await reading.promise;
  const disabling = h.lifecycle.setEnabled(false); acquired.resolve(token);
  await Promise.all([enabling, disabling]);
  assert.equal(h.calls.some(call => call[0] === 'register'), false);
  assert.equal(h.calls.at(-1)[0], 'unregister'); assert.equal(h.state.saved, false);
});

test('opt-out serializes unregister after an in-flight register', async () => {
  const h = harness(); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
  const sent = deferred(), reply = deferred();
  h.remote.register = async request => { h.calls.push(['register', request]); sent.resolve(); return reply.promise; };
  const enabling = h.lifecycle.setEnabled(true); await sent.promise;
  const disabling = h.lifecycle.setEnabled(false); reply.resolve({ registered: true, environment: 'development' });
  await Promise.all([enabling, disabling]);
  assert.equal(h.calls.at(-1)[0], 'unregister'); assert.equal(h.lifecycle.snapshot().status, 'unregistered');
});

test('pairing/capability loss and teardown fence late tokens and remove native listeners', async () => {
  for (const end of ['stop', 'capability', 'pairing']) {
    const h = harness(); h.connect(); await h.lifecycle.start();
    const reading = deferred(), acquired = deferred();
    h.ports.token = () => { reading.resolve(); return acquired.promise; };
    const enabling = h.lifecycle.setEnabled(true); await reading.promise;
    const changed = h.state.changed;
    if (end === 'stop') h.lifecycle.stop();
    else h.lifecycle.configure({ paired: end !== 'pairing', granted: false, remote: h.remote });
    acquired.resolve(token); changed?.('cd'.repeat(32)); await enabling; await h.lifecycle.refresh();
    assert.equal(h.calls.some(call => call[0] === 'register'), false);
    assert.equal(h.state.changed, undefined); assert.ok(h.state.stops > 0);
  }
});

test('offline opt-out persists and unregisters at the next capable connection', async () => {
  const h = harness({ enabled: true }); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
  h.lifecycle.configure({ paired: true, granted: false }); await h.lifecycle.setEnabled(false);
  assert.equal(h.state.saved, false); assert.equal(h.calls.some(call => call[0] === 'unregister'), false);
  h.connect(); await h.lifecycle.refresh();
  assert.equal(h.calls.at(-1)[0], 'unregister'); assert.equal(h.state.changed, undefined);
});

test('forget unregisters best effort on the last capable connection and prevents registration', async () => {
  const h = harness({ enabled: true }); h.connect(); await h.lifecycle.start();
  const changed = h.state.changed;
  h.lifecycle.configure({ paired: true, granted: false }); h.calls.length = 0;
  h.remote.unregister = async () => { h.calls.push(['unregister']); throw new Error(token); };
  await h.lifecycle.forget(); changed(token); await h.lifecycle.refresh();
  assert.deepEqual(h.calls, [['setting', false], ['unregister']]);
  assert.equal(h.state.saved, false); assert.equal(h.state.changed, undefined);
  assert.equal(JSON.stringify(h.lifecycle.snapshot()).includes(token), false);
});

test('forget waits for an in-flight registration before unregistering', async () => {
  const h = harness(); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
  const sent = deferred(), reply = deferred();
  h.remote.register = async request => { h.calls.push(['register', request]); sent.resolve(); return reply.promise; };
  const enabling = h.lifecycle.setEnabled(true); await sent.promise;
  const forgetting = h.lifecycle.forget(); reply.resolve({ registered: true, environment: 'development' });
  await Promise.all([enabling, forgetting]);
  assert.equal(h.calls.at(-1)[0], 'unregister'); assert.equal(h.state.saved, false);
});

test('network/native failures expose only safe status, never token-bearing exception text', async () => {
  for (const failure of ['permission', 'token', 'register']) {
    const h = harness({ enabled: true });
    if (failure === 'register') h.remote.register = async () => { throw new Error(token); };
    else h.ports[failure] = async () => { throw new Error(token); };
    h.connect(); await h.lifecycle.start();
    assert.equal(h.lifecycle.snapshot().status, 'failed');
    assert.equal(JSON.stringify(h.lifecycle.snapshot()).includes(token), false);
  }
});

test('opt-out and forget do not wait for native permission/token callbacks that never arrive', async () => {
  for (const operation of ['off', 'forget']) for (const pending of ['permission', 'token']) {
    const h = harness(); h.connect(); await h.lifecycle.start(); h.calls.length = 0;
    const entered = deferred(), native = deferred();
    h.ports[pending] = () => { entered.resolve(); return native.promise; };
    const enabling = h.lifecycle.setEnabled(true); await entered.promise;
    if (operation === 'off') await h.lifecycle.setEnabled(false); else await h.lifecycle.forget();
    await enabling;
    assert.equal(h.calls.some(call => call[0] === 'register'), false);
    assert.equal(h.calls.at(-1)[0], 'unregister');
    assert.equal(h.state.saved, false);
    native.resolve(pending === 'permission' ? true : token);
    await h.lifecycle.refresh();
    assert.equal(h.calls.some(call => call[0] === 'register'), false);
  }
});
