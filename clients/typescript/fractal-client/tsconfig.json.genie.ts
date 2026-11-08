import { domLib, fractalCompilerOptions, tsconfigJson } from '../../../genie/tsconfig.ts'

export default tsconfigJson({
  compilerOptions: { ...fractalCompilerOptions, lib: [...domLib], types: [] },
  files: ['index.ts', 'types.test.ts'],
})
