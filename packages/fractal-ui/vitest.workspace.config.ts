import { defineConfig } from 'vitest/config'

/** The unstyled workspace primitives in `src/workspace`: model, store, palette core and React parts. */
export default defineConfig({
  test: {
    include: ['src/workspace/**/*.unit.test.ts', 'src/workspace/**/*.unit.test.tsx'],
    environment: 'node',
  },
})
