const base = require('./app.json').expo;

// APP_VARIANT selects the installed identity: dev (default), daily, or the isolated
// Release test client. Keep Expo's name (and the generated fractal workspace) stable.
module.exports = () => {
  const variant = process.env.APP_VARIANT ?? 'dev';
  if (!['dev', 'daily', 'test'].includes(variant)) throw new Error('APP_VARIANT must be dev, daily or test');
  const daily = variant === 'daily';
  const testClient = variant === 'test';
  // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
  const push = (process.env.FRACTAL_IOS_PUSH ?? process.env.ST_IOS_PUSH) === '1';
  // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  return {
    ...base,
    ...((push || testClient) ? { extra: { ...base.extra, ...(push ? { stIosPush: true } : {}), ...(testClient ? { testClientMode: true } : {}) } } : {}),
    plugins: [
      ...(base.plugins ?? []),
      push
        ? ['expo-notifications', { mode: 'development' }]
        : './plugins/withoutPush',
      ...(testClient ? ['./plugins/withTestClient.cjs'] : []),
    ],
    icon: daily || testClient ? './assets/icon.png' : './assets/icon-dev.png',
    ios: {
      ...base.ios,
      bundleIdentifier: testClient ? 'com.compoundingtech.fractal.test' : daily ? 'com.compoundingtech.fractal' : 'com.compoundingtech.fractal.starter',
      ...(testClient ? { entitlements: {
        ...base.ios.entitlements,
        'com.apple.security.application-groups': ['group.com.compoundingtech.fractal.test'],
        'keychain-access-groups': ['$(AppIdentifierPrefix)com.compoundingtech.fractal.test'],
      } } : {}),
      infoPlist: {
        ...base.ios.infoPlist,
        CFBundleDisplayName: testClient ? 'Fractal Test' : daily ? 'Fractal' : 'Fractal Dev',
        ...(testClient ? { FractalTestClientMode: true } : {}),
      },
    },
  };
};
