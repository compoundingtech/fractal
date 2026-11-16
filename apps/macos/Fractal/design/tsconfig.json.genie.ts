import { baseTsconfigCompilerOptions, domLib, reactJsx, tsconfigJson } from '../../../../repos/effect-utils/genie/external.ts'

export default tsconfigJson({
  compilerOptions: {
    ...baseTsconfigCompilerOptions,
    ...reactJsx,
    composite: true,
    tsBuildInfoFile: './dist/tsconfig.tsbuildinfo',
    lib: [...domLib],
    noEmit: true,
    rootDir: '.',
    outDir: 'dist',
    types: ['node', 'vite/client'],
  },
  // Storybook loads its configuration separately; this project checks the gallery sources.
  include: ['src/**/*'],
})
