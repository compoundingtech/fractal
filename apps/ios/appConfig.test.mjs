import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
const configure = createRequire(import.meta.url)('./app.config.js');
const base = createRequire(import.meta.url)('./app.json').expo;

const withVariant = (value, run) => {
  const saved = process.env.APP_VARIANT;
  try {
    if (value === undefined) delete process.env.APP_VARIANT; else process.env.APP_VARIANT = value;
    return run();
  } finally {
    if (saved === undefined) delete process.env.APP_VARIANT; else process.env.APP_VARIANT = saved;
  }
};

test('daily installs as Fractal with the ordinary icon', () => {
  const daily = withVariant('daily', configure);
  assert.equal(daily.name, 'fractal');
  assert.equal(daily.slug, 'fractal-starter');
  assert.equal(daily.ios.infoPlist.CFBundleDisplayName, 'Fractal');
  assert.equal(daily.ios.infoPlist.NSMicrophoneUsageDescription.length > 0, true);
  assert.equal(daily.icon, './assets/icon.png');
  assert.equal(daily.ios.bundleIdentifier, 'com.compoundingtech.fractal');
  assert.equal(daily.ios.supportsTablet, true);
  assert.equal(daily.updates, undefined);
  assert.equal(daily.extra?.testClientMode, undefined);
  assert.equal(daily.ios.infoPlist.FractalTestClientMode, undefined);
  assert.equal(daily.plugins.includes('./plugins/withTestClient.cjs'), false);
  assert.equal(daily.ios.entitlements, undefined);
});

test('default and dev keep the starter identity with the DEV-badged icon', () => {
  const dev = withVariant(undefined, configure);
  assert.equal(dev.name, 'fractal');
  assert.equal(dev.slug, 'fractal-starter');
  assert.equal(dev.ios.infoPlist.CFBundleDisplayName, 'Fractal Dev');
  assert.equal(dev.icon, './assets/icon-dev.png');
  assert.equal(dev.ios.bundleIdentifier, 'com.compoundingtech.fractal.starter');
  assert.deepEqual(withVariant('dev', configure), dev);
  assert.equal(dev.extra?.testClientMode, undefined);
  assert.equal(dev.ios.infoPlist.FractalTestClientMode, undefined);
  assert.equal(dev.plugins.includes('./plugins/withTestClient.cjs'), false);
  assert.equal(dev.ios.entitlements, undefined);
});

test('test is a separately named, isolated Release-source client with a native observer', () => {
  const isolated = withVariant('test', configure);
  assert.equal(isolated.name, 'fractal');
  assert.equal(isolated.slug, 'fractal-starter');
  assert.equal(isolated.ios.bundleIdentifier, 'com.compoundingtech.fractal.test');
  assert.equal(isolated.ios.infoPlist.CFBundleDisplayName, 'Fractal Test');
  assert.equal(isolated.icon, './assets/icon.png');
  assert.equal(isolated.extra.testClientMode, true);
  assert.equal(isolated.ios.infoPlist.FractalTestClientMode, true);
  assert.deepEqual(isolated.ios.entitlements['com.apple.security.application-groups'], ['group.com.compoundingtech.fractal.test']);
  assert.deepEqual(isolated.ios.entitlements['keychain-access-groups'], ['$(AppIdentifierPrefix)com.compoundingtech.fractal.test']);
  assert.equal(isolated.plugins.filter(plugin => plugin === './plugins/withTestClient.cjs').length, 1);
  assert.equal(isolated.ios.supportsTablet, true);
  assert.equal(isolated.updates, undefined);
});

test('an unknown variant fails closed', () => {
  assert.throws(() => withVariant('typo', configure), /APP_VARIANT/);
});

test('normal variants preserve the complete origin/main configuration', () => {
  for (const variant of ['dev', 'daily']) {
    const daily = variant === 'daily';
    const push = (process.env.FRACTAL_IOS_PUSH ?? process.env.ST_IOS_PUSH) === '1';
    const original = {
      ...base,
      ...(push ? { extra: { ...base.extra, stIosPush: true } } : {}),
      plugins: [...(base.plugins ?? []), push ? ['expo-notifications', { mode: 'development' }] : './plugins/withoutPush'],
      icon: daily ? './assets/icon.png' : './assets/icon-dev.png',
      ios: {
        ...base.ios,
        bundleIdentifier: daily ? 'com.compoundingtech.fractal' : 'com.compoundingtech.fractal.starter',
        infoPlist: { ...base.ios.infoPlist, CFBundleDisplayName: daily ? 'Fractal' : 'Fractal Dev' },
      },
    };
    assert.deepEqual(withVariant(variant, configure), original);
  }
});
