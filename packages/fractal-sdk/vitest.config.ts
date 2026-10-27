import { defineConfig } from 'vitest/config'

// Keep the SDK's complete deterministic suite independent of application test selection.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    exclude: ['**/node_modules/**', 'src/**/*.perf.test.ts'],
    maxWorkers: 1,
    testTimeout: 10000,
  },
})
