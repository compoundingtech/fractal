import { domLib, effectNativeLanguageService, fractalCompilerOptions, nodeTypes, tsconfigJson } from '../../genie/tsconfig.ts'

export default tsconfigJson({
  compilerOptions: {
    ...fractalCompilerOptions,
    ...nodeTypes,
    lib: [...domLib],
    plugins: [effectNativeLanguageService({ include: ['src/**/*.ts'], exempt: ['src/**/*.test.ts', 'src/**/*.fixture.ts'] })],
  },
  include: ['src/**/*.ts', 'vitest.config.ts', 'vitest.perf.config.ts'],
  exclude: ['node_modules'],
})
