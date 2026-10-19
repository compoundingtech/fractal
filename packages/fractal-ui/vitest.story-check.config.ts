import { defineConfig } from 'vitest/config'
import { createStylexVitePlugins } from '../../apps/fractal-web/scripts/stylex.mjs'

export default defineConfig({
  plugins: createStylexVitePlugins(),
  test: {
    include: ['src/scenario.story-check.test.ts'],
    environment: 'node',
    maxWorkers: 1,
  },
})
