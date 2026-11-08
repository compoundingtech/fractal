import { domLib, fractalCompilerOptions, nodeTypes, reactJsx, tsconfigJson } from '../../../genie/tsconfig.ts'

export default tsconfigJson({
  compilerOptions: { ...fractalCompilerOptions, ...reactJsx, ...nodeTypes, lib: [...domLib] },
  include: ['vitest.config.ts', 'src/**/*.ts', 'src/**/*.tsx', 'scripts/**/*.ts', 'test/**/*.ts', 'test/**/*.tsx'],
})
