const base = require('./app.json').expo;

// APP_VARIANT selects the installed identity. dev (default) is the Debug/Metro starter app;
// daily is the Release app used every day. Both coexist on one phone with separate Keychain profiles.
// The home-screen name is set through CFBundleDisplayName so Expo's `name`, and with it the
// generated Xcode project, workspace and scheme (`ios/fractal.xcworkspace`), stay the same for both.
module.exports = () => {
  const variant = process.env.APP_VARIANT ?? 'dev';
  if (variant !== 'dev' && variant !== 'daily') throw new Error('APP_VARIANT must be dev or daily');
  const daily = variant === 'daily';
  // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
  const push = (process.env.FRACTAL_IOS_PUSH ?? process.env.ST_IOS_PUSH) === '1';
  // LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  return {
    ...base,
    ...(push ? { extra: { ...base.extra, stIosPush: true } } : {}),
    plugins: [
      ...(base.plugins ?? []),
      push
        ? ['expo-notifications', { mode: 'development' }]
        : './plugins/withoutPush',
    ],
    icon: daily ? './assets/icon.png' : './assets/icon-dev.png',
    ios: {
      ...base.ios,
      bundleIdentifier: daily ? 'com.compoundingtech.fractal' : 'com.compoundingtech.fractal.starter',
      infoPlist: { ...base.ios.infoPlist, CFBundleDisplayName: daily ? 'Fractal' : 'Fractal Dev' },
    },
  };
};
