/** Dev-only HTML transform: production's early-connect script stays unchanged. */
export const suppressReplayEarlyConnect = (html) => html.replace(
  "const key = '__wfEarlyCollections'",
  "if (new URLSearchParams(location.search).has('scenario')) return\n        const key = '__wfEarlyCollections'",
)

export const fractalDevReplay = () => ({
  name: 'fractal:dev-replay',
  apply: 'serve',
  transformIndexHtml: suppressReplayEarlyConnect,
})
