module.exports = ({ config }) => ({
  ...config,
  ...(process.env.ST_IOS_PUSH === '1' ? { extra: { ...config.extra, stIosPush: true } } : {}),
  plugins: [
    ...(config.plugins ?? []),
    process.env.ST_IOS_PUSH === '1'
      ? ['expo-notifications', { mode: 'development' }]
      : './plugins/withoutPush',
  ],
});
