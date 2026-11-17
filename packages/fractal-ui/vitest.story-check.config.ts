import { defineConfig } from 'vitest/config'
import stylex from '@stylexjs/unplugin/rollup'

export default defineConfig({
  // SSR checks need compilation, not the Vite adapter's browser CSS refresh timer.
  plugins: [stylex({ useCSSLayers: false })],
  test: {
    include: ['src/scenario.story-check.test.ts', 'src/assistant-ui/PlainComposer.integration.test.tsx'],
    environment: 'node',
    maxWorkers: 1,
  },
})
