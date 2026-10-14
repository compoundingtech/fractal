import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';

const require = createRequire(import.meta.url);
const configure = require('./app.config.js');
const baseline = require('./app.json').expo;
const withoutPush = require('./plugins/withoutPush.js');
const withNotifications = require('expo-notifications/app.plugin.js');

test('only ST_IOS_PUSH=1 selects the notification plugin; default leaves iOS config unchanged', () => {
  const saved = process.env.ST_IOS_PUSH;
  try {
    for (const flag of [undefined, '0', 'true', '1']) {
      if (flag === undefined) delete process.env.ST_IOS_PUSH; else process.env.ST_IOS_PUSH = flag;
      const configured = configure({ config: structuredClone(baseline) });
      assert.deepEqual(configured.ios, baseline.ios);
      assert.deepEqual(configured.plugins.slice(0, -1), baseline.plugins);
      assert.equal(configured.extra?.stIosPush, flag === '1' ? true : undefined);
      assert.deepEqual(configured.plugins.at(-1), flag === '1' ? ['expo-notifications', { mode: 'development' }] : './plugins/withoutPush');
    }
  } finally {
    if (saved === undefined) delete process.env.ST_IOS_PUSH; else process.env.ST_IOS_PUSH = saved;
  }
});

test('default no-op suppresses Expo automatic notifications plugin without creating native mods', () => {
  const config = withoutPush(structuredClone(baseline));
  assert.deepEqual(config.ios, baseline.ios);
  assert.equal(config.mods, undefined);
  const automatic = withNotifications(config, {});
  assert.equal(automatic, config);
  assert.equal(automatic.mods, undefined);
  assert.equal(automatic.ios.entitlements?.['aps-environment'], undefined);
});

test('flagged notification plugin installs the iOS entitlement mod', () => {
  const config = withNotifications(structuredClone(baseline), { mode: 'development' });
  assert.equal(typeof config.mods?.ios?.entitlements, 'function');
});
