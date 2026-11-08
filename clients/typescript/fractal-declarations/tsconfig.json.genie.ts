import { fractalCompilerOptions, tsconfigJson } from '../../../genie/tsconfig.ts'

export default tsconfigJson({
  compilerOptions: { ...fractalCompilerOptions, types: [] },
  files: ['index.ts', 'type-tests.ts', 'examples/daily-review.ts'],
})
