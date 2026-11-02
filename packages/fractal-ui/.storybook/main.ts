import type { StorybookConfig } from '@storybook/react-vite'
// Storybook 10.6.1 exposes PreviewAnnotation only from this internal export,
// not its public entry points. The timing contracts pin the version/path/shapes.
import type { PreviewAnnotation } from 'storybook/internal/types'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'
import { createStylexVitePlugins } from '../../../apps/fractal-web/scripts/stylex.mjs'
import { storyGlobs } from './stories.ts'

const config: StorybookConfig = {
  stories: storyGlobs,
  staticDirs: ['./fixtures'],
  framework: '@storybook/react-vite',
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y'],
  // Storybook 10.6.1: time the original a11y afterEach, including lazy axe import/reporting.
  previewAnnotations: async (entries = []) => {
    const original = fileURLToPath(import.meta.resolve('@storybook/addon-a11y/preview'))
    let replaced = false
    const annotations = [...entries]
    // The exported preset type also covers structured { bare, absolute } entries.
    const presetEntries: readonly PreviewAnnotation[] = entries
    for (const [index, entry] of presetEntries.entries()) {
      const absolute = typeof entry === 'string' ? entry : entry.absolute
      if (absolute !== original) continue
      replaced = true
      annotations[index] = fileURLToPath(new URL('./a11y-timing.mjs', import.meta.url))
    }
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
