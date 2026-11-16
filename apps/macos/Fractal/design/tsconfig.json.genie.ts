import { baseTsconfigCompilerOptions, domLib, reactJsx, tsconfigJson } from '../../../genie/repo.ts'

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
  // `.storybook/**` is excluded from tsc as in otto-tray/design: `main.ts` imports the effect-utils
  // source of `@overeng/utils-storybook/config`; the Storybook build validates it.
  include: ['src/**/*'],
})
