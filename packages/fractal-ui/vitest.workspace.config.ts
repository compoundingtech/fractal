import { defineConfig } from 'vitest/config'

/** The unstyled workspace primitives in `src/workspace` and the pure thing-kit rules in `src/assistant-ui/things`. */
export default defineConfig({
  test: {
    include: ['src/workspace/**/*.unit.test.ts', 'src/workspace/**/*.unit.test.tsx', 'src/assistant-ui/things/**/*.unit.test.ts'],
    environment: 'node',
  },
})
