import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['clients/typescript/fractal-client/*.unit.test.ts'],
    maxWorkers: 1,
  },
});
