import type { StorybookConfig } from '@storybook/react-vite'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'
import { createStylexVitePlugins } from '../../../apps/fractal-web/scripts/stylex.mjs'

const config: StorybookConfig = {
  stories: [
    '../src/**/*.stories.tsx',
    '../../../apps/fractal-web/src/web/ColdOpen.stories.tsx',
    '../../../apps/fractal-web/src/web/NarrowLayout.stories.tsx',
    // The live roster story replays captured daemon payloads through the SDK decode.
    '../../../apps/fractal-web/src/web/LiveRoster.stories.tsx',
  ],
  staticDirs: ['./fixtures'],
  framework: '@storybook/react-vite',
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  // Time the existing a11y afterEach, including its lazy axe import and reporting.
  previewAnnotations: async (entries = []) => {
    const original = fileURLToPath(import.meta.resolve('@storybook/addon-a11y/preview'))
    let replaced = false
    const annotations = entries.map(entry => {
      // Storybook 10 supplies { bare, absolute } addon entries at runtime.
      const absolute = typeof entry === 'object' && entry !== null && 'absolute' in entry ? entry.absolute : entry
      if (absolute !== original) return entry
      replaced = true
      return fileURLToPath(new URL('./a11y-timing.mjs', import.meta.url))
    })
    if (!replaced) throw new Error('Missing pinned a11y preview annotation')
    return annotations
  },
  core: { disableTelemetry: true },
  viteFinal: async (config) => ({
    ...config,
    plugins: [
      ...(config.plugins ?? []),
      tailwindcss(),
      // Keep compiled rules on the eager preview entry, not a lazy app pane's CSS asset.
      ...createStylexVitePlugins({ entries: [new URL('./preview.ts', import.meta.url).pathname] }),
    ],
  }),
}
export default config
