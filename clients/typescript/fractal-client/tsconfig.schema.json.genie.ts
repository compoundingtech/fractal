import { domLib, effectLanguageService, fractalCompilerOptions, tsconfigJson } from '../../../genie/tsconfig.ts'

export default tsconfigJson({
  compilerOptions: {
    ...fractalCompilerOptions,
    lib: [...domLib],
    types: [],
    plugins: [{
      ...effectLanguageService,
      // The generated schema spells numbers and tagged structs exactly as the JSON Schema states them.
      diagnosticSeverity: { ...effectLanguageService.diagnosticSeverity, schemaNumber: 'off', schemaStructWithTag: 'off' },
    }],
  },
  files: ['Schema.generated.ts', 'notice.types.test.ts'],
})
